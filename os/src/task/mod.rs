//! 任务管理与调度。
//!
//! 第 5 周起是**抢占式调度**。任务让出 CPU 有三条路径：
//!
//! 1. 主动调 `sys_yield`（协作，第 4 周就有了）
//! 2. 调 `sys_exit` 退出
//! 3. **时间片用完，被时钟中断强行切走**（第 5 周新增）
//!
//! 第 3 条是关键：调度不再需要任务配合。一个 `loop {}` 的任务
//! 再也卡不死系统了。
//!
//! # 调度算法
//!
//! 最朴素的轮转（Round Robin）：从当前任务的下一个开始，
//! 环形扫描找第一个 `Ready` 的任务。
//!
//! 注意这三条路径最终都汇到同一个 [`schedule_next`]。
//! 无论是"我主动让"还是"被硬件夺走"，切换动作本身完全一样 ——
//! 差别只在于是谁触发的。

mod context;
mod switch;
mod tcb;

use crate::config::MAX_APP_NUM;
use crate::loader::{get_app_data, num_app};
use crate::sbi::shutdown;
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::vec::Vec;
use context::TaskContext;
use switch::__switch;
use tcb::{TaskControlBlock, TaskStatus};

struct TaskManagerInner {
    /// 第 8 周起 `TaskControlBlock` 不再是 `Copy`（它拥有一整个地址空间），
    /// 所以定长数组换成了 `Vec`。
    tasks: Vec<TaskControlBlock>,
    current_task: usize,
}

impl TaskManagerInner {
    const fn empty() -> Self {
        Self {
            tasks: Vec::new(),
            current_task: 0,
        }
    }
}

static TASK_MANAGER: UPSafeCell<TaskManagerInner> =
    // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
    unsafe { UPSafeCell::new(TaskManagerInner::empty()) };

/// 初始化任务管理器：为每个程序建立地址空间和内核栈。
pub fn init() {
    let n = num_app();
    // 内核栈从跳板页往下依次排布，每个占 KERNEL_STACK_SIZE + 一页 guard page。
    // 任务太多会一路排到用户地址空间里去 —— 在这里拦住。
    assert!(n <= MAX_APP_NUM, "用户程序数量 {n} 超过上限 {MAX_APP_NUM}");
    let mut inner = TASK_MANAGER.exclusive_access();
    for app_id in 0..n {
        inner.tasks.push(TaskControlBlock::new(get_app_data(app_id), app_id));
    }
    info!("任务管理器就绪：{} 个任务，各自拥有独立地址空间", n);
}

/// 当前任务地址空间的 `satp`。`trap_return` 用它切回用户页表。
pub fn current_user_token() -> usize {
    let inner = TASK_MANAGER.exclusive_access();
    inner.tasks[inner.current_task].user_token()
}

/// 当前任务的 [`TrapContext`]。
///
/// 它住在用户地址空间里，内核通过物理页号 + 恒等映射访问它。
pub fn current_trap_cx() -> &'static mut TrapContext {
    let inner = TASK_MANAGER.exclusive_access();
    inner.tasks[inner.current_task].trap_cx()
}

/// 当前正在运行的任务编号。系统调用里用它做权限检查。
pub fn current_task_id() -> usize {
    TASK_MANAGER.exclusive_access().current_task
}

/// 从当前任务的下一个开始环形扫描，找第一个就绪的任务。
fn find_next_ready(inner: &TaskManagerInner) -> Option<usize> {
    let n = inner.tasks.len();
    let current = inner.current_task;
    (current + 1..current + 1 + n)
        .map(|id| id % n)
        .find(|&id| inner.tasks[id].task_status == TaskStatus::Ready)
}

/// 切到下一个就绪任务。没有就绪任务时关机。
///
/// 注意这个函数**会返回** —— 但返回的时机是"当前任务将来再次被调度时"。
/// 中间可能已经跑过了别的任务。
fn schedule_next() {
    let (current_cx, next_cx) = {
        let mut inner = TASK_MANAGER.exclusive_access();

        let Some(next) = find_next_ready(&inner) else {
            info!("没有就绪任务了，全部执行完毕，关机");
            drop(inner); // shutdown 不返回，必须手动释放借用
            shutdown(false);
        };

        let current = inner.current_task;
        inner.tasks[next].task_status = TaskStatus::Running;
        inner.current_task = next;

        // 用 &raw 而不是 &mut / &：同时借用 tasks 的两个元素会过不了借用检查，
        // 而裸指针的创建不会产生持续的借用。
        let current_cx = &raw mut inner.tasks[current].task_cx;
        let next_cx = &raw const inner.tasks[next].task_cx;
        (current_cx, next_cx)
        // inner 在这里析构 —— 必须的。__switch 之后我们就不在这个
        // 执行流里了，Drop 不会在"离开"的那一刻运行。
    };

    // SAFETY: 两个指针都指向 TASK_MANAGER 里静态存活的 TaskContext。
    unsafe {
        __switch(current_cx, next_cx);
    }
    // 能走到这里，说明当前任务又被调度回来了。
}

/// 挂起当前任务，切到下一个。`sys_yield` 的落地点。
pub fn suspend_current_and_run_next() {
    {
        let mut inner = TASK_MANAGER.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].task_status = TaskStatus::Ready;
    }
    schedule_next();
}

/// 结束当前任务，切到下一个。永不返回。
pub fn exit_current_and_run_next() -> ! {
    {
        let mut inner = TASK_MANAGER.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].task_status = TaskStatus::Exited;
    }
    schedule_next();
    // 当前任务已经是 Exited，调度器永远不会再选中它，
    // 所以 schedule_next 里的 __switch 不会返回到这里。
    unreachable!("已退出的任务不应该被重新调度")
}

/// 运行第一个任务。永不返回。
pub fn run_first_task() -> ! {
    let next_cx = {
        let mut inner = TASK_MANAGER.exclusive_access();
        assert!(!inner.tasks.is_empty(), "没有任何用户程序可以运行");
        inner.tasks[0].task_status = TaskStatus::Running;
        inner.current_task = 0;
        &raw const inner.tasks[0].task_cx
    };

    // 内核的启动流程本身不是一个"任务"，没有 TaskContext 可存。
    // 但 __switch 一定会往第一个参数里写东西，所以给它一块废弃的空间。
    // 这块内容永远不会被读回来 —— 启动流程到此为止，不会再被调度。
    let mut discarded = TaskContext::zero_init();

    // SAFETY: discarded 在栈上有效；next_cx 指向静态存活的 TaskContext。
    unsafe {
        __switch(&raw mut discarded, next_cx);
    }
    unreachable!("__switch 到第一个任务后不应该返回")
}

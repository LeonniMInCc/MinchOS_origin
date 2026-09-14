//! 当前进程与调度循环。
//!
//! # 调度模型变了
//!
//! 第 4–8 周是**任务直接切到任务**：A 让出 CPU 时，自己挑好下一个 B，
//! 然后 `__switch(A, B)`。
//!
//! 第 9 周改成**任务 ↔ idle 的双向切换**：
//!
//! ```text
//!   任务 A ──__switch──▶ idle 控制流 ──__switch──▶ 任务 B
//! ```
//!
//! 多了一次切换，为什么反而更好？
//!
//! - **让出的一方不必知道下一个是谁。** 进程退出时尤其明显：
//!   它已经是僵尸了，让它自己去挑下一个任务很别扭
//! - **就绪队列为空有了自然的处理位置。** idle 控制流发现队列空了，
//!   就在那里停下来，不需要在每个让出点都判断一次
//! - idle 控制流跑在**启动栈**上，不属于任何进程，
//!   所以进程的内核栈可以被安全地回收
//!
//! 这是操作系统里一个反复出现的模式：**多引入一层间接，换来职责的分离。**

use super::context::TaskContext;
use super::manager::fetch_task;
use super::switch::__switch;
use super::tcb::{TaskControlBlock, TaskStatus};
use crate::sbi::shutdown;
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::sync::Arc;

struct Processor {
    /// 正在 CPU 上运行的进程
    current: Option<Arc<TaskControlBlock>>,
    /// idle 控制流自己的上下文。进程让出 CPU 时切回这里。
    idle_task_cx: TaskContext,
}

impl Processor {
    const fn new() -> Self {
        Self {
            current: None,
            idle_task_cx: TaskContext::zero_init(),
        }
    }
}

static PROCESSOR: UPSafeCell<Processor> =
    // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
    unsafe { UPSafeCell::new(Processor::new()) };

/// 取走当前进程（把 `current` 置空）。进程让出或退出时用。
pub fn take_current_task() -> Option<Arc<TaskControlBlock>> {
    PROCESSOR.exclusive_access().current.take()
}

/// 当前进程。
pub fn current_task() -> Option<Arc<TaskControlBlock>> {
    PROCESSOR.exclusive_access().current.clone()
}

/// 当前进程的 PID。
pub fn current_pid() -> usize {
    current_task().map(|t| t.pid()).unwrap_or(usize::MAX)
}

/// 当前进程地址空间的 `satp`。`trap_return` 用它切回用户页表。
pub fn current_user_token() -> usize {
    current_task()
        .expect("没有当前进程")
        .inner_exclusive_access()
        .user_token()
}

/// 当前进程的 [`TrapContext`]。
pub fn current_trap_cx() -> &'static mut TrapContext {
    current_task()
        .expect("没有当前进程")
        .inner_exclusive_access()
        .trap_cx()
}

/// idle 控制流：不断从就绪队列取进程来跑。永不返回。
///
/// 它跑在内核的**启动栈**上，不属于任何进程。
pub fn run_tasks() -> ! {
    loop {
        let Some(task) = fetch_task() else {
            // 就绪队列空了。本项目还没有阻塞（没有 I/O 等待、没有睡眠），
            // 所以"队列空"就等于"活干完了"。
            //
            // 有了阻塞之后这里不能直接关机 —— 那时要区分"没人可跑"
            // 和"大家都在等 I/O"，后者应该 wfi 等中断。
            info!("就绪队列已空，全部进程执行完毕");
            // initproc 没有父进程，没人 waitpid 它 —— 内核这边主动撒手，
            // 让它的页表和内核栈也走完 RAII 回收，核对才有意义。
            super::release_initproc();
            crate::mm::report_leak();
            shutdown(false);
        };

        let (idle_cx, next_cx) = {
            let mut processor = PROCESSOR.exclusive_access();
            let idle_cx = &raw mut processor.idle_task_cx;

            let mut inner = task.inner_exclusive_access();
            inner.task_status = TaskStatus::Running;
            let next_cx = &raw const inner.task_cx;
            drop(inner);

            processor.current = Some(task);
            (idle_cx, next_cx)
            // processor 在这里析构 —— 必须的，__switch 之后我们就离开
            // 这个执行流了，Drop 不会在"离开"的那一刻运行。
        };

        // SAFETY: 两个指针都指向静态存活（PROCESSOR / Arc 里）的 TaskContext。
        unsafe {
            __switch(idle_cx, next_cx);
        }
        // 回到这里 = 某个进程让出或退出了，继续下一轮
    }
}

/// 从当前进程切回 idle 控制流。
///
/// `switched_task_cx` 指向**要保存到哪里** —— 通常是让出者自己的
/// `task_cx`；进程退出时则是一个丢弃用的临时变量。
pub fn schedule(switched_task_cx: *mut TaskContext) {
    let idle_cx = {
        let processor = PROCESSOR.exclusive_access();
        &raw const processor.idle_task_cx
    };
    // SAFETY: 两个指针都指向有效的 TaskContext。
    unsafe {
        __switch(switched_task_cx, idle_cx);
    }
}

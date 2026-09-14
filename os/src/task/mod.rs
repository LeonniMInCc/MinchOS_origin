//! 进程管理与调度。
//!
//! 第 9 周把"一张定长任务表"换成了**一棵进程树**：
//!
//! ```text
//!   initproc (pid 0)
//!     ├── 子进程 A (pid 1)
//!     ├── 子进程 B (pid 2)
//!     │     └── 孙进程 (pid 4)
//!     └── 子进程 C (pid 3)
//! ```
//!
//! 进程通过 `fork` 产生、`exec` 换装、`exit` 变成僵尸、
//! 被父进程 `waitpid` 回收。
//!
//! # 一个进程是怎么消失的
//!
//! 这条路径值得看清楚，它是本章最容易出错的地方：
//!
//! 1. 进程调 `exit`，状态变成 `Zombie`，退出码存进 PCB
//! 2. **立刻归还用户内存** —— 死掉的进程没理由继续占着几 MiB 物理内存
//! 3. 它的子进程（如果有）过继给 `initproc`，否则会变成没人回收的孤儿
//! 4. PCB 本身还活着，因为父进程的 `children` 里握着一个 `Arc`
//! 5. 父进程 `waitpid` 把它从 `children` 里移除 → 引用计数归零 →
//!    `PidHandle` 和 `KernelStack` 的 `Drop` 触发，PID 和内核栈归还
//!
//! 第 5 步全靠 RAII 自动完成。手写这段回收逻辑的话，四五条退出路径
//! 里漏掉任何一条都会造成泄漏。

mod context;
mod manager;
mod pid;
mod processor;
mod switch;
mod tcb;

use crate::loader::get_app_data_by_name;
use crate::sync::UPSafeCell;
use alloc::sync::Arc;
use context::TaskContext;
use manager::add_task;
use tcb::{TaskControlBlock, TaskStatus};

pub use processor::{
    current_pid, current_task, current_trap_cx, current_user_token, run_tasks, schedule,
    take_current_task,
};

/// 所有孤儿进程的收养人。
///
/// Unix 里它是 pid 1（`init`）。这里是 pid 0，因为我们的 PID 从 0 开始分配。
static INITPROC: UPSafeCell<Option<Arc<TaskControlBlock>>> =
    // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
    unsafe { UPSafeCell::new(None) };

fn initproc() -> Arc<TaskControlBlock> {
    INITPROC
        .exclusive_access()
        .as_ref()
        .expect("initproc 还没创建")
        .clone()
}

/// 创建 `initproc` 并放进就绪队列。整个用户态世界的起点。
pub fn add_initproc() {
    let data = get_app_data_by_name("initproc").expect("内核镜像里找不到 initproc");
    let task = Arc::new(TaskControlBlock::new(data));
    info!("initproc 已创建，pid = {}", task.pid());
    *INITPROC.exclusive_access() = Some(Arc::clone(&task));
    add_task(task);
}

/// 放掉内核对 `initproc` 的最后一个引用。只在关机前调用。
///
/// `initproc` 退出后变成僵尸，但它没有父进程，没人会 `waitpid` 它 ——
/// 引用只剩 [`INITPROC`] 这一个。不放掉的话，它的页表和内核栈会一直占着，
/// 关机前的页帧核对就会报出一个"假泄漏"。
///
/// 放掉之后，整条 RAII 链会走完：`MemorySet` 归还页表页帧、
/// `KernelStack` 解除内核栈映射、`PidHandle` 归还 PID。
pub fn release_initproc() {
    *INITPROC.exclusive_access() = None;
}

/// 挂起当前进程，切回 idle 控制流。`sys_yield` 和时钟中断的落地点。
///
/// 这个函数**会返回** —— 但返回的时机是"当前进程再次被调度上来"。
pub fn suspend_current_and_run_next() {
    let task = take_current_task().expect("没有当前进程");

    let task_cx_ptr = {
        let mut inner = task.inner_exclusive_access();
        inner.task_status = TaskStatus::Ready;
        &raw mut inner.task_cx
    };

    // 放回队尾再切走。顺序不能反：切走之后这段代码就不执行了。
    add_task(task);
    schedule(task_cx_ptr);
}

/// 结束当前进程。永不返回。
pub fn exit_current_and_run_next(exit_code: i32) -> ! {
    let task = take_current_task().expect("没有当前进程");
    let pid = task.pid();

    let is_initproc = Arc::ptr_eq(&task, &initproc());
    if is_initproc {
        info!("initproc 退出，退出码 {}", exit_code);
    }

    {
        let mut inner = task.inner_exclusive_access();
        inner.task_status = TaskStatus::Zombie;
        inner.exit_code = exit_code;

        // 把子进程过继给 initproc。
        //
        // 不这么做的话，这些子进程退出后就成了没人 wait 的僵尸，
        // PCB、PID、内核栈全都永远回收不掉。Unix 也是这么处理孤儿的。
        if !is_initproc && !inner.children.is_empty() {
            let n = inner.children.len();
            let foster = initproc();
            let mut foster_inner = foster.inner_exclusive_access();
            for child in inner.children.iter() {
                child.inner_exclusive_access().parent = Some(Arc::downgrade(&foster));
                foster_inner.children.push(Arc::clone(child));
            }
            debug!("进程 {} 的 {} 个子进程过继给 initproc", pid, n);
        }
        inner.children.clear();

        // 用户内存立刻还回去，只留一个带退出码的空壳等父进程来收。
        // 页表本身还留着 —— 它很小，而且 TrapContext 页还挂在上面。
        inner.memory_set.recycle_data_pages();
    }

    // 这里只是放掉"当前进程"这一个引用。父进程的 children 里还握着一份，
    // 所以 PCB 不会现在就消失 —— 它要等 waitpid。
    drop(task);

    // 进程已经死了，它的 TaskContext 不需要保存。给 __switch 一个
    // 丢弃用的目标即可。
    let mut discarded = TaskContext::zero_init();
    schedule(&raw mut discarded);
    unreachable!("僵尸进程不应该被重新调度")
}

/// `waitpid` 的内核实现。
///
/// - `pid < 0`：等任意一个子进程
/// - `Ok(Some((pid, exit_code)))`：回收成功
/// - `Ok(None)`：有这个子进程，但还没退出
/// - `Err(())`：根本没有这个子进程
pub fn wait_child(pid: isize) -> Result<Option<(usize, i32)>, ()> {
    let task = current_task().expect("没有当前进程");
    let mut inner = task.inner_exclusive_access();

    let matches = |child: &Arc<TaskControlBlock>| pid < 0 || child.pid() == pid as usize;

    if !inner.children.iter().any(matches) {
        return Err(());
    }

    let found = inner
        .children
        .iter()
        .position(|child| matches(child) && child.inner_exclusive_access().is_zombie());

    let Some(idx) = found else {
        return Ok(None); // 子进程还活着，让调用方去等
    };

    // 从 children 里移走。这是最后一个强引用 ——
    // 离开这个作用域时，PidHandle 和 KernelStack 的 Drop 会被触发，
    // PID 和内核栈自动归还。
    let child = inner.children.remove(idx);
    assert_eq!(
        Arc::strong_count(&child),
        1,
        "回收进程 {} 时还有别人持有它的引用",
        child.pid()
    );

    let child_pid = child.pid();
    let exit_code = child.inner_exclusive_access().exit_code;
    Ok(Some((child_pid, exit_code)))
}

/// `fork` 当前进程，把子进程放进就绪队列，返回子进程 PID。
pub fn fork_current() -> usize {
    let parent = current_task().expect("没有当前进程");
    let child = parent.fork();
    let child_pid = child.pid();

    // 子进程从 fork 返回时拿到 0 —— 这就是 fork "一次调用、两次返回"
    // 里那个能让两边区分彼此的唯一线索。
    child.inner_exclusive_access().trap_cx().x[10] = 0;

    add_task(child);
    child_pid
}

/// 用 `elf_data` 替换当前进程的地址空间。
pub fn exec_current(elf_data: &[u8]) {
    current_task()
        .expect("没有当前进程")
        .exec(elf_data);
}

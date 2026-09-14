//! 任务控制块。

use super::context::TaskContext;

/// 任务的生命周期状态。
///
/// 第 4 周只有这四种。第 9 周引入真正的进程后还会加上 `Zombie`
/// （已退出但父进程还没 wait 走它的退出码）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskStatus {
    /// 槽位还没被使用
    UnInit,
    /// 就绪，等待被调度
    Ready,
    /// 正在 CPU 上运行
    Running,
    /// 已退出
    Exited,
}

/// 任务控制块 (TCB)：内核眼里的"一个任务"就是这么点东西。
///
/// 第 4 周它只有状态和上下文。后面几周会逐步长大 ——
/// 第 7 周加地址空间，第 9 周加 PID 和父子关系，第 10 周加文件描述符表。
/// 这个结构体的膨胀史，就是操作系统抽象能力的成长史。
#[derive(Clone, Copy)]
pub struct TaskControlBlock {
    pub task_status: TaskStatus,
    pub task_cx: TaskContext,
}

impl TaskControlBlock {
    pub const fn uninit() -> Self {
        Self {
            task_status: TaskStatus::UnInit,
            task_cx: TaskContext::zero_init(),
        }
    }
}

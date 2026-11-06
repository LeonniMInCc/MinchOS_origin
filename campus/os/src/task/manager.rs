//! 就绪队列。
//!
//! 第 4–8 周的"调度器"是一张定长任务表加一个环形扫描。有了 `fork` 之后
//! 进程是动态来去的，改成一个先进先出队列更自然 —— 而且 FIFO 轮转
//! 和之前的环形扫描在行为上是一回事。
//!
//! 队列里放的是 `Arc<TaskControlBlock>`。用 `Arc` 是因为一个进程会被
//! 好几个地方同时引用：就绪队列、父进程的 `children`、以及
//! [`crate::task::processor`] 里的"当前进程"。

use super::tcb::TaskControlBlock;
use crate::sync::UPSafeCell;
use alloc::collections::VecDeque;
use alloc::sync::Arc;

struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

impl TaskManager {
    const fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
}

static TASK_MANAGER: UPSafeCell<TaskManager> =
    // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
    unsafe { UPSafeCell::new(TaskManager::new()) };

/// 把一个进程放进就绪队列队尾。
pub fn add_task(task: Arc<TaskControlBlock>) {
    TASK_MANAGER.exclusive_access().ready_queue.push_back(task);
}

/// 从队首取一个就绪进程。队列空时返回 `None`。
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    TASK_MANAGER.exclusive_access().ready_queue.pop_front()
}

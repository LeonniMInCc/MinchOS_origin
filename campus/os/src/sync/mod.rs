//! 同步。
//!
//! - [`UPSafeCell`]（第 2 周）：单核下的内部可变性
//! - [`WaitQueue`]（第 14 周）：让进程睡下、等事件来了再唤醒
//! - [`mutex`] / [`semaphore`] / [`condvar`]（第 14 周）：建在等待队列之上
//!
//! # 一个设计上的说明
//!
//! 真实系统里，互斥锁和条件变量是**进程内**的对象，用来同步同一个地址
//! 空间里的多个线程。CCNUOS 没有线程（每个进程独占一个地址空间），
//! 所以这三样做成了**系统级的内核对象**，用一个整数 ID 引用 ——
//! 更接近 System V 的信号量，而不是 pthread 的那一套。
//!
//! 这个选择让同步原语可以跨进程使用（哲学家就餐那种经典场景就成立了），
//! 代价是没有"锁属于谁"的概念，任何进程都能解开任何锁。
//! **这是个有意的简化，不是疏忽。**

mod condvar;
mod mutex;
mod semaphore;
mod up;
mod wait_queue;

pub use condvar::Condvar;
pub use mutex::Mutex;
pub use semaphore::Semaphore;
pub use up::UPSafeCell;
pub use wait_queue::WaitQueue;

use crate::task::{block_current_and_run_next, block_current_on_io, current_task, wakeup_task};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// 内核里所有同步对象的总表。
///
/// 用整数 ID 而不是指针：ID 要跨越用户/内核边界传递，而指针不能。
/// 这和文件描述符是同一个道理。
pub struct SyncTable {
    mutexes: Vec<Arc<Mutex>>,
    semaphores: Vec<Arc<Semaphore>>,
    condvars: Vec<Arc<Condvar>>,
}

impl SyncTable {
    const fn new() -> Self {
        Self {
            mutexes: Vec::new(),
            semaphores: Vec::new(),
            condvars: Vec::new(),
        }
    }
}

static SYNC_TABLE: UPSafeCell<SyncTable> =
    // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
    unsafe { UPSafeCell::new(SyncTable::new()) };

pub fn create_mutex() -> usize {
    let mut table = SYNC_TABLE.exclusive_access();
    table.mutexes.push(Arc::new(Mutex::new()));
    table.mutexes.len() - 1
}

pub fn get_mutex(id: usize) -> Option<Arc<Mutex>> {
    SYNC_TABLE.exclusive_access().mutexes.get(id).cloned()
}

pub fn create_semaphore(initial: usize) -> usize {
    let mut table = SYNC_TABLE.exclusive_access();
    table.semaphores.push(Arc::new(Semaphore::new(initial)));
    table.semaphores.len() - 1
}

pub fn get_semaphore(id: usize) -> Option<Arc<Semaphore>> {
    SYNC_TABLE.exclusive_access().semaphores.get(id).cloned()
}

pub fn create_condvar() -> usize {
    let mut table = SYNC_TABLE.exclusive_access();
    table.condvars.push(Arc::new(Condvar::new()));
    table.condvars.len() - 1
}

pub fn get_condvar(id: usize) -> Option<Arc<Condvar>> {
    SYNC_TABLE.exclusive_access().condvars.get(id).cloned()
}

// ---------------------------------------------------------------------------
// 阻塞与唤醒的公共动作
// ---------------------------------------------------------------------------

/// 把自己挂进队列然后睡下。
///
/// # 顺序不能反
///
/// 必须**先入队、再阻塞**。反过来的话，在"阻塞"和"入队"之间事件就发生了，
/// 唤醒者会发现队列是空的，于是什么都不做 —— 而我们已经睡下了，
/// 再也醒不过来。这就是经典的 **lost wakeup**。
///
/// 本项目的内核不可抢占（第 5 章），所以这两步之间不会被打断，
/// 顺序对了就足够。真实的可抢占内核还需要一把锁把两步包起来。
pub fn wait_on(queue: &WaitQueue) {
    let task = current_task().expect("没有当前进程");
    queue.push(task);
    block_current_and_run_next();
}

/// 同 [`wait_on`]，但告诉调度器"这是在等 I/O，不是在等别的进程"。
///
/// 这个区分只有一个用途，但很重要：idle 发现就绪队列空了时，
/// 要能分清**死锁**（大家互相等，永远不会好）和**等 I/O**
/// （硬件迟早会来中断）。前者该报错，后者该 `wfi` 睡着等。
pub fn wait_on_io(queue: &WaitQueue) {
    let task = current_task().expect("没有当前进程");
    queue.push(task);
    block_current_on_io();
}

/// 唤醒队列里的一个等待者。
pub fn notify_one(queue: &WaitQueue) -> bool {
    match queue.pop() {
        Some(task) => {
            wakeup_task(task);
            true
        }
        None => false,
    }
}

/// 唤醒队列里的全部等待者。返回唤醒了几个。
pub fn notify_all(queue: &WaitQueue) -> usize {
    let waiters = queue.drain();
    let n = waiters.len();
    for task in waiters {
        wakeup_task(task);
    }
    n
}

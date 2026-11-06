//! 条件变量。

use super::{notify_all, notify_one, wait_on, Mutex, WaitQueue};
use alloc::sync::Arc;

/// 条件变量：等一个**条件**成立，而不是等一把锁。
///
/// # 它和互斥锁解决的是不同的问题
///
/// 互斥锁回答"这块数据现在归谁"，条件变量回答"我要的状态到了吗"。
/// 典型用法是两者配合：
///
/// ```text
///   消费者                          生产者
///   lock(m)                         lock(m)
///   while 队列为空 {                 往队列里放东西
///       condvar.wait(m)   ←──────    condvar.signal()
///   }                               unlock(m)
///   取走东西
///   unlock(m)
/// ```
///
/// # `wait` 为什么必须原子地"放锁 + 睡下"
///
/// 如果先 `unlock` 再 `wait`，这中间生产者可能已经 `signal` 过了 ——
/// 那次通知落空，消费者随后睡下就再也醒不来。这是经典的 **lost wakeup**。
///
/// 我们的内核不可抢占（第 5 章），`unlock` 和 `wait` 之间不会被打断，
/// 所以顺序对了就够。可抢占的内核需要把这两步真正做成原子的。
///
/// # 为什么调用方要用 `while` 而不是 `if`
///
/// 被唤醒**不等于**条件成立：可能有别的进程抢先把东西取走了。
/// 所以醒来之后必须重新检查条件。这叫"虚假唤醒"，是所有条件变量
/// 实现的共同约定。
pub struct Condvar {
    waiters: WaitQueue,
}

impl Condvar {
    pub const fn new() -> Self {
        Self {
            waiters: WaitQueue::new(),
        }
    }

    /// 放开锁、睡下；被唤醒后重新上锁。
    pub fn wait(&self, mutex: &Arc<Mutex>) {
        mutex.unlock();
        wait_on(&self.waiters);
        mutex.lock();
    }

    /// 唤醒一个等待者。
    pub fn signal(&self) -> bool {
        notify_one(&self.waiters)
    }

    /// 唤醒所有等待者。
    pub fn broadcast(&self) -> usize {
        notify_all(&self.waiters)
    }
}

impl Default for Condvar {
    fn default() -> Self {
        Self::new()
    }
}

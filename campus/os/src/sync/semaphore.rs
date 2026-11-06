//! 计数信号量。

use super::{notify_one, wait_on, UPSafeCell, WaitQueue};

/// 一个计数信号量。
///
/// 可以理解成"有 N 张票"：`down` 取一张（没票就等），`up` 还一张。
///
/// - N = 1 时它就是一把互斥锁
/// - N = 0 起步时它是一个"事件通知"：等的人先到，发的人后到
///
/// **信号量比互斥锁更通用，但也更容易用错** —— 它没有"谁持有"的概念，
/// 任何进程都可以 `up`。这既是它的威力（可以做跨进程的事件通知），
/// 也是它的危险（配对错了就是死锁或者计数泄漏）。
pub struct Semaphore {
    count: UPSafeCell<isize>,
    waiters: WaitQueue,
}

impl Semaphore {
    pub fn new(initial: usize) -> Self {
        Self {
            // SAFETY: 单核，访问都在 exclusive_access 的短作用域内。
            count: unsafe { UPSafeCell::new(initial as isize) },
            waiters: WaitQueue::new(),
        }
    }

    /// P 操作：取一张票，没票就阻塞。
    pub fn down(&self) {
        let should_wait = {
            let mut count = self.count.exclusive_access();
            *count -= 1;
            *count < 0
            // 借用在这里结束，然后才敢阻塞
        };
        if should_wait {
            wait_on(&self.waiters);
        }
    }

    /// V 操作：还一张票，有人在等就唤醒一个。
    ///
    /// 注意计数可以是**负数** —— 它的绝对值正是等待者的个数。
    /// 这个技巧让 `down` 只需要一次判断，不必先看队列再改计数。
    pub fn up(&self) {
        let has_waiter = {
            let mut count = self.count.exclusive_access();
            *count += 1;
            *count <= 0
        };
        if has_waiter {
            notify_one(&self.waiters);
        }
    }
}

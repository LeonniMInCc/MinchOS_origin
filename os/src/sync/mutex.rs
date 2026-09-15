//! 互斥锁（阻塞式）。

use super::{notify_one, wait_on, UPSafeCell, WaitQueue};

/// 一把阻塞式的互斥锁。
///
/// # 为什么不是自旋锁
///
/// 自旋锁在等待时死死占着 CPU。在单核系统上这是**最坏的选择** ——
/// 持锁者根本没机会运行来释放锁，自旋只能一直转到时间片用完。
///
/// 阻塞锁把等待者从调度器视野里拿掉，CPU 立刻给别人（很可能就是持锁者）。
/// 代价是每次竞争要多两次上下文切换。
///
/// 规律：**争用少、临界区短**用自旋；**争用多、临界区长**用阻塞。
/// 单核内核里几乎总该用阻塞。
pub struct Mutex {
    inner: UPSafeCell<bool>,
    waiters: WaitQueue,
}

impl Mutex {
    pub const fn new() -> Self {
        Self {
            // SAFETY: 单核，访问都在 exclusive_access 的短作用域内。
            inner: unsafe { UPSafeCell::new(false) },
            waiters: WaitQueue::new(),
        }
    }

    /// 上锁。已经被别人持有时阻塞。
    pub fn lock(&self) {
        loop {
            let locked = {
                let mut held = self.inner.exclusive_access();
                if *held {
                    true
                } else {
                    *held = true;
                    false
                }
                // 借用必须在这里结束 —— 下面要阻塞，
                // 而阻塞会切走执行流，借用标记就再也放不掉了。
                // 这条规矩从第 10 章起反复出现。
            };

            if !locked {
                return;
            }
            wait_on(&self.waiters);
            // 被唤醒之后要**重新抢**：唤醒不等于拿到锁，
            // 中间可能有别的进程插队。所以这里是 loop 而不是 if。
        }
    }

    /// 解锁，并唤醒一个等待者（如果有）。
    pub fn unlock(&self) {
        *self.inner.exclusive_access() = false;
        notify_one(&self.waiters);
    }
}

impl Default for Mutex {
    fn default() -> Self {
        Self::new()
    }
}

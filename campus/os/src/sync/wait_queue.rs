//! 等待队列：让进程睡下，等事件来了再唤醒。
//!
//! # 这是第 14 周真正的核心
//!
//! 前面几章留下的三笔账，根子都是同一件事 —— 缺少"阻塞/唤醒"这个机制：
//!
//! | 欠账 | 当时的权宜之计 | 代价 |
//! |---|---|---|
//! | `Stdin::read`（第 10 章） | 没输入就 `yield` 重试 | 白烧时间片 |
//! | `waitpid`（第 9 章） | 返回 -2 让用户库循环重试 | 同上，还多一趟系统调用 |
//! | 串口输入（第 10 章） | 轮询取字符 | 两次轮询之间的字符会丢 |
//!
//! 轮询能工作，是因为调度器总会把 CPU 分给别人。但它有两个真实代价：
//! **浪费**（等待者反复被调度、什么也干不成）和**丢事件**
//! （没人盯着的那段时间里发生的事，没人知道）。
//!
//! 等待队列同时解决这两个问题：等待者从调度器视野里消失，
//! 事件发生时由**事件源**负责把它放回来。

use super::UPSafeCell;
use crate::task::TaskHandle;
use alloc::collections::VecDeque;

/// 一组正在等同一件事的进程。
pub struct WaitQueue {
    inner: UPSafeCell<VecDeque<TaskHandle>>,
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self {
            // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
            inner: unsafe { UPSafeCell::new(VecDeque::new()) },
        }
    }

    /// 把一个进程挂进队列。调用方随后应该立刻阻塞自己。
    pub fn push(&self, task: TaskHandle) {
        self.inner.exclusive_access().push_back(task);
    }

    /// 取出一个等待者（先进先出，避免饿死）。
    pub fn pop(&self) -> Option<TaskHandle> {
        self.inner.exclusive_access().pop_front()
    }

    /// 取出全部等待者。
    pub fn drain(&self) -> VecDeque<TaskHandle> {
        core::mem::take(&mut self.inner.exclusive_access())
    }
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}

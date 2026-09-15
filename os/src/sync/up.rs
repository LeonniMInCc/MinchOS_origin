//! 单核环境下的内部可变性。
//!
//! 内核里有大量"全局唯一、且需要可变"的对象：堆分配器、任务管理器、
//! 文件描述符表。但 Rust 要求 `static` 必须是 `Sync` 的，而
//! [`RefCell`] 不是 —— 它假定自己可能被多线程访问。
//!
//! [`UPSafeCell`] 的作用就是手动声明 `Sync`，并把"凭什么这样是安全的"
//! 这个责任显式地交给调用方。这不是在糊弄借用检查器，而是在**记账**：
//! 每一个 `unsafe` 都对应一条我们必须自己维护的不变量。

use core::cell::{RefCell, RefMut};

/// Uni-Processor Safe Cell —— 在"只有一个 hart，且访问期间不会被抢占"
/// 的前提下，为全局可变状态提供访问。
///
/// # 这个前提什么时候会失效
///
/// - **第 5 周开启时钟中断后**：中断处理程序可能在临界区中间插进来，
///   如果它也访问同一个 `UPSafeCell`，`borrow_mut` 会 panic。
///   届时需要升级成"访问期间关中断"。
/// - **引入多核后**：单核假设直接崩塌，需要换成真正的自旋锁。
///
/// 现在先记下这笔账。用 `RefCell` 而不是裸 `UnsafeCell` 正是为了这个：
/// 违反不变量时会得到一个明确的 panic，而不是一段诡异的内存损坏。
pub struct UPSafeCell<T> {
    inner: RefCell<T>,
}

/// # Safety
///
/// 仅在单核、且访问期间不被抢占的前提下成立。见类型文档。
unsafe impl<T> Sync for UPSafeCell<T> {}

impl<T> UPSafeCell<T> {
    /// # Safety
    ///
    /// 调用方需保证后续访问满足上述前提。
    pub const unsafe fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }

    /// 取得独占访问。
    ///
    /// 同一时刻只能存在一个。重复借用会 panic —— 这个 panic 是好事：
    /// 它把"内核里的数据竞争"变成了一个有文件名和行号的崩溃现场，
    /// 而不是三天后某个无法复现的诡异 bug。
    pub fn exclusive_access(&self) -> RefMut<'_, T> {
        self.inner.borrow_mut()
    }
}

//! 第二章仅在单核运行，尚未开启可能重入分配器的中断。
//! 后续开启抢占或多核时必须升级临界区保护。
use core::cell::{RefCell, RefMut};
pub struct UPSafeCell<T> {
    inner: RefCell<T>,
}
// SAFETY: 本实验只有一个 hart，且访问期间不会被中断重入。
unsafe impl<T> Sync for UPSafeCell<T> {}
impl<T> UPSafeCell<T> {
    /// SAFETY: 调用方保证单核、不可重入的不变量。
    pub const unsafe fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }
    pub fn exclusive_access(&self) -> RefMut<'_, T> {
        self.inner.borrow_mut()
    }
}

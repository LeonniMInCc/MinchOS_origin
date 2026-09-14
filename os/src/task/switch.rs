//! `__switch` 的 Rust 声明。

use super::context::TaskContext;
use core::arch::global_asm;

global_asm!(include_str!("switch.S"));

extern "C" {
    /// 保存当前执行流到 `current`，恢复 `next` 指向的执行流。
    ///
    /// 从 Rust 侧看这是一次普通的函数调用，会正常返回 —— 但"返回"的那一刻
    /// 可能已经过去了很久，中间跑过了别的任务。详见 `switch.S` 的注释。
    ///
    /// # Safety
    ///
    /// 两个指针都必须指向有效的、生命周期足够长的 [`TaskContext`]。
    /// `current` 会被写入，`next` 只读。
    pub fn __switch(current: *mut TaskContext, next: *const TaskContext);
}

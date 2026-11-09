//! 时间系统调用。

/// `get_time()` —— 返回 SBI 单调计时器的当前值。
pub fn sys_get_time() -> isize {
    crate::sbi::get_time() as isize
}

//! 语言项（lang items）。
//!
//! `#![no_std]` 下没有标准库替我们兜底，panic 发生时该干什么必须自己定义。
//! 对内核来说，panic 意味着内核自身出了不可恢复的错误 —— 打印现场，然后关机。

use crate::sbi::shutdown;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    match info.location() {
        Some(location) => {
            println!(
                "\u{1b}[31m[KERNEL PANIC] {}:{}:{} — {}\u{1b}[0m",
                location.file(),
                location.line(),
                location.column(),
                info.message()
            );
        }
        None => {
            println!("\u{1b}[31m[KERNEL PANIC] {}\u{1b}[0m", info.message());
        }
    }
    shutdown(true)
}

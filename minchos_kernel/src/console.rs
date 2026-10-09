//! 串口输出模块 —— 为内核接上 print! / println! 宏
//!
//! Rust 的格式化系统 (core::fmt) 与输出设备是解耦的：
//! 只要给某个类型实现 core::fmt::Write trait 的 write_str 方法，
//! 就能把整套 format_args! 的能力"嫁接"过来。
//!
//! 这里我们用一个零大小类型 Stdout 挂载 Write 实现，
//! 底层调用 sbi.rs 里的 console_putchar 逐字节输出。

use crate::sbi::console_putchar;
use core::fmt::{self, Write};

/// 零大小类型，仅用于挂载 Write trait
struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // 注意：必须按 bytes() 遍历，不能用 chars()
        // 因为 console_putchar 一次接受一个字节，
        // 而 UTF-8 编码的中文是多字节的。
        // 如果用 chars() 然后 as usize，拿到的是 Unicode 码点，
        // 只有低 8 位会被写出，中文就乱码了。
        for byte in s.bytes() {
            console_putchar(byte as usize);
        }
        Ok(())
    }
}

/// 格式化输出（供 print! 宏使用）
pub fn print(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}

/// 类似标准库的 print! 宏
#[macro_export]
macro_rules! print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!($fmt $(, $($arg)+)?));
    }
}

/// 类似标准库的 println! 宏
#[macro_export]
macro_rules! println {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?));
    }
}

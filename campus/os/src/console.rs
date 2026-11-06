//! 内核的格式化输出。
//!
//! 内核跑在裸机上，没有 libc、没有 stdout。这里手动给一个零大小的
//! `Stdout` 类型实现 [`core::fmt::Write`]，把每个字节交给 SBI 串口，
//! 就能白嫖 Rust 的整套 `format_args!` 机制，用上 `println!`。

use crate::sbi::console_putchar;
use core::fmt::{self, Write};

/// 标准输出。零大小类型，不占内存，只是 `Write` 实现的挂载点。
struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // 按**字节**而不是按 char 发送。SBI 的 console_putchar 一次只接受
        // 一个字节，而一个中文字符在 UTF-8 下占 3 个字节 —— 按 char 发送的话
        // `c as usize` 得到的是 Unicode 码点（如 0x5185），送进去只有低 8 位
        // 会被写出，中文就全成了乱码。逐字节发送则把 UTF-8 原样交给终端，
        // 由终端负责重新组装。
        for byte in s.bytes() {
            console_putchar(byte as usize);
        }
        Ok(())
    }
}

/// 输出格式化内容。`print!` / `println!` 宏的落地点。
pub fn print(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}

/// 打印，不换行。
#[macro_export]
macro_rules! print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!($fmt $(, $($arg)+)?))
    };
}

/// 打印并换行。
#[macro_export]
macro_rules! println {
    () => {
        $crate::console::print(format_args!("\n"))
    };
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
    };
}

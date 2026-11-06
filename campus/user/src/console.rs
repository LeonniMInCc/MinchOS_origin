//! 用户态的格式化输出。
//!
//! 和内核里的 `console.rs` 几乎一样，唯一的区别是最底层：
//! 内核直接 `ecall` 给 SBI 固件，用户程序 `ecall` 给**内核**。
//!
//! 这个对称性值得注意 —— 每一层都只认识自己下面那一层。

use crate::syscall::sys_write;
use core::fmt::{self, Write};

const STDOUT: usize = 1;

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // 一次系统调用写完整个字符串，而不是逐字节调用 ——
        // 每次 ecall 都要陷入内核再返回，代价远高于一次函数调用。
        sys_write(STDOUT, s.as_bytes());
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!($fmt $(, $($arg)+)?))
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::console::print(format_args!("\n"))
    };
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
    };
}

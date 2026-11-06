//! 管道演示：把标准输入原样转成大写，写到标准输出。
//!
//! 一个典型的"过滤器"程序：读 stdin、变换、写 stdout。
//! 它既可以单独用（那时 stdin 是键盘），也可以夹在管道中间。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{read, STDIN};

#[no_mangle]
pub fn main() -> i32 {
    let mut buf = [0u8; 64];
    loop {
        let n = read(STDIN, &mut buf);
        if n <= 0 {
            break; // 0 = 写端全关了（EOF），负数 = 出错
        }
        for byte in buf.iter_mut().take(n as usize) {
            *byte = byte.to_ascii_uppercase();
        }
        // 直接写字节，不走 println! —— 输入未必是完整的行
        let text = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
        print!("{}", text);
    }
    0
}

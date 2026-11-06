//! 管道演示：统计标准输入的行数和字节数。
//!
//! 管道末端的典型程序：只读不写（除了最后那行结果）。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{read, STDIN};

#[no_mangle]
pub fn main() -> i32 {
    let mut buf = [0u8; 64];
    let (mut lines, mut bytes) = (0usize, 0usize);

    loop {
        let n = read(STDIN, &mut buf);
        if n <= 0 {
            break;
        }
        bytes += n as usize;
        lines += buf[..n as usize].iter().filter(|&&b| b == b'\n').count();
    }

    println!("{} 行，{} 字节", lines, bytes);
    0
}

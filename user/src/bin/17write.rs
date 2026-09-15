//! 往磁盘上写一个文件。
//!
//! 和 `16cat` 配合演示持久化：先跑这个，再跑 `16cat`，
//! **然后重启 QEMU 再跑一次 `16cat`** —— 内容还在。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{close, get_time_ms, open, open_flags, write};

const FILE: &str = "readme.txt";

#[no_mangle]
pub fn main() -> i32 {
    // WRONLY | CREATE | TRUNC：不存在就建，存在就清空重写
    let flags = open_flags::WRONLY | open_flags::CREATE | open_flags::TRUNC;
    let fd = open(FILE, flags);
    if fd < 0 {
        println!("write: 打不开 {}", FILE);
        return -1;
    }
    let fd = fd as usize;

    // 把开机时刻写进去，这样每次重写的内容都不一样，
    // 一眼就能看出 cat 读到的是不是这一次写的
    let stamp = get_time_ms();
    let mut line = [0u8; 96];
    let text = b"CCNUOS file system works.\nwritten at ";
    line[..text.len()].copy_from_slice(text);
    let mut len = text.len();
    len += write_usize(&mut line[len..], stamp);
    line[len] = b'\n';
    len += 1;

    let n = write(fd, &line[..len]);
    close(fd);

    println!("write: 往 {} 写了 {} 字节（时间戳 {} ms）", FILE, n, stamp);
    println!("write: 现在可以跑 16cat 看看 —— 重启之后再跑一次，内容还在。");
    0
}

/// 把一个数字写成十进制文本。用户库没有 `format!`（没有堆）。
fn write_usize(buf: &mut [u8], mut n: usize) -> usize {
    if n == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut digits = [0u8; 20];
    let mut count = 0;
    while n > 0 {
        digits[count] = b'0' + (n % 10) as u8;
        n /= 10;
        count += 1;
    }
    for i in 0..count {
        buf[i] = digits[count - 1 - i];
    }
    count
}

//! 恶意程序之一：给系统调用传一个指向**内核内存**的指针。
//!
//! 如果内核不加检查地照单全收，就会把自己的内存内容打印出来 ——
//! 这是一个真实的信息泄露漏洞。
//!
//! 预期结果：内核的 `is_user_accessible` 拦下来，返回 -1，
//! 而这个程序本身不会被杀掉（它没有做任何硬件层面的违规），
//! 只是拿到一个错误码。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::sys_write_raw;

/// 内核代码段的起始地址。用户程序本来完全不该知道、更不该碰这个地址。
const KERNEL_TEXT: usize = 0x8020_0000;

#[no_mangle]
pub fn main() -> i32 {
    println!("我要试着让内核把它自己的内存打印给我看。");

    let ret = sys_write_raw(1, KERNEL_TEXT, 64);
    println!(
        "sys_write(fd=1, buf={:#x}, len=64) 返回 {}",
        KERNEL_TEXT, ret
    );

    if ret < 0 {
        println!("被内核拒绝了 —— 这是正确的行为。");
    } else {
        println!("内核居然照做了！这是一个信息泄露漏洞。");
        return -1;
    }

    // 再试一个会导致整数溢出的参数：start + len 会绕回一个小数字，
    // 天真的范围检查（start >= X && start + len <= Y）会被骗过去。
    let ret = sys_write_raw(1, usize::MAX - 8, 64);
    println!("sys_write(buf={:#x}, len=64) 返回 {}", usize::MAX - 8, ret);

    if ret < 0 {
        println!("溢出参数也被挡住了。");
        0
    } else {
        println!("溢出检查有漏洞！");
        -1
    }
}

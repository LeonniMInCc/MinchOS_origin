//! 恶意程序之二：在 U 态执行只有 S 态才能用的指令。
//!
//! `sret` 是"从内核返回用户态"的指令。用户程序执行它，相当于
//! 试图自己给自己提权。
//!
//! 预期结果：CPU 抛出 IllegalInstruction 异常，内核捕获后杀掉这个程序，
//! 换下一个继续跑 —— **内核不会崩溃**。这正是操作系统该有的样子。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use core::arch::asm;

#[no_mangle]
pub fn main() -> i32 {
    println!("我要执行一条 S 态特权指令 sret，试图提权。");

    unsafe {
        asm!("sret");
    }

    println!("如果你看到这行，说明内核没拦住我 —— 那是个严重问题。");
    -1
}

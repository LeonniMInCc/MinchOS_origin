//! `ls` —— 列出根目录下的文件。
//!
//! 走的是 `sys_listdir`，一个非 POSIX 的简化接口：内核直接返回一串
//! 换行分隔的名字。真实系统用 `getdents`，返回结构化的目录项。
//!
//! 我们只有一个扁平的根目录、十几个文件，所以够用了 ——
//! **接口的复杂度应该配得上需求**。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::listdir;

#[no_mangle]
pub fn main() -> i32 {
    let mut buf = [0u8; 1024];
    let n = listdir(&mut buf);
    if n < 0 {
        println!("ls: 读取目录失败");
        return -1;
    }

    let text = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
    let mut count = 0;
    for name in text.lines() {
        println!("  {}", name);
        count += 1;
    }
    println!("共 {} 个文件", count);
    0
}

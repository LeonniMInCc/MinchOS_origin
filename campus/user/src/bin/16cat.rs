//! `cat` —— 把一个文件的内容打印出来。
//!
//! 因为 shell 还不支持命令行参数（`exec` 只传程序名），这个版本固定
//! 读一个演示文件。加参数需要在 `exec` 里传 argv，那是另一件事。
//!
//! 它演示的是完整的 `open` → `read` → `close` 流程，
//! 而这条流程走的正是第 10 章那张文件描述符表。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{close, open, open_flags, read};

const FILE: &str = "readme.txt";

#[no_mangle]
pub fn main() -> i32 {
    let fd = open(FILE, open_flags::RDONLY);
    if fd < 0 {
        println!("cat: 打不开 {}（先用 17write 创建它）", FILE);
        return -1;
    }
    let fd = fd as usize;

    let mut buf = [0u8; 64];
    let mut total = 0usize;
    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        let text = core::str::from_utf8(&buf[..n as usize]).unwrap_or("<非法UTF8>");
        print!("{}", text);
        total += n as usize;
    }
    close(fd);

    println!();
    println!("cat: 共 {} 字节", total);
    0
}

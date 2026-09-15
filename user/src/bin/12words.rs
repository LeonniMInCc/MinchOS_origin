//! 管道演示：往标准输出吐几行文字。
//!
//! 它完全不知道自己的输出会去哪里 —— 可能是屏幕，也可能是另一个进程的
//! 标准输入。**这种"不知道"正是管道能成立的原因**：程序只管往 fd 1 写，
//! 至于 fd 1 连着什么，是 shell 在 `exec` 之前布置好的。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

#[no_mangle]
pub fn main() -> i32 {
    for line in [
        "hello from ccnuos",
        "pipes connect processes",
        "everything is a file",
    ] {
        println!("{}", line);
    }
    0
}

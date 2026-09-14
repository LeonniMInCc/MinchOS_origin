//! 最简单的用户程序：打印几行字，正常退出。
//!
//! 它能跑通，就意味着这条链路全线打通了：
//! 内核伪造 TrapContext → `__restore` → `sret` 进 U 态 → 程序执行 →
//! `ecall` → `__alltraps` 保存现场 → `trap_handler` → `sys_write` →
//! `__restore` 恢复现场 → 程序继续 → `exit`。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

#[no_mangle]
pub fn main() -> i32 {
    println!("你好，我是跑在 U 态的第一个用户程序。");
    println!("我刚才那句话，是通过 ecall 请内核帮我打印的。");
    print!("不换行的输出也能用");
    println!("，拼接正常。");
    println!("现在我要 exit(0) 了。");
    0
}

//! 算一会儿数：验证用户程序能长时间占着 CPU 做纯计算，
//! 中间穿插系统调用也不会把现场弄乱。
//!
//! 这一章是批处理系统，这个程序会一直跑到自己算完为止 ——
//! 内核没有任何手段把它抢走。第 5 周引入时钟中断后就不一样了。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

const P: u64 = 998_244_353;
const STEP: usize = 100_000;

#[no_mangle]
pub fn main() -> i32 {
    println!("开始计算 3^n mod {}", P);

    let mut power: u64 = 1;
    for i in 1..=STEP {
        power = power * 3 % P;
        if i % 20_000 == 0 {
            // 每算两万次汇报一次。每一次汇报都是一趟完整的
            // U 态 → S 态 → U 态往返 —— 寄存器现场必须分毫不差，
            // 否则下面的 power 就会算错。
            println!("  3^{:<6} mod {} = {}", i, P, power);
        }
    }

    println!(
        "算完了，{} 次乘法加 {} 次系统调用，现场没乱。",
        STEP,
        STEP / 20_000 + 1
    );
    0
}

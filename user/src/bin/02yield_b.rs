//! 协作式调度演示 B。见 `01yield_a.rs`。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::yield_now;

const ROUNDS: usize = 4;

#[no_mangle]
pub fn main() -> i32 {
    for round in 0..ROUNDS {
        println!("  [B] 第 {} 轮 —— 让出 CPU", round);
        yield_now();
    }
    println!("  [B] 四轮跑完，退出");
    0
}

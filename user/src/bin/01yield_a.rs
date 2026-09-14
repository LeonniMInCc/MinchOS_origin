//! 协作式调度演示 A。
//!
//! 三个几乎一样的程序 A / B / C，各自打印一行然后主动让出 CPU。
//! 把它们的输出交错在一起，就能直观看到调度器在轮转。
//!
//! 注意：每一次 `yield_now()` 返回时，**中间已经跑过了 B 和 C**。
//! 从程序自己的视角看，只是这一行执行得慢了点。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::yield_now;

const ROUNDS: usize = 4;

#[no_mangle]
pub fn main() -> i32 {
    for round in 0..ROUNDS {
        println!("  [A] 第 {} 轮 —— 让出 CPU", round);
        yield_now();
    }
    println!("  [A] 四轮跑完，退出");
    0
}

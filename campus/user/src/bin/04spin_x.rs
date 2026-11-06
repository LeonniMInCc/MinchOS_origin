//! 抢占演示 X：**从头到尾一次都不 yield** 的计算密集程序。
//!
//! 这是第 4 周那个"霸占者"的升级版。程序本身一个字都没改，改的是内核：
//!
//! - **第 4 周（协作式）**：它一旦拿到 CPU 就再不撒手，
//!   其他任务哪怕就绪也只能干等，直到它算完
//! - **第 5 周（抢占式）**：时钟中断每 10 ms 强行把 CPU 收回去，
//!   于是它和 Y 交替推进
//!
//! **这正是"抢占"的含义：调度不再需要任务配合。**
//!
//! # 怎么看出自己被抢占了
//!
//! 每段计算的工作量是固定的，独占 CPU 时耗时应该也固定。
//! 所以只要打印每段的**墙钟耗时**，就能看出差别：
//! 和 Y 并行时，每段的墙钟时间会明显拉长 —— 多出来的部分就是 Y 用掉的。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::get_time_ms;

const P: u64 = 998_244_353;
/// 每段的乘法次数。调到让一段计算明显长于一个时间片（10 ms），
/// 这样每段都必然被打断至少一次。
const WORK: usize = 5_000_000;
const SEGMENTS: usize = 6;

#[no_mangle]
pub fn main() -> i32 {
    println!("  [X] 开始。我从头到尾一次都不调 yield。");
    let begin = get_time_ms();

    let mut acc: u64 = 1;
    for seg in 0..SEGMENTS {
        let t0 = get_time_ms();
        for _ in 0..WORK {
            acc = acc * 3 % P;
        }
        let t1 = get_time_ms();
        println!("  [X] 第 {} 段算完，墙钟 {} ms", seg, t1 - t0);
    }

    let total = get_time_ms() - begin;
    println!("  [X] 全部算完，总墙钟 {} ms，acc = {}", total, acc);
    0
}

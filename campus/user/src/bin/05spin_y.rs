//! 抢占演示 Y。和 `04spin_x.rs` 结构相同，用来和它交错。
//!
//! 两个都不 yield 的程序能交替推进 —— 这件事在第 4 周是不可能的。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::get_time_ms;

const P: u64 = 998_244_353;
const WORK: usize = 5_000_000;
const SEGMENTS: usize = 6;

#[no_mangle]
pub fn main() -> i32 {
    println!("  [Y] 开始。我也一次都不调 yield。");
    let begin = get_time_ms();

    let mut acc: u64 = 2;
    for seg in 0..SEGMENTS {
        let t0 = get_time_ms();
        for _ in 0..WORK {
            acc = acc * 5 % P;
        }
        let t1 = get_time_ms();
        println!("  [Y] 第 {} 段算完，墙钟 {} ms", seg, t1 - t0);
    }

    let total = get_time_ms() - begin;
    println!("  [Y] 全部算完，总墙钟 {} ms，acc = {}", total, acc);
    0
}

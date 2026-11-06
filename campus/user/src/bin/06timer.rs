//! 计时：用 `sys_get_time` 观察"被抢占"这件事的代价。
//!
//! 用户程序读不到 `time` CSR —— 那是 S 态寄存器，U 态执行 `csrr` 会
//! 触发非法指令异常。所以时间也得向内核要。
//!
//! 这个程序做一段固定量的计算，测它的**墙钟耗时**。在抢占式调度下，
//! 这个数字会明显大于"独占 CPU 时需要的时间"，差额就是被别的任务
//! 占走的部分。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{get_time_ms, yield_now};

const P: u64 = 998_244_353;
const WORK: usize = 5_000_000;

fn burn() -> u64 {
    let mut acc: u64 = 1;
    for _ in 0..WORK {
        acc = acc * 7 % P;
    }
    acc
}

#[no_mangle]
pub fn main() -> i32 {
    let t0 = get_time_ms();
    let a = burn();
    let t1 = get_time_ms();

    println!("  [计时] {} 次乘法，墙钟耗时 {} ms（acc={}）", WORK, t1 - t0, a);
    println!("  [计时] 这里面既有我自己算的时间，也有被别的任务抢走的时间。");

    // 主动睡一小会儿：反复 yield 直到过了 50 ms。
    // 这是最朴素的"定时等待" —— 没有阻塞队列，只能忙等着让出 CPU。
    // 真正的 sleep 要等到有了等待队列才做得好。
    let deadline = get_time_ms() + 50;
    let mut yields = 0u32;
    while get_time_ms() < deadline {
        yield_now();
        yields += 1;
    }

    println!("  [计时] 忙等 50 ms 期间，我主动让出了 {} 次 CPU。", yields);
    0
}

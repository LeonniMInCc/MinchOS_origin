//! 一个**不肯让出 CPU** 的计算密集程序。
//!
//! 这是本章要暴露的问题：协作式调度完全依赖任务自觉。
//! 这个程序从头算到尾一次 `yield` 都不调，于是在它跑完之前，
//! A / B / C 全都动不了 —— 哪怕它们已经就绪。
//!
//! 看运行日志里 [A][B][C] 的交替是怎么被这个程序打断的，
//! 就明白第 5 周为什么必须引入时钟中断了。
//!
//! 更极端的情况：把主体换成 `loop {}`，整个系统就永远卡死。
//! 内核没有任何手段抢回控制权。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

const P: u64 = 998_244_353;
const STEP: usize = 30_000;

#[no_mangle]
pub fn main() -> i32 {
    println!("  [霸占者] 我要连算 {} 次乘法，中途一次都不 yield。", STEP);

    let mut power: u64 = 1;
    for i in 1..=STEP {
        power = power * 3 % P;
        if i % 10_000 == 0 {
            // 这里虽然有系统调用，但 sys_write 不会让出 CPU ——
            // 陷入内核、打印、再返回，整个过程当前任务始终占着 CPU。
            // **陷入内核 ≠ 让出 CPU**，这两件事要分清楚。
            println!("  [霸占者] 3^{} mod {} = {}", i, P, power);
        }
    }

    println!("  [霸占者] 算完了，现在才轮到别人。");
    0
}

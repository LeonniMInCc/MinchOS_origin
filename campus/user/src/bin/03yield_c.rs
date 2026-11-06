//! 地址空间隔离演示 C。见 `01yield_a.rs` 的说明。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::yield_now;

const ROUNDS: usize = 4;
const MAGIC: u64 = 0xCCCC;

static mut MARK: u64 = MAGIC;

#[no_mangle]
pub fn main() -> i32 {
    let addr = &raw const MARK as usize;
    for round in 0..ROUNDS {
        // SAFETY: 单线程程序，这个全局只有自己会碰。
        let value = unsafe { MARK };
        println!(
            "  [C] 第 {} 轮：MARK @ {:#x} = {:#x} —— 让出 CPU",
            round, addr, value
        );
        assert_eq!(value, MAGIC, "[C] 全局变量被别的进程改了，隔离失效");
        yield_now();
    }
    println!("  [C] 四轮跑完，MARK 始终是 {:#x}，隔离完好", MAGIC);
    0
}

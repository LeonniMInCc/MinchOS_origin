//! 协作式调度 + **地址空间隔离**演示 A。
//!
//! A / B / C 是三个独立的程序，但它们链接到完全相同的虚拟地址
//! （见 `user/src/linker.ld`，都是 `0x10000`）。所以三者的全局变量
//! `MARK` 在各自地址空间里的地址是**同一个数**。
//!
//! 每一轮打印这个地址和它的值：
//!
//! - **地址三者相同** → 它们确实用着同一段虚拟地址
//! - **值各不相同且始终不变** → 那是三份互不相干的物理内存
//!
//! 第 8 周之前做不到这件事：那时所有程序挤在同一片物理内存里，
//! 必须被链接到互不重叠的槽位。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::yield_now;

const ROUNDS: usize = 4;
const MAGIC: u64 = 0xAAAA;

/// 一个普通的全局变量。三个程序里它的**虚拟地址完全相同**。
static mut MARK: u64 = MAGIC;

#[no_mangle]
pub fn main() -> i32 {
    let addr = &raw const MARK as usize;
    for round in 0..ROUNDS {
        // SAFETY: 单线程程序，这个全局只有自己会碰。
        let value = unsafe { MARK };
        println!(
            "  [A] 第 {} 轮：MARK @ {:#x} = {:#x} —— 让出 CPU",
            round, addr, value
        );
        assert_eq!(value, MAGIC, "[A] 全局变量被别的进程改了，隔离失效");
        yield_now();
    }
    println!("  [A] 四轮跑完，MARK 始终是 {:#x}，隔离完好", MAGIC);
    0
}

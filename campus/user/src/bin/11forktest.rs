//! `fork` 语义演示。
//!
//! 三件事：
//!
//! 1. **一次调用，两次返回** —— 父进程拿到子进程 PID，子进程拿到 0，
//!    两边从同一条指令继续往下跑
//! 2. **子进程是副本，不是共享** —— 父子的全局变量在**同一个虚拟地址**上，
//!    但改子进程的不影响父进程
//! 3. **`waitpid` 能拿到子进程的退出码**

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{fork, getpid, waitpid, yield_now};

const PARENT_MARK: u64 = 0xF00D;
const CHILD_MARK: u64 = 0xBEEF;
const CHILD_EXIT: i32 = 42;

/// 一个全局变量。fork 之后父子各有一份，但**地址相同**。
static mut MARK: u64 = PARENT_MARK;

fn read_mark() -> u64 {
    // SAFETY: 单线程程序，这个全局只有自己会碰。
    unsafe { MARK }
}

#[no_mangle]
pub fn main() -> i32 {
    let addr = &raw const MARK as usize;
    println!("  [fork] 我是 pid {}，MARK @ {:#x} = {:#x}", getpid(), addr, read_mark());

    let pid = fork();

    if pid == 0 {
        // ---- 子进程 ----
        // SAFETY: 单线程程序。
        unsafe { MARK = CHILD_MARK };
        println!(
            "  [fork] 子进程 pid={}，fork 返回 {}，把 MARK @ {:#x} 改成 {:#x}",
            getpid(),
            pid,
            addr,
            read_mark()
        );
        yield_now(); // 让父进程有机会检查它自己的那一份
        println!("  [fork] 子进程再看一眼 MARK = {:#x}，准备以 {} 退出", read_mark(), CHILD_EXIT);
        return CHILD_EXIT;
    }

    // ---- 父进程 ----
    println!(
        "  [fork] 父进程 pid={}，fork 返回 {}（子进程的 PID）",
        getpid(),
        pid
    );
    yield_now(); // 给子进程改 MARK 的机会

    let mine = read_mark();
    println!("  [fork] 父进程的 MARK @ {:#x} 仍是 {:#x}", addr, mine);
    assert_eq!(mine, PARENT_MARK, "子进程改动串到父进程了，fork 没有真正复制");

    let mut exit_code = 0;
    let reaped = waitpid(pid, &mut exit_code);
    println!("  [fork] 父进程回收了 pid {}，退出码 {}", reaped, exit_code);
    assert_eq!(reaped, pid, "回收到的不是我 fork 出的那个");
    assert_eq!(exit_code, CHILD_EXIT, "退出码没传对");

    println!("  [fork] 全部断言通过：一次调用两次返回、副本独立、退出码可回收");
    0
}

//! 用户态世界的起点。
//!
//! 第 9 周之前，内核开机时把所有程序一股脑做成任务扔进调度器。
//! 现在内核**只创建这一个进程**，剩下的全部由它 `fork` + `exec` 出来 ——
//! 和真实的 Unix 一样。
//!
//! 它还承担另一个职责：**收养孤儿**。任何进程退出时，如果它还有活着的
//! 子进程，那些子进程会被过继到 initproc 名下，由它负责 `wait` 回收。
//! 不这么做的话，那些子进程退出后就成了没人收的僵尸，PID 和内核栈
//! 永远回收不掉。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{exec, fork, getpid, wait};

/// 要跑的程序。顺序就是 fork 的顺序，但它们之后是**并发**执行的。
const PROGRAMS: &[&str] = &[
    "00hello_world",
    "01yield_a",
    "02yield_b",
    "03yield_c",
    "04spin_x",
    "05spin_y",
    "06timer",
    "07bad_syscall_arg",
    "08bad_instruction",
    "09bad_address",
    "10bad_kernel_write",
    "11forktest",
];

#[no_mangle]
pub fn main() -> i32 {
    println!("[initproc] 我是 pid {}，整个用户态世界从我开始。", getpid());

    for name in PROGRAMS {
        let pid = fork();
        if pid == 0 {
            // ---- 子进程：换装成目标程序 ----
            exec(name);
            // exec 成功的话根本回不到这里
            println!("[initproc] exec \"{}\" 失败", name);
            return -1;
        }
        println!("[initproc] fork 出 pid {} 去跑 {}", pid, name);
    }

    // ---- 回收 ----
    //
    // wait 返回 -1 表示"已经没有子进程了"。注意这里也会收到那些
    // 被过继过来的孤儿 —— 比如 11forktest 如果没来得及 wait 自己的
    // 子进程就退出了，那个孙进程就会落到这里。
    let mut reaped = 0;
    loop {
        let mut exit_code = 0;
        let pid = wait(&mut exit_code);
        if pid < 0 {
            break;
        }
        reaped += 1;
        println!("[initproc] 回收子进程 {}，退出码 {}", pid, exit_code);
    }

    println!("[initproc] 共回收 {} 个子进程，收工。", reaped);
    0
}

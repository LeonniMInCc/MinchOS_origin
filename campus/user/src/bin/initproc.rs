//! 用户态世界的起点。
//!
//! 内核开机时**只创建这一个进程**，剩下的全部由它 `fork` + `exec` 出来 ——
//! 和真实的 Unix 一样。
//!
//! 第 10 周起它只做两件事：
//!
//! 1. 启动 shell
//! 2. **收养孤儿并回收僵尸**
//!
//! 第 2 件事容易被忽略但不可或缺。任何进程退出时，如果它还有活着的
//! 子进程，那些子进程会被过继到 initproc 名下。不这么做的话，
//! 它们退出后就成了没人收的僵尸，PID 和内核栈永远回收不掉。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{exec, fork, getpid, wait};

#[no_mangle]
pub fn main() -> i32 {
    println!("[initproc] 我是 pid {}，整个用户态世界从我开始。", getpid());

    let pid = fork();
    if pid == 0 {
        exec("user_shell");
        println!("[initproc] 启动 shell 失败");
        return -1;
    }
    println!("[initproc] shell 已启动，pid = {}", pid);

    // 回收循环。
    //
    // 这里收的不只是 shell：任何被过继过来的孤儿最终也会落到这里。
    // `wait` 返回 -1 表示"已经没有子进程了" —— 那时 shell 也退出了，
    // 我们跟着收工，内核会发现就绪队列空了然后关机。
    let mut reaped = 0;
    loop {
        let mut exit_code = 0;
        let child = wait(&mut exit_code);
        if child < 0 {
            break;
        }
        reaped += 1;
        println!("[initproc] 回收子进程 {}，退出码 {}", child, exit_code);
    }

    println!("[initproc] 共回收 {} 个子进程，收工。", reaped);
    0
}

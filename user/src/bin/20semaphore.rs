//! 用信号量强制执行顺序。
//!
//! 五个进程被 `fork` 出来之后，谁先跑完全看调度器心情。这个程序用一串
//! 信号量把它们串成一条链：第 i 个进程必须等第 i-1 个通知它才能动。
//!
//! 输出必然是 0,1,2,3,4 —— **连续跑很多次都不会变**。
//! 这正是同步原语的意义：把"碰运气"变成"有保证"。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{fork, wait, Semaphore};

const N: usize = 5;

#[no_mangle]
pub fn main() -> i32 {
    // sems[i] 表示"轮到第 i 个进程了"。全部初始为 0 —— 没有票，
    // 所以每个进程一上来就会阻塞，直到前一个把票递过来。
    let mut sems = [Semaphore::create(0); N + 1];
    for s in sems.iter_mut() {
        *s = Semaphore::create(0);
    }

    println!("五个进程会严格按 0→4 的顺序打印，无论调度器怎么排。");

    for i in 0..N {
        if fork() == 0 {
            sems[i].down(); // 等前一个通知我
            println!("  我是第 {} 个", i);
            sems[i + 1].up(); // 通知下一个
            return 0;
        }
    }

    // 发令：给第一个进程递票
    sems[0].up();
    // 最后一个进程会往 sems[N] 里放一张票，父进程在这里收掉
    sems[N].down();

    let mut done = 0;
    loop {
        let mut code = 0;
        if wait(&mut code) < 0 {
            break;
        }
        done += 1;
    }
    println!("顺序执行完毕（共 {} 个进程）。", done);
    0
}

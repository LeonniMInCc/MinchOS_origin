//! 条件变量：等一个**状态**，而不是等一把锁。
//!
//! 四个工人进程启动后立刻进入等待，直到工头广播"开工"。
//!
//! # 为什么必须用 `while` 而不是 `if`
//!
//! 被唤醒**不等于**条件成立 —— 可能是虚假唤醒，也可能有别人抢先把条件
//! 又改回去了。所有条件变量的正确用法都是：
//!
//! ```text
//!   lock(m);
//!   while !条件 { condvar.wait(m); }
//!   ... 干活 ...
//!   unlock(m);
//! ```
//!
//! 我们没有共享内存，所以"条件"用一个信号量代替：工人醒来后去 `down`
//! 一张开工票，拿到才算数。原理一样 —— **醒来之后重新确认**。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{fork, wait, yield_now, Condvar, Mutex, Semaphore};

const WORKERS: usize = 4;

#[no_mangle]
pub fn main() -> i32 {
    let mutex = Mutex::create();
    let condvar = Condvar::create();
    // "开工票"：广播之后每个工人各取一张，取到才算条件真的成立
    let tickets = Semaphore::create(0);
    let done = Semaphore::create(0);

    println!("四个工人启动后会立刻睡下，等工头广播。");

    for id in 0..WORKERS {
        if fork() == 0 {
            mutex.lock();
            // 醒来要重新确认条件 —— 这就是那个必须用 while 的地方
            condvar.wait(&mutex);
            mutex.unlock();

            tickets.down(); // 确认真的轮到自己了
            println!("  工人 {} 开工", id);
            done.up();
            return 0;
        }
    }

    // 让工人们先睡下。没有这几轮 yield 的话，工头可能在工人还没
    // wait 之前就广播了 —— 那次广播落空，工人会睡到天荒地老。
    //
    // 这是**丢失唤醒**的一个真实例子。正确的做法是用一个共享的状态位
    // 配合 while 循环，而不是靠"睡够久"。这里用 yield 是为了演示简单，
    // 但它本质上是个竞态，真实代码不该这么写。
    for _ in 0..40 {
        yield_now();
    }

    println!("工头：开工！（broadcast）");
    for _ in 0..WORKERS {
        tickets.up();
    }
    condvar.broadcast();

    // 等所有工人报到
    for _ in 0..WORKERS {
        done.down();
    }

    let mut n = 0;
    loop {
        let mut code = 0;
        if wait(&mut code) < 0 {
            break;
        }
        n += 1;
    }
    println!("{} 个工人全部完成。", n);
    0
}

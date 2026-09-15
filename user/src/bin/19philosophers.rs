//! 哲学家就餐 —— 互斥锁的经典场景。
//!
//! 五个哲学家围坐，每人左右各一只叉子，吃饭需要同时拿到两只。
//! 天真的做法（每人先拿左手边的）会**死锁**：五个人各拿一只左叉，
//! 然后一起等右叉，谁也等不到。
//!
//! 破解办法是**打破环**：给叉子编号，所有人都先拿编号小的那只。
//! 于是最后一位哲学家的取用顺序和别人相反，环就断了。
//!
//! # 这里的"哲学家"是进程，不是线程
//!
//! CCNUOS 没有线程，所以五个哲学家是五个 `fork` 出来的进程。
//! 它们共用的是**系统级的内核锁**，靠一个整数 ID 引用 ——
//! `fork` 之后子进程继承这个整数，就指向同一把锁。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{fork, wait, yield_now, Mutex};

const N: usize = 5;
const MEALS: usize = 3;

#[no_mangle]
pub fn main() -> i32 {
    // 五只叉子，外加一把"输出锁"
    let mut forks = [Mutex::create(); N];
    for f in forks.iter_mut() {
        *f = Mutex::create();
    }
    let print_lock = Mutex::create();

    println!("五位哲学家入座，每人要吃 {} 轮。", MEALS);

    for i in 0..N {
        if fork() == 0 {
            philosopher(i, &forks, &print_lock);
            return 0;
        }
    }

    let mut done = 0;
    loop {
        let mut code = 0;
        if wait(&mut code) < 0 {
            break;
        }
        done += 1;
    }

    println!("{} 位哲学家都吃完了，没有人饿死。", done);
    0
}

fn philosopher(id: usize, forks: &[Mutex; N], print_lock: &Mutex) {
    // **关键一步**：总是先拿编号小的叉子。
    // 这让最后一位哲学家的顺序和其他人相反，打破了循环等待。
    let left = id;
    let right = (id + 1) % N;
    let (first, second) = if left < right { (left, right) } else { (right, left) };

    for round in 0..MEALS {
        // 思考一会儿，让交错更明显
        for _ in 0..3 {
            yield_now();
        }

        forks[first].lock();
        forks[second].lock();

        // 输出也要加锁 —— 否则 println! 会被拆成好几次 sys_write，
        // 中间被抢占就会和别人的输出混在一起。
        // 第 8 章那个"输出半行交错"的现象，根子就在这里。
        print_lock.lock();
        println!("  哲学家 {} 第 {} 轮：拿到叉子 {} 和 {}，开吃", id, round, first, second);
        print_lock.unlock();

        for _ in 0..2 {
            yield_now();
        }

        forks[second].unlock();
        forks[first].unlock();
    }

    print_lock.lock();
    println!("  哲学家 {} 吃饱了，离席", id);
    print_lock.unlock();
}

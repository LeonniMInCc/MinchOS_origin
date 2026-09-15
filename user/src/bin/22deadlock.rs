//! **故意制造一次死锁**，看内核能不能发现。
//!
//! 就是哲学家就餐的天真版：每个人都先拿左手边的叉子。五个人各拿一只，
//! 然后一起等右边那只 —— 谁也等不到，谁也不肯放手。
//!
//! 和 `19philosophers` 的唯一区别是**取叉顺序**：那边所有人都先拿编号
//! 小的，于是最后一位的顺序和别人相反，环就断了。这边不做这个处理。
//!
//! 预期结果：内核报告死锁并以错误码关机。
//!
//! # 为什么内核必须能发现它
//!
//! 死锁之后，就绪队列会空掉。而"就绪队列空了"在第 9–13 周一直被当作
//! **"活干完了"** 来处理 —— 直接关机。也就是说，一次死锁会表现成
//! "系统正常退出"，这是最糟糕的失败方式：出了大事，却什么都没说。
//!
//! 所以内核要区分这两种情况：队列空了但还有进程阻塞着 = 死锁。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{fork, wait, yield_now, Mutex};

const N: usize = 5;

#[no_mangle]
pub fn main() -> i32 {
    let mut forks = [Mutex::create(); N];
    for f in forks.iter_mut() {
        *f = Mutex::create();
    }

    println!("五位哲学家，每人都先拿左手边的叉子 —— 这必然死锁。");
    println!("内核应该发现并报错，而不是当作'正常结束'关机。");

    for i in 0..N {
        if fork() == 0 {
            // 天真做法：先左后右，不做排序
            let left = i;
            let right = (i + 1) % N;

            forks[left].lock();
            println!("  哲学家 {} 拿到了左叉 {}", i, left);
            yield_now(); // 让所有人都先拿到左叉，把环坐实
            forks[right].lock(); // 这里所有人一起卡死

            println!("  哲学家 {} 居然吃上了？（不该出现）", i);
            forks[right].unlock();
            forks[left].unlock();
            return 0;
        }
    }

    let mut code = 0;
    while wait(&mut code) >= 0 {}
    println!("（不该走到这里）");
    0
}

//! 直接使用 `pipe()` 的例子 —— 不经过 shell。
//!
//! 演示管道最原始的用法：一个进程 fork 出另一个，父写子读。
//! shell 里的 `a | b` 无非是把这套动作做了两遍（或 n 遍）。
//!
//! 重点看**关掉不用的那一端**这件事：
//!
//! - 子进程（读）必须关掉写端，否则"写端全关了"永远不成立，读不到 EOF
//! - 父进程（写）必须关掉读端，不然没人来取时它会一直傻等
//!
//! 这是管道编程里最经典的 bug 来源。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{close, fork, pipe, read, wait, write};

/// 故意比管道缓冲区（32 字节）长。
const MESSAGE: &str = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOP";

#[no_mangle]
pub fn main() -> i32 {
    let mut fds = [0i32; 2];
    if pipe(&mut fds) < 0 {
        println!("  [管道] 创建失败");
        return -1;
    }
    let (read_fd, write_fd) = (fds[0] as usize, fds[1] as usize);
    println!("  [管道] 建好了：读端 fd={}，写端 fd={}", read_fd, write_fd);

    let pid = fork();

    if pid == 0 {
        // ---- 子进程：只读 ----
        // 关掉写端。不关的话，这个进程自己就持有一个写端，
        // "所有写端都关了"永远不成立，下面的循环会一直等下去。
        close(write_fd);

        let mut buf = [0u8; 16];
        let mut total = 0usize;
        loop {
            let n = read(read_fd, &mut buf);
            if n <= 0 {
                break; // 0 = EOF
            }
            let text = core::str::from_utf8(&buf[..n as usize]).unwrap_or("<非法UTF8>");
            println!("  [管道·子] 收到 {} 字节：{}", n, text);
            total += n as usize;
        }
        println!("  [管道·子] 读到 EOF，共 {} 字节", total);
        close(read_fd);
        return 0;
    }

    // ---- 父进程：只写 ----
    close(read_fd);

    // MESSAGE 比缓冲区长，所以这次 write 一定会经历
    // "写满 → 等子进程取走 → 继续写"的完整过程。
    // 缓冲区故意只开 32 字节，就是为了让这条路径在日常使用中被真实走到。
    println!(
        "  [管道·父] 写入 {} 字节（管道缓冲区只有 32 字节）",
        MESSAGE.len()
    );
    let written = write(write_fd, MESSAGE.as_bytes());
    println!("  [管道·父] 写完了，返回 {}", written);

    // 关掉写端，子进程才能读到 EOF
    close(write_fd);

    let mut exit_code = 0;
    let child = wait(&mut exit_code);
    println!("  [管道·父] 回收了子进程 {}，退出码 {}", child, exit_code);

    assert_eq!(written, MESSAGE.len() as isize, "写出的字节数不对");
    println!("  [管道] 测试通过：跨进程传输 {} 字节，跨越了缓冲区容量", written);
    0
}

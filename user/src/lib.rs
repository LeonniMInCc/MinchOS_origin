//! CCNUOS 用户态运行库。
//!
//! 用户程序也是 `#![no_std]` 的 —— 标准库依赖一个成熟的操作系统，
//! 而 CCNUOS 现在只提供两个系统调用。这个 crate 就是最小的"libc"：
//! 负责程序启动、清 `.bss`、提供 `println!`、以及在 `main` 返回后调 `exit`。
//!
//! # 一个用户程序长什么样
//!
//! ```rust
//! #![no_std]
//! #![no_main]
//!
//! #[macro_use]
//! extern crate user_lib;
//!
//! #[no_mangle]
//! pub fn main() -> i32 {
//!     println!("你好");
//!     0
//! }
//! ```

#![no_std]

#[macro_use]
pub mod console;
mod lang_items;
pub mod syscall;

pub use syscall::{
    sys_close, sys_exec, sys_exit, sys_fork, sys_get_time, sys_getpid, sys_read, sys_waitpid,
    sys_write, sys_write_raw, sys_yield,
};

/// 标准输入的文件描述符。
pub const STDIN: usize = 0;
/// 标准输出的文件描述符。
pub const STDOUT: usize = 1;

/// 从标准输入读一个字节。会阻塞到真的有输入为止。
pub fn getchar() -> u8 {
    let mut buf = [0u8; 1];
    loop {
        match sys_read(STDIN, &mut buf) {
            1 => return buf[0],
            // 读失败（比如 fd 被关了）。返回 0 让调用方自己判断。
            n if n < 0 => return 0,
            _ => yield_now(),
        }
    }
}

/// 关闭一个文件描述符。
pub fn close(fd: usize) -> isize {
    sys_close(fd)
}

/// 主动让出 CPU，让别的程序跑一会儿。
pub fn yield_now() {
    sys_yield();
}

/// 开机以来的毫秒数。
pub fn get_time_ms() -> usize {
    sys_get_time() as usize
}

/// 当前进程的 PID。
pub fn getpid() -> isize {
    sys_getpid()
}

/// 复制出一个子进程。
///
/// **一次调用，两次返回**：父进程拿到子进程 PID，子进程拿到 0。
pub fn fork() -> isize {
    sys_fork()
}

/// 用另一个程序替换当前进程。
///
/// `name` 不需要自带 `\0` —— 这里负责补上。内核按 `\0` 找字符串结尾，
/// 直接把 Rust 的 `&str` 传过去的话，内核会一路读到下一个恰好是 0 的
/// 字节为止，读出一堆垃圾。
pub fn exec(name: &str) -> isize {
    let mut buf = [0u8; 128];
    if name.len() >= buf.len() {
        return -1;
    }
    buf[..name.len()].copy_from_slice(name.as_bytes());
    // buf 其余部分是 0，天然就是结尾符
    sys_exec(unsafe { core::str::from_utf8_unchecked(&buf[..=name.len()]) })
}

/// 等一个已退出的子进程，返回它的 PID。
///
/// `pid < 0` 表示"等任意一个"。没有子进程时返回 -1。
///
/// 内核的 `sys_waitpid` 在"子进程还活着"时返回 -2 而不是阻塞 ——
/// 因为还没有等待队列（那是第 14 章的事）。这里就自己 yield 重试，
/// 语义一样，只是浪费些 CPU。
pub fn waitpid(pid: isize, exit_code: &mut i32) -> isize {
    loop {
        match sys_waitpid(pid, exit_code as *mut i32) {
            -2 => yield_now(),
            n => return n,
        }
    }
}

/// 等任意一个子进程。
pub fn wait(exit_code: &mut i32) -> isize {
    waitpid(-1, exit_code)
}

/// 用户程序的真正入口。
///
/// 内核用 `sret` 跳到 `APP_BASE_ADDRESS`，而链接脚本保证了
/// `.text.entry`（也就是这个函数）就在那个地址上。
///
/// 此刻我们拥有：一个 U 态的 CPU，和内核在 `sp` 里放好的用户栈。
/// 仅此而已。
#[no_mangle]
#[link_section = ".text.entry"]
pub extern "C" fn _start() -> ! {
    clear_bss();
    // SAFETY: main 由具体的用户程序用 #[no_mangle] 定义。
    // 忘了定义的话是链接错误，不会跑到这里。
    let exit_code = unsafe { main() };
    sys_exit(exit_code)
}

extern "Rust" {
    /// 由具体的用户程序提供。
    fn main() -> i32;
}

/// 清零 `.bss`。
///
/// 和内核里做的是同一件事、同样的理由：没有 C 运行时替我们兑现
/// "未初始化的全局变量是 0"这个假定。
///
/// 区别在于内核的 `.bss` 由它自己清，而用户程序的 `.bss` 位于
/// 加载区内 —— 内核装载时已经把整个加载区清零过一遍了。
/// 这里再清一次是为了不依赖内核的实现细节：**用户程序不该假设
/// 内核替它做了什么**。
fn clear_bss() {
    extern "C" {
        static start_bss: u8;
        static end_bss: u8;
    }
    let (start, end) = (&raw const start_bss as usize, &raw const end_bss as usize);
    (start..end).for_each(|addr| unsafe {
        (addr as *mut u8).write_volatile(0);
    });
}

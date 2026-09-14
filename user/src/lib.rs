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

pub use syscall::{sys_exit, sys_write, sys_write_raw};

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

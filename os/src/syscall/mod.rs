//! 系统调用分派。
//!
//! 系统调用是用户程序**唯一**能请内核办事的途径。用户程序执行 `ecall`，
//! CPU 从 U 态陷入 S 态，内核看一眼请求的是什么，办完再返回。
//!
//! # 调用约定
//!
//! 我们沿用 Linux 在 RISC-V 上的约定，这样以后想跑真实的用户程序会容易些：
//!
//! | 寄存器 | 用途           |
//! |--------|----------------|
//! | `a7`   | 系统调用号     |
//! | `a0`   | 第 1 个参数 / 返回值 |
//! | `a1`   | 第 2 个参数    |
//! | `a2`   | 第 3 个参数    |
//!
//! 调用号也和 Linux 保持一致（`write` = 64，`exit` = 93），
//! 都取自 `asm-generic/unistd.h`。

mod fs;
mod process;

use fs::sys_write;
use process::{sys_exit, sys_yield};

const SYSCALL_WRITE: usize = 64;
const SYSCALL_EXIT: usize = 93;
const SYSCALL_YIELD: usize = 124;

/// 分派一次系统调用。由 [`crate::trap::trap_handler`] 调用。
pub fn syscall(id: usize, args: [usize; 3]) -> isize {
    match id {
        SYSCALL_WRITE => sys_write(args[0], args[1] as *const u8, args[2]),
        SYSCALL_EXIT => sys_exit(args[0] as i32),
        SYSCALL_YIELD => sys_yield(),
        _ => {
            // 用户程序请求了一个不存在的服务。
            // 这不该让内核崩溃 —— 返回错误码，让用户程序自己处理。
            error!("未知的系统调用号 {}，参数 {:?}", id, args);
            -1
        }
    }
}

//! 系统调用的用户侧封装。
//!
//! 这是用户程序与内核之间的全部接口。每个函数都只做一件事：
//! 按约定把参数放进寄存器，执行 `ecall`，然后取回返回值。
//!
//! 对照 `os/src/syscall/mod.rs` 看 —— 那边是同一个约定的内核侧。

use core::arch::asm;

const SYSCALL_WRITE: usize = 64;
const SYSCALL_EXIT: usize = 93;

/// 执行一次系统调用。
///
/// `ecall` 会触发一次同步异常，把 CPU 从 U 态切到 S 态，
/// 跳到内核的 `stvec`（也就是 `__alltraps`）。内核处理完之后
/// `sret` 回来，程序从 `ecall` 的下一条指令继续。
///
/// 注意 `a0` 用了 `inlateout`：它既是第一个参数，又是返回值。
fn syscall(id: usize, args: [usize; 3]) -> isize {
    let mut ret: isize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") args[0] => ret,
            in("a1") args[1],
            in("a2") args[2],
            in("a7") id,
        );
    }
    ret
}

/// 把缓冲区写到文件描述符 `fd`。返回写出的字节数，失败返回负数。
pub fn sys_write(fd: usize, buffer: &[u8]) -> isize {
    syscall(
        SYSCALL_WRITE,
        [fd, buffer.as_ptr() as usize, buffer.len()],
    )
}

/// 裸的 write，可以传任意地址 —— 专门用来测试内核的参数校验。
pub fn sys_write_raw(fd: usize, ptr: usize, len: usize) -> isize {
    syscall(SYSCALL_WRITE, [fd, ptr, len])
}

/// 退出当前程序。永不返回。
pub fn sys_exit(exit_code: i32) -> ! {
    syscall(SYSCALL_EXIT, [exit_code as usize, 0, 0]);
    unreachable!("sys_exit 之后不该还在运行")
}

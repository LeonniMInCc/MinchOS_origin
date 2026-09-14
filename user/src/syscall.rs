//! 系统调用的用户侧封装。
//!
//! 这是用户程序与内核之间的全部接口。每个函数都只做一件事：
//! 按约定把参数放进寄存器，执行 `ecall`，然后取回返回值。
//!
//! 对照 `os/src/syscall/mod.rs` 看 —— 那边是同一个约定的内核侧。

use core::arch::asm;

const SYSCALL_WRITE: usize = 64;
const SYSCALL_EXIT: usize = 93;
const SYSCALL_YIELD: usize = 124;
const SYSCALL_GET_TIME: usize = 169;
const SYSCALL_GETPID: usize = 172;
const SYSCALL_FORK: usize = 220;
const SYSCALL_EXEC: usize = 221;
const SYSCALL_WAITPID: usize = 260;

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

/// 主动让出 CPU。
///
/// 这个调用**会返回** —— 但返回的时刻是"内核再次把 CPU 分给我"。
/// 从程序自己的视角看，这一行执行得有点慢，仅此而已；
/// 实际上中间可能已经跑过了好几个别的程序。
pub fn sys_yield() -> isize {
    syscall(SYSCALL_YIELD, [0, 0, 0])
}

/// 返回开机以来的毫秒数。
///
/// 用户程序读不到 `time` CSR —— 那是 S 态寄存器，U 态执行 `csrr`
/// 会触发非法指令异常。时间也得向内核要。
pub fn sys_get_time() -> isize {
    syscall(SYSCALL_GET_TIME, [0, 0, 0])
}

/// 当前进程的 PID。
pub fn sys_getpid() -> isize {
    syscall(SYSCALL_GETPID, [0, 0, 0])
}

/// 复制出一个子进程。父进程拿到子进程 PID，子进程拿到 0。
pub fn sys_fork() -> isize {
    syscall(SYSCALL_FORK, [0, 0, 0])
}

/// 用另一个程序替换当前进程。成功则不返回。
pub fn sys_exec(path: &str) -> isize {
    // 内核按 \0 找字符串结尾，所以传进去的必须是 C 风格字符串。
    // 调用方要保证 path 以 \0 结尾 —— 见 user_lib::exec。
    syscall(SYSCALL_EXEC, [path.as_ptr() as usize, 0, 0])
}

/// 回收一个已退出的子进程。
///
/// 返回 -1 表示没有这个子进程，-2 表示它还没退出。
pub fn sys_waitpid(pid: isize, exit_code: *mut i32) -> isize {
    syscall(SYSCALL_WAITPID, [pid as usize, exit_code as usize, 0])
}

/// 退出当前程序。永不返回。
pub fn sys_exit(exit_code: i32) -> ! {
    syscall(SYSCALL_EXIT, [exit_code as usize, 0, 0]);
    unreachable!("sys_exit 之后不该还在运行")
}

//! 进程相关的系统调用。

use crate::fs::read_file;
use crate::mm::{translated_refmut, translated_str};
use crate::task::{
    current_pid, current_user_token, exec_current, exit_current_and_run_next, fork_current,
    suspend_current_and_run_next, wait_child,
};
use crate::timer::get_time_ms;

/// 程序名的长度上限。防的是用户传来一段没有 `\0` 的内存。
const MAX_PATH_LEN: usize = 128;

/// `exit(code)` —— 当前进程退出。永不返回。
///
/// 进程不会立刻消失，而是变成僵尸等父进程 `waitpid`。
/// 用户内存会在这里就归还 —— 死掉的进程没理由继续占着物理内存。
pub fn sys_exit(exit_code: i32) -> ! {
    if exit_code == 0 {
        info!("进程 {} 正常退出，退出码 {}", current_pid(), exit_code);
    } else {
        warn!("进程 {} 退出，退出码 {}", current_pid(), exit_code);
    }
    exit_current_and_run_next(exit_code)
}

/// `yield()` —— 主动让出 CPU。
pub fn sys_yield() -> isize {
    suspend_current_and_run_next();
    0
}

/// `get_time()` —— 开机以来的毫秒数。
pub fn sys_get_time() -> isize {
    get_time_ms() as isize
}

/// `getpid()` —— 当前进程的 PID。
pub fn sys_getpid() -> isize {
    current_pid() as isize
}

/// `fork()` —— 复制出一个子进程。
///
/// **一次调用，两次返回**：
///
/// - 父进程拿到子进程的 PID
/// - 子进程拿到 0
///
/// 两边从同一条指令（`ecall` 的下一条）继续执行，唯一的区别就是这个返回值。
/// 实现上，子进程那个 0 是 [`fork_current`] 直接写进子进程 `TrapContext`
/// 的 `a0` 里的 —— 它压根没有"执行完一次系统调用"，只是被布置成了
/// 仿佛刚执行完的样子。
pub fn sys_fork() -> isize {
    fork_current() as isize
}

/// `exec(path)` —— 用另一个程序替换当前进程。
///
/// 成功时**不返回**（返回的话就回到旧程序里了）—— 准确地说，它会返回到
/// `trap_return`，然后 `sret` 到新程序的入口。失败返回 -1。
///
/// 注意 PID、父子关系、内核栈都不变。变的只是"这个进程在执行什么"。
pub fn sys_exec(path_ptr: usize) -> isize {
    let token = current_user_token();
    let Some(path) = translated_str(token, path_ptr, MAX_PATH_LEN) else {
        error!("sys_exec：进程 {} 传来了不可读的路径指针", current_pid());
        return -1;
    };

    let Some(data) = read_file(&path) else {
        error!("sys_exec：找不到程序 \"{}\"", path);
        return -1;
    };

    debug!("进程 {} exec 成 \"{}\"", current_pid(), path);
    exec_current(&data);

    // 这个返回值不会被用户程序看到：exec 已经换掉了整个地址空间，
    // 新的 TrapContext 里 a0 是 0。写在这里只是为了让函数签名完整。
    0
}

/// `waitpid(pid, exit_code_ptr)` —— 回收一个已退出的子进程。
///
/// - `pid < 0` 表示"等任意一个子进程"
/// - 返回被回收的子进程 PID
/// - 返回 **-1**：根本没有这个子进程
/// - 返回 **-2**：有，但还没退出（调用方应该 yield 之后重试）
///
/// 为什么不直接在内核里阻塞等待？因为还没有等待队列 —— 那是第 14 章的事。
/// 现在让用户库循环 `yield` 重试，语义一样，只是浪费些 CPU。
pub fn sys_waitpid(pid: isize, exit_code_ptr: usize) -> isize {
    match wait_child(pid) {
        Err(()) => -1,
        Ok(None) => -2,
        Ok(Some((child_pid, exit_code))) => {
            // 用户可能传 0 表示"不关心退出码"
            if exit_code_ptr != 0 {
                let token = current_user_token();
                match translated_refmut::<i32>(token, exit_code_ptr) {
                    Some(slot) => *slot = exit_code,
                    None => {
                        error!(
                            "sys_waitpid：进程 {} 传来了不可写的退出码指针 {:#x}",
                            current_pid(),
                            exit_code_ptr
                        );
                        return -1;
                    }
                }
            }
            debug!("进程 {} 回收了子进程 {}（退出码 {}）", current_pid(), child_pid, exit_code);
            child_pid as isize
        }
    }
}

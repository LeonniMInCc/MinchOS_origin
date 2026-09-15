//! 文件相关的系统调用。
//!
//! 第 10 周起，`read` / `write` 不再直接操作串口，而是通过**文件描述符表**
//! 分派给一个 [`File`] 实现。第 11 周的管道和第 13 周的磁盘文件
//! 只要多写一个实现，这里一行都不用改。

use crate::fs::File;
use crate::mm::{translated_byte_buffer, UserBuffer};
use crate::task::{current_pid, current_task, current_user_token};
use alloc::sync::Arc;

/// 取出 fd 对应的文件对象。
///
/// # 为什么要单独抽出来，而且必须返回 `Arc` 而不是引用
///
/// 这是本章最容易踩的坑。看下面这段**错误**的写法：
///
/// ```ignore
/// let inner = task.inner_exclusive_access();
/// let file = inner.fd_table[fd].as_ref().unwrap();
/// file.read(buf)   // ← inner 还借着！
/// ```
///
/// `Stdin::read` 在没有输入时会调 `suspend_current_and_run_next()`，
/// 而那个函数要访问同一个进程控制块。于是 `RefCell` 发现重复借用，
/// 直接 panic。
///
/// 所以必须**先把 `Arc` 克隆出来、释放借用，再去 read**。
/// 一个"可能让出 CPU 的调用"绝不能握着锁进行 —— 这是所有内核里
/// 都成立的规矩，只是在别的语言里它不会当场把你拦下来。
fn fd_to_file(fd: usize) -> Option<Arc<dyn File>> {
    let task = current_task()?;
    let inner = task.inner_exclusive_access();
    inner.fd_table.get(fd)?.clone()
    // inner 在这里析构，借用释放
}

/// `write(fd, buf, len)`。
pub fn sys_write(fd: usize, buf: usize, len: usize) -> isize {
    let Some(file) = fd_to_file(fd) else {
        error!("sys_write：进程 {} 用了无效的 fd {}", current_pid(), fd);
        return -1;
    };
    if !file.writable() {
        error!("sys_write：fd {} 不可写", fd);
        return -1;
    }

    let token = current_user_token();
    let Some(buffers) = translated_byte_buffer(token, buf, len) else {
        error!(
            "sys_write：进程 {} 传来了不可访问的缓冲区 ptr={:#x} len={}，拒绝",
            current_pid(),
            buf,
            len
        );
        return -1;
    };

    file.write(UserBuffer::new(buffers))
        .map(|n| n as isize)
        .unwrap_or(-1)
}

/// `read(fd, buf, len)`。
///
/// 从标准输入读时会**阻塞到真的有输入为止**（见 [`crate::fs::Stdin::read`]），
/// 而且一次只返回一个字符 —— 串口是裸设备，没有行缓冲。
pub fn sys_read(fd: usize, buf: usize, len: usize) -> isize {
    let Some(file) = fd_to_file(fd) else {
        error!("sys_read：进程 {} 用了无效的 fd {}", current_pid(), fd);
        return -1;
    };
    if !file.readable() {
        error!("sys_read：fd {} 不可读", fd);
        return -1;
    }

    let token = current_user_token();
    let Some(buffers) = translated_byte_buffer(token, buf, len) else {
        error!(
            "sys_read：进程 {} 传来了不可访问的缓冲区 ptr={:#x} len={}，拒绝",
            current_pid(),
            buf,
            len
        );
        return -1;
    };

    file.read(UserBuffer::new(buffers))
        .map(|n| n as isize)
        .unwrap_or(-1)
}

/// `close(fd)` —— 关闭一个文件描述符。
///
/// 只是把表里的槽位置空。底下的文件对象由 `Arc` 的引用计数管着 ——
/// 所有持有它的进程都关掉之后才真正销毁。第 11 周的管道靠这个
/// 来判断"写端全关了，读端该看到 EOF 了"。
pub fn sys_close(fd: usize) -> isize {
    let Some(task) = current_task() else {
        return -1;
    };
    let mut inner = task.inner_exclusive_access();

    match inner.fd_table.get_mut(fd) {
        Some(slot @ Some(_)) => {
            *slot = None;
            0
        }
        _ => {
            error!("sys_close：进程 {} 关了一个无效的 fd {}", task.pid(), fd);
            -1
        }
    }
}

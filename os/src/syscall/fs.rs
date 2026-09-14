//! 文件相关的系统调用。
//!
//! 第 3 周只有 `write`，而且只认标准输出。第 10 周会把它扩展成
//! 真正的文件描述符表，第 13 周接上文件系统。

use crate::loader::is_accessible_by;
use crate::task::current_task_id;

/// 标准输出的文件描述符。
const FD_STDOUT: usize = 1;

/// `write(fd, buf, len)` —— 把用户缓冲区的内容写到文件描述符。
///
/// # 内核不能相信用户给的指针
///
/// `buf` 是用户程序传进来的地址。用户程序可能出于恶意或 bug，传一个
/// 指向内核数据的地址过来 —— 如果内核照单全收地读，就等于把内核内存
/// 泄露给了用户程序。
///
/// 所以这里必须检查范围。这是**内核代码和普通代码的根本区别**：
/// 跨越信任边界的每一个输入都要验。
///
/// 顺带一提：本章还没有分页，所以这个软件检查是唯一的防线。
/// 第 8 周建立地址空间隔离之后，硬件会帮我们兜住大部分情况 ——
/// 但这个检查依然要留着，因为用户程序完全可以传一个"在自己地址空间内、
/// 但不是它本意"的指针。
pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    if fd != FD_STDOUT {
        error!("sys_write：暂时只支持 fd=1 (stdout)，收到 fd={}", fd);
        return -1;
    }

    // 检查是针对**当前任务**做的：现在内存里同时躺着好几个程序，
    // 所以这个检查除了挡住"读内核内存"，还顺带挡住了"读别的程序的槽位"。
    let app_id = current_task_id();
    if !is_accessible_by(app_id, buf as usize, len) {
        // 注意这里打印的是 ptr 和 len 而不是 [start, end)：
        // 用户完全可以传一个让 start + len 溢出的参数（这正是下面
        // 02bad_syscall_arg 干的事），而在 debug 构建下整数溢出会 panic ——
        // 内核就被一条日志搞崩了。**错误处理路径本身也在信任边界上。**
        error!(
            "sys_write：任务 {} 传来了越界的缓冲区 ptr={:#x} len={}，拒绝",
            app_id,
            buf as usize,
            len
        );
        return -1;
    }

    // SAFETY: 上面已经验证过 [buf, buf+len) 完全落在当前用户程序
    // 可访问的区域内。
    let slice = unsafe { core::slice::from_raw_parts(buf, len) };

    // 用户程序送来的是任意字节，不保证是合法 UTF-8。
    // 非法字节用 U+FFFD 替换，而不是让内核 panic ——
    // 用户程序永远不该有能力搞崩内核。
    for chunk in slice.utf8_chunks() {
        print!("{}", chunk.valid());
        if !chunk.invalid().is_empty() {
            print!("\u{FFFD}");
        }
    }

    len as isize
}

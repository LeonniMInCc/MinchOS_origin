//! 文件相关的系统调用。
//!
//! 第 8 周只有 `write`，而且只认标准输出。第 10 周会把它扩展成
//! 真正的文件描述符表，第 13 周接上文件系统。

use crate::mm::translated_byte_buffer;
use crate::task::{current_task_id, current_user_token};
use alloc::vec::Vec;

/// 标准输出的文件描述符。
const FD_STDOUT: usize = 1;

/// `write(fd, buf, len)` —— 把用户缓冲区的内容写到文件描述符。
///
/// # 第 8 周：内核再也不能直接解引用用户指针了
///
/// 之前几章内核和用户共用一个地址空间，`buf` 拿来就能用（代价是必须
/// 打开 `sstatus.SUM`），安全性靠一个手写的区间检查 `is_accessible_by`。
///
/// 现在两者的地址空间彻底分开：用户传来的 `0x10234` 在内核页表里
/// **根本不存在**，直接解引用只会缺页。所以必须
/// [`translated_byte_buffer`] 手动走一遍用户的页表。
///
/// 这个改变顺带解决了安全问题，而且解得更干净：
///
/// - 以前要问"这个地址落在我给它划的那几个区间里吗"，
///   得手写范围算术、还得记着防整数溢出
/// - 现在只要问"**用户自己的页表里，这页带 `U` 位吗**"
///
/// 后者不需要知道任何区间常量，而且天然覆盖了所有情况 ——
/// 用户传内核地址？那页在它的页表里不存在。传跳板页？那页没有 `U`。
/// **把权限问题交还给权限系统本身。**
pub fn sys_write(fd: usize, buf: usize, len: usize) -> isize {
    if fd != FD_STDOUT {
        error!("sys_write：暂时只支持 fd=1 (stdout)，收到 fd={}", fd);
        return -1;
    }

    let token = current_user_token();
    let Some(buffers) = translated_byte_buffer(token, buf, len) else {
        error!(
            "sys_write：任务 {} 传来了不可访问的缓冲区 ptr={:#x} len={}，拒绝",
            current_task_id(),
            buf,
            len
        );
        return -1;
    };

    // 用户的缓冲区在它自己的地址空间里是连续的，但对应的物理页散落各处，
    // 所以 translated_byte_buffer 返回的是按页切开的若干片段。
    // 先拼回一整块再解码 —— 否则一个跨页的多字节 UTF-8 字符会被拆成两半。
    let mut bytes = Vec::with_capacity(len);
    for buffer in buffers {
        bytes.extend_from_slice(buffer);
    }

    // 用户程序送来的是任意字节，不保证是合法 UTF-8。
    // 非法字节用 U+FFFD 替换，而不是让内核 panic ——
    // 用户程序永远不该有能力搞崩内核。
    for chunk in bytes.utf8_chunks() {
        print!("{}", chunk.valid());
        if !chunk.invalid().is_empty() {
            print!("\u{FFFD}");
        }
    }

    len as isize
}

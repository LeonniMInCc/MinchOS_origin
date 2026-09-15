//! 文件抽象。
//!
//! # 为什么现在就要抽象
//!
//! 第 10 周只有两种"文件"：标准输入和标准输出，都是串口。为它们定义一个
//! trait 看起来像是过度设计 —— 直接在 `sys_read` / `sys_write` 里
//! `if fd == 0 {...} else if fd == 1 {...}` 也能跑。
//!
//! 但这个抽象是后面几章的地基：
//!
//! - 第 11 周：管道也是"文件"，`ls | wc` 靠的就是把 `stdout` 换成管道
//! - 第 13 周：磁盘上的真实文件
//!
//! 有了 trait，那两章只要多写一个实现，`sys_read` / `sys_write`
//! 一行都不用改。这就是 Unix "一切皆文件"那句话的技术含义。

mod stdio;

use crate::mm::UserBuffer;

pub use stdio::{Stdin, Stdout};

/// 任何可以被读写的东西。
///
/// `Send + Sync` 是因为它要放进 `Arc` 里、在 `fork` 时被多个进程共享。
/// 本项目是单核不抢占内核，这两个约束目前只是形式要求。
pub trait File: Send + Sync {
    fn readable(&self) -> bool;
    fn writable(&self) -> bool;

    /// 读入数据，返回读到的字节数。返回 `None` 表示出错。
    ///
    /// 注意 `&self` 而不是 `&mut self` —— 一个打开的文件可能被
    /// 好几个进程同时持有（`fork` 会复制文件描述符表，但共享底下的
    /// 文件对象），所以内部可变性要由实现自己处理。
    fn read(&self, buf: UserBuffer) -> Option<usize>;

    /// 写出数据，返回写出的字节数。
    fn write(&self, buf: UserBuffer) -> Option<usize>;
}

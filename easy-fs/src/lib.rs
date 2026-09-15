//! CCNUOS 的简易文件系统。
//!
//! # 为什么是一个独立的 crate
//!
//! 文件系统是这个项目里**唯一一块可以脱离内核单独测试**的代码：
//! 它的全部外部依赖就是"能读写编号为 N 的 512 字节块"这一件事。
//! 把这件事抽成 [`BlockDevice`] trait 之后：
//!
//! - **在内核里**，它的实现是 virtio-blk 驱动
//! - **在宿主机上**，它的实现可以是一个普通文件
//!
//! 于是整个文件系统可以用 `cargo test` 在宿主机上跑 —— 秒级反馈，
//! 有真正的断言失败信息，还能用 `println!` 调试。
//! 相比之下，每改一行就重新编译内核、启动 QEMU、肉眼看串口输出，
//! 效率差一个数量级。
//!
//! **能不进 QEMU 就别进。** 这是第 1 周排计划时就写下的一条，
//! 到这一章兑现。
//!
//! # 磁盘布局
//!
//! ```text
//!   块 0          超级块      魔数、各区大小
//!   块 1..        索引节点位图  哪些 inode 被用了
//!   ...           索引节点区   inode 本体
//!   ...           数据块位图   哪些数据块被用了
//!   ...           数据块区    文件内容
//! ```
//!
//! 位图放在各区前面，是为了让"分配一个 inode/数据块"变成一次位扫描。

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod bitmap;
mod block_cache;
mod block_dev;
mod efs;
mod layout;
mod mutex;
mod vfs;

pub use bitmap::Bitmap;
pub use block_cache::{block_cache_stats, clear_block_cache, get_block_cache, sync_all_blocks};
pub use block_dev::BlockDevice;
pub use efs::EasyFileSystem;
pub use layout::{
    read_super_block, DirEntry, DiskInode, DiskInodeType, SuperBlock, EFS_MAGIC, MAX_FILE_SIZE,
    NAME_LENGTH_LIMIT,
};
pub use mutex::Mutex;
pub use vfs::Inode;

/// 一个块的大小：512 字节。
///
/// 和绝大多数磁盘的扇区大小一致，也是 virtio-blk 请求的单位。
pub const BLOCK_SZ: usize = 512;

/// 每个块能放多少个位（给位图用）。
pub const BITS_PER_BLOCK: usize = BLOCK_SZ * 8;

#[cfg(test)]
mod tests;

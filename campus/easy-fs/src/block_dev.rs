//! 块设备抽象。

use core::any::Any;

/// 任何能按块号读写 512 字节的东西。
///
/// 这个 trait 是整个文件系统与外界的**唯一**接口。它只有两个方法，
/// 而且刻意不提供"异步"或"批量"版本 —— 简单到内核和宿主机都能轻松实现。
///
/// - 内核里：virtio-blk 驱动
/// - 宿主机测试里：一个普通文件，或者一段内存
///
/// `Any` 是为了让持有者能在需要时向下转型（本项目暂时用不到，
/// 但它是这类 trait 的惯例）。
pub trait BlockDevice: Send + Sync + Any {
    /// 把第 `block_id` 块读进 `buf`。`buf` 必须正好 [`crate::BLOCK_SZ`] 字节。
    fn read_block(&self, block_id: usize, buf: &mut [u8]);

    /// 把 `buf` 写到第 `block_id` 块。
    fn write_block(&self, block_id: usize, buf: &[u8]);
}

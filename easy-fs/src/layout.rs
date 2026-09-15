//! 磁盘上的数据结构。

use crate::block_cache::get_block_cache;
use crate::block_dev::BlockDevice;
use alloc::sync::Arc;

/// 超级块的魔数：`"CCNU"` 的 ASCII。
///
/// 挂载时第一件事就是核对它。核对魔数不是形式主义 —— 挂载一个格式不对的
/// 镜像，后果是内核按错误的布局去解释一堆随机字节，崩在离起因十万八千里
/// 的地方。**在边界上验，比在深处崩强。**
pub const EFS_MAGIC: u32 = 0x4343_4E55;

/// 块 0：整个文件系统的元信息。
///
/// `#[repr(C)]` 是必须的：这个结构体会被原样写进磁盘、又被原样读回来，
/// 字段顺序必须是确定的。Rust 默认的 `repr(Rust)` 允许编译器重排字段 ——
/// 那样换个编译器版本就读不出来了。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SuperBlock {
    magic: u32,
    pub total_blocks: u32,
    pub inode_bitmap_blocks: u32,
    pub inode_area_blocks: u32,
    pub data_bitmap_blocks: u32,
    pub data_area_blocks: u32,
}

impl SuperBlock {
    /// 写一个全新的超级块。格式化时用。
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        &mut self,
        total_blocks: u32,
        inode_bitmap_blocks: u32,
        inode_area_blocks: u32,
        data_bitmap_blocks: u32,
        data_area_blocks: u32,
    ) {
        *self = Self {
            magic: EFS_MAGIC,
            total_blocks,
            inode_bitmap_blocks,
            inode_area_blocks,
            data_bitmap_blocks,
            data_area_blocks,
        };
    }

    /// 魔数对不对。
    pub fn is_valid(&self) -> bool {
        self.magic == EFS_MAGIC
    }

    pub fn magic(&self) -> u32 {
        self.magic
    }

    /// 索引节点区从哪一块开始。
    pub fn inode_area_start(&self) -> u32 {
        1 + self.inode_bitmap_blocks
    }

    /// 数据块位图从哪一块开始。
    pub fn data_bitmap_start(&self) -> u32 {
        self.inode_area_start() + self.inode_area_blocks
    }

    /// 数据块区从哪一块开始。
    pub fn data_area_start(&self) -> u32 {
        self.data_bitmap_start() + self.data_bitmap_blocks
    }
}

/// 读出并校验超级块。挂载时的第一步。
pub fn read_super_block(device: &Arc<dyn BlockDevice>) -> Option<SuperBlock> {
    get_block_cache(0, Arc::clone(device))
        .lock()
        .read(0, |sb: &SuperBlock| {
            if sb.is_valid() {
                Some(*sb)
            } else {
                None
            }
        })
}

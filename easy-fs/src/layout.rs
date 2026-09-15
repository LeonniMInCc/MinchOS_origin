//! 磁盘上的数据结构。

use crate::block_cache::get_block_cache;
use crate::block_dev::BlockDevice;
use crate::BLOCK_SZ;
use alloc::sync::Arc;
use alloc::vec::Vec;

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

// ---------------------------------------------------------------------------
// 索引节点
// ---------------------------------------------------------------------------

/// 一个 inode 里直接记录的数据块数。
pub const INODE_DIRECT_COUNT: usize = 27;
/// 一个间接块能记录多少个数据块号（512 / 4）。
pub const INODE_INDIRECT1_COUNT: usize = BLOCK_SZ / 4;
/// 单个文件的大小上限。
pub const MAX_FILE_SIZE: usize = (INODE_DIRECT_COUNT + INODE_INDIRECT1_COUNT) * BLOCK_SZ;

/// 一个 inode 在磁盘上占的字节数。
///
/// 刻意凑成 128：这样一个 512 字节的块正好放 4 个，
/// 不会出现"一个 inode 横跨两个块"这种恶心情况。
pub const DISK_INODE_SIZE: usize = 128;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
#[repr(u32)]
pub enum DiskInodeType {
    File = 0,
    Directory = 1,
}

/// 磁盘上的索引节点：一个文件或目录的全部元信息。
///
/// # 为什么是"直接块 + 间接块"这种奇怪结构
///
/// 一个文件的内容散落在若干个数据块里，inode 得记住是哪些。直接列出来的话，
/// 大文件的 inode 会非常大 —— 而绝大多数文件都很小。
///
/// 所以用两级：前 27 个块号直接存在 inode 里（覆盖 13.5 KiB 以内的文件，
/// 也就是绝大多数），再大的话才去借一个"间接块"专门存块号表。
///
/// ```text
///   direct[0..27]  ──▶ 数据块
///   indirect1      ──▶ [块号 × 128] ──▶ 数据块
/// ```
///
/// 代价是访问大文件的后半部分要多读一次盘。真实文件系统会做到三级间接，
/// 原理一样。
#[repr(C)]
pub struct DiskInode {
    pub size: u32,
    pub direct: [u32; INODE_DIRECT_COUNT],
    pub indirect1: u32,
    type_: DiskInodeType,
    _pad: [u8; DISK_INODE_SIZE - 4 - INODE_DIRECT_COUNT * 4 - 4 - 4],
}

const _: () = assert!(core::mem::size_of::<DiskInode>() == DISK_INODE_SIZE);
const _: () = assert!(BLOCK_SZ.is_multiple_of(DISK_INODE_SIZE));

impl DiskInode {
    /// 初始化成一个空文件/空目录。
    pub fn initialize(&mut self, type_: DiskInodeType) {
        self.size = 0;
        self.direct = [0; INODE_DIRECT_COUNT];
        self.indirect1 = 0;
        self.type_ = type_;
    }

    pub fn is_dir(&self) -> bool {
        self.type_ == DiskInodeType::Directory
    }

    /// 原始类型值。只在报错时用来说明现场。
    pub fn raw_type(&self) -> u32 {
        // SAFETY: DiskInodeType 是 #[repr(u32)]，按 u32 读总是合法的。
        unsafe { *(&self.type_ as *const _ as *const u32) }
    }

    pub fn is_file(&self) -> bool {
        self.type_ == DiskInodeType::File
    }

    /// 装下 `size` 字节需要几个数据块（不含间接块自身）。
    fn data_blocks_for(size: u32) -> u32 {
        size.div_ceil(BLOCK_SZ as u32)
    }

    /// 装下 `size` 字节一共要占几个块（含间接块自身）。
    pub fn total_blocks_for(size: u32) -> u32 {
        let data = Self::data_blocks_for(size);
        if data as usize > INODE_DIRECT_COUNT {
            data + 1 // 多一个间接块
        } else {
            data
        }
    }

    /// 从 `self.size` 涨到 `new_size` 还需要几个新块。
    pub fn blocks_needed(&self, new_size: u32) -> u32 {
        assert!(new_size >= self.size);
        Self::total_blocks_for(new_size) - Self::total_blocks_for(self.size)
    }

    /// 文件内第 `inner_id` 个数据块，在磁盘上是第几块。
    pub fn block_id(&self, inner_id: u32, device: &Arc<dyn BlockDevice>) -> u32 {
        let inner = inner_id as usize;
        if inner < INODE_DIRECT_COUNT {
            return self.direct[inner];
        }
        // 走间接块：先读出块号表，再从表里取
        get_block_cache(self.indirect1 as usize, Arc::clone(device))
            .lock()
            .read(0, |table: &[u32; INODE_INDIRECT1_COUNT]| {
                table[inner - INODE_DIRECT_COUNT]
            })
    }

    /// 扩容。`new_blocks` 是调用方已经分配好的新块号。
    pub fn increase_size(
        &mut self,
        new_size: u32,
        new_blocks: Vec<u32>,
        device: &Arc<dyn BlockDevice>,
    ) {
        let mut current = Self::data_blocks_for(self.size) as usize;
        let target = Self::data_blocks_for(new_size) as usize;
        self.size = new_size;
        let mut blocks = new_blocks.into_iter();

        // 先填直接块
        while current < target.min(INODE_DIRECT_COUNT) {
            self.direct[current] = blocks.next().expect("扩容给的块不够");
            current += 1;
        }
        if target <= INODE_DIRECT_COUNT {
            return;
        }

        // 需要间接块了。第一次用到时先分配它自己
        if self.indirect1 == 0 {
            self.indirect1 = blocks.next().expect("扩容给的块不够（间接块）");
        }

        get_block_cache(self.indirect1 as usize, Arc::clone(device))
            .lock()
            .modify(0, |table: &mut [u32; INODE_INDIRECT1_COUNT]| {
                while current < target {
                    table[current - INODE_DIRECT_COUNT] =
                        blocks.next().expect("扩容给的块不够（间接区）");
                    current += 1;
                }
            });
    }

    /// 清空文件，返回**所有**该被回收的块号（含间接块自身）。
    ///
    /// 由调用方负责把它们还给位图 —— inode 不知道位图在哪，
    /// 这个分工让 `DiskInode` 保持成一个纯粹的数据结构。
    pub fn clear_size(&mut self, device: &Arc<dyn BlockDevice>) -> Vec<u32> {
        let mut freed = Vec::new();
        let data_blocks = Self::data_blocks_for(self.size) as usize;
        self.size = 0;

        for slot in self.direct.iter_mut().take(data_blocks.min(INODE_DIRECT_COUNT)) {
            freed.push(*slot);
            *slot = 0;
        }
        if data_blocks <= INODE_DIRECT_COUNT {
            return freed;
        }

        let indirect_count = data_blocks - INODE_DIRECT_COUNT;
        get_block_cache(self.indirect1 as usize, Arc::clone(device))
            .lock()
            .modify(0, |table: &mut [u32; INODE_INDIRECT1_COUNT]| {
                for slot in table.iter_mut().take(indirect_count) {
                    freed.push(*slot);
                    *slot = 0;
                }
            });
        freed.push(self.indirect1); // 间接块自己也要还
        self.indirect1 = 0;
        freed
    }

    /// 从 `offset` 开始读，最多读满 `buf`。返回实际读了多少。
    pub fn read_at(
        &self,
        offset: usize,
        buf: &mut [u8],
        device: &Arc<dyn BlockDevice>,
    ) -> usize {
        let end = (offset + buf.len()).min(self.size as usize);
        if offset >= end {
            return 0;
        }

        let mut read = 0;
        let mut pos = offset;
        while pos < end {
            let block_inner = (pos / BLOCK_SZ) as u32;
            let in_block = pos % BLOCK_SZ;
            let chunk = (BLOCK_SZ - in_block).min(end - pos);

            get_block_cache(self.block_id(block_inner, device) as usize, Arc::clone(device))
                .lock()
                .read(0, |data: &[u8; BLOCK_SZ]| {
                    buf[read..read + chunk].copy_from_slice(&data[in_block..in_block + chunk]);
                });

            read += chunk;
            pos += chunk;
        }
        read
    }

    /// 从 `offset` 开始写。调用方必须**先**把文件扩到足够大。
    pub fn write_at(&mut self, offset: usize, buf: &[u8], device: &Arc<dyn BlockDevice>) -> usize {
        let end = offset + buf.len();
        assert!(
            end <= self.size as usize,
            "写越界：文件只有 {} 字节，却要写到 {}。应该先 increase_size",
            self.size,
            end
        );

        let mut written = 0;
        let mut pos = offset;
        while pos < end {
            let block_inner = (pos / BLOCK_SZ) as u32;
            let in_block = pos % BLOCK_SZ;
            let chunk = (BLOCK_SZ - in_block).min(end - pos);

            get_block_cache(self.block_id(block_inner, device) as usize, Arc::clone(device))
                .lock()
                .modify(0, |data: &mut [u8; BLOCK_SZ]| {
                    data[in_block..in_block + chunk]
                        .copy_from_slice(&buf[written..written + chunk]);
                });

            written += chunk;
            pos += chunk;
        }
        written
    }
}

// ---------------------------------------------------------------------------
// 目录项
// ---------------------------------------------------------------------------

/// 文件名长度上限。
pub const NAME_LENGTH_LIMIT: usize = 27;
/// 一个目录项占的字节数。
pub const DIRENT_SZ: usize = 32;

/// 目录项：一个名字 → 一个 inode 编号。
///
/// **目录就是一个内容为目录项数组的普通文件。** 这是 Unix 的一个关键
/// 简化：不需要为目录发明单独的存储结构，`read_at` / `write_at` 那套
/// 直接就能用。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DirEntry {
    name: [u8; NAME_LENGTH_LIMIT + 1],
    inode_id: u32,
}

const _: () = assert!(core::mem::size_of::<DirEntry>() == DIRENT_SZ);

impl DirEntry {
    pub fn empty() -> Self {
        Self {
            name: [0; NAME_LENGTH_LIMIT + 1],
            inode_id: 0,
        }
    }

    pub fn new(name: &str, inode_id: u32) -> Self {
        assert!(
            name.len() <= NAME_LENGTH_LIMIT,
            "文件名太长：{name}（上限 {NAME_LENGTH_LIMIT}）"
        );
        let mut bytes = [0u8; NAME_LENGTH_LIMIT + 1];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Self {
            name: bytes,
            inode_id,
        }
    }

    pub fn name(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(self.name.len());
        core::str::from_utf8(&self.name[..len]).unwrap_or("<非法UTF8>")
    }

    pub fn inode_id(&self) -> u32 {
        self.inode_id
    }

    /// 当作字节序列，好交给 `read_at` / `write_at`。
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: repr(C) 的 POD 结构体，没有填充以外的不确定内容。
        unsafe { core::slice::from_raw_parts(self as *const _ as *const u8, DIRENT_SZ) }
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: 同上。
        unsafe { core::slice::from_raw_parts_mut(self as *mut _ as *mut u8, DIRENT_SZ) }
    }
}

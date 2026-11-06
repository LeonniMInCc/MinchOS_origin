//! 文件系统本体：管理位图和两个区，负责分配/回收 inode 和数据块。

use crate::bitmap::Bitmap;
use crate::block_cache::{get_block_cache, sync_all_blocks};
use crate::block_dev::BlockDevice;
use crate::layout::{DiskInode, DiskInodeType, SuperBlock, DISK_INODE_SIZE};
use crate::mutex::Mutex;
use crate::vfs::Inode;
use crate::BLOCK_SZ;
use alloc::sync::Arc;

pub struct EasyFileSystem {
    pub device: Arc<dyn BlockDevice>,
    pub inode_bitmap: Bitmap,
    pub data_bitmap: Bitmap,
    inode_area_start: u32,
    data_area_start: u32,
}

impl EasyFileSystem {
    /// 格式化一块新盘。
    ///
    /// 做完之后盘上有：一个合法的超级块、两张清零的位图、
    /// 以及一个空的根目录（inode 0）。
    pub fn format(
        device: Arc<dyn BlockDevice>,
        total_blocks: u32,
        inode_bitmap_blocks: u32,
        inode_area_blocks: u32,
    ) -> Arc<Mutex<Self>> {
        let data_total = total_blocks - 1 - inode_bitmap_blocks - inode_area_blocks;
        // 数据块位图自己也占块。一块位图管 4096 个数据块，
        // 所以位图块数 = 数据块数 / 4096 向上取整（再算一次扣除）。
        let data_bitmap_blocks =
            (data_total + crate::BITS_PER_BLOCK as u32) / (crate::BITS_PER_BLOCK as u32 + 1);
        let data_area_blocks = data_total - data_bitmap_blocks;

        let inode_bitmap = Bitmap::new(1, inode_bitmap_blocks as usize);
        let inode_area_start = 1 + inode_bitmap_blocks;
        let data_bitmap_start = inode_area_start + inode_area_blocks;
        let data_bitmap = Bitmap::new(data_bitmap_start as usize, data_bitmap_blocks as usize);
        let data_area_start = data_bitmap_start + data_bitmap_blocks;

        // 把元数据区全部清零 —— 位图必须从"全空闲"开始，
        // inode 区里的残留数据会被当成合法 inode 解释。
        for block in 0..data_area_start as usize {
            get_block_cache(block, Arc::clone(&device))
                .lock()
                .modify(0, |data: &mut [u8; BLOCK_SZ]| data.fill(0));
        }

        get_block_cache(0, Arc::clone(&device))
            .lock()
            .modify(0, |sb: &mut SuperBlock| {
                sb.initialize(
                    total_blocks,
                    inode_bitmap_blocks,
                    inode_area_blocks,
                    data_bitmap_blocks,
                    data_area_blocks,
                );
            });

        let fs = Arc::new(Mutex::new(Self {
            device: Arc::clone(&device),
            inode_bitmap,
            data_bitmap,
            inode_area_start,
            data_area_start,
        }));

        // 根目录固定是 0 号 inode。格式化时把它建出来。
        {
            let mut guard = fs.lock();
            let root = guard.alloc_inode();
            assert_eq!(root, 0, "根目录必须是 0 号 inode");
            let (block, offset) = guard.disk_inode_pos(0);
            get_block_cache(block as usize, Arc::clone(&device))
                .lock()
                .modify(offset, |inode: &mut DiskInode| {
                    inode.initialize(DiskInodeType::Directory);
                });
        }

        sync_all_blocks();
        fs
    }

    /// 挂载一块已经格式化过的盘。魔数不对就返回 `None`。
    pub fn open(device: Arc<dyn BlockDevice>) -> Option<Arc<Mutex<Self>>> {
        let sb = crate::layout::read_super_block(&device)?;

        let inode_area_start = sb.inode_area_start();
        let data_bitmap_start = sb.data_bitmap_start();
        let data_area_start = sb.data_area_start();

        Some(Arc::new(Mutex::new(Self {
            device,
            inode_bitmap: Bitmap::new(1, sb.inode_bitmap_blocks as usize),
            data_bitmap: Bitmap::new(data_bitmap_start as usize, sb.data_bitmap_blocks as usize),
            inode_area_start,
            data_area_start,
        })))
    }

    /// 根目录的 inode 句柄。
    pub fn root_inode(fs: &Arc<Mutex<Self>>) -> Inode {
        let device = Arc::clone(&fs.lock().device);
        let (block, offset) = fs.lock().disk_inode_pos(0);
        Inode::new(block, offset, Arc::clone(fs), device)
    }

    /// `inode_id` 号 inode 存在磁盘上的哪个位置。
    pub fn disk_inode_pos(&self, inode_id: u32) -> (u32, usize) {
        let per_block = (BLOCK_SZ / DISK_INODE_SIZE) as u32;
        (
            self.inode_area_start + inode_id / per_block,
            (inode_id % per_block) as usize * DISK_INODE_SIZE,
        )
    }

    pub fn alloc_inode(&mut self) -> u32 {
        self.inode_bitmap.alloc(&self.device).expect("inode 用尽") as u32
    }

    /// 分配一个数据块，返回**磁盘上的块号**（不是数据区内的编号）。
    pub fn alloc_data(&mut self) -> u32 {
        let id = self.data_bitmap.alloc(&self.device).expect("数据块用尽") as u32;
        let block = self.data_area_start + id;
        // 新分配的块必须清零：残留的旧数据会成为新文件的内容，
        // 那是信息泄露。和第 6 章页帧分配器清零是同一个道理。
        get_block_cache(block as usize, Arc::clone(&self.device))
            .lock()
            .modify(0, |data: &mut [u8; BLOCK_SZ]| data.fill(0));
        block
    }

    pub fn dealloc_data(&mut self, block: u32) {
        self.data_bitmap
            .dealloc(&self.device, (block - self.data_area_start) as usize);
    }

    /// `(已用 inode, 已用数据块, 数据块总数)`。
    pub fn usage(&self) -> (usize, usize, usize) {
        (
            self.inode_bitmap.allocated(&self.device),
            self.data_bitmap.allocated(&self.device),
            self.data_bitmap.maximum(),
        )
    }
}

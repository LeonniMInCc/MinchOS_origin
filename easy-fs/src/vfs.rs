//! 内存里的 inode 句柄：对外提供"文件"这个概念。
//!
//! [`crate::layout::DiskInode`] 是磁盘上的数据结构，只知道字节和块号。
//! `Inode` 是它在内存里的把手，知道自己在哪块盘的哪个位置，
//! 也知道文件系统在哪 —— 于是能做"扩容时去要新块"这类需要全局信息的事。
//!
//! # 目录就是一个普通文件
//!
//! 这是 Unix 的一个关键简化：目录的内容就是一串
//! [`DirEntry`]，用和普通文件完全相同的 `read_at` / `write_at` 存取。
//! 不需要为目录发明单独的存储结构。

use crate::block_cache::{get_block_cache, sync_all_blocks};
use crate::block_dev::BlockDevice;
use crate::efs::EasyFileSystem;
use crate::layout::{DirEntry, DiskInode, DiskInodeType, DIRENT_SZ, MAX_FILE_SIZE};
use crate::mutex::Mutex;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub struct Inode {
    block_id: usize,
    block_offset: usize,
    fs: Arc<Mutex<EasyFileSystem>>,
    device: Arc<dyn BlockDevice>,
}

impl Inode {
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        device: Arc<dyn BlockDevice>,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            device,
        }
    }

    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.device))
            .lock()
            .read(self.block_offset, f)
    }

    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.device))
            .lock()
            .modify(self.block_offset, f)
    }

    pub fn size(&self) -> usize {
        self.read_disk_inode(|inode| inode.size as usize)
    }

    pub fn is_dir(&self) -> bool {
        self.read_disk_inode(|inode| inode.is_dir())
    }

    /// 在这个目录里按名字查一个条目。
    fn find_inode_id(&self, name: &str, dir: &DiskInode) -> Option<u32> {
        assert!(
            dir.is_dir(),
            "只能在目录里查找：inode 位于块 {} 偏移 {}，size={}，type={}",
            self.block_id,
            self.block_offset,
            dir.size,
            dir.raw_type()
        );
        let count = dir.size as usize / DIRENT_SZ;
        let mut entry = DirEntry::empty();
        for i in 0..count {
            let read = dir.read_at(i * DIRENT_SZ, entry.as_bytes_mut(), &self.device);
            assert_eq!(read, DIRENT_SZ, "目录项读取不完整，文件系统可能已损坏");
            if entry.name() == name {
                return Some(entry.inode_id());
            }
        }
        None
    }

    /// 按名字打开一个文件。
    pub fn find(&self, name: &str) -> Option<Arc<Inode>> {
        let inode_id = self.read_disk_inode(|dir| self.find_inode_id(name, dir))?;
        let (block, offset) = self.fs.lock().disk_inode_pos(inode_id);
        Some(Arc::new(Self::new(
            block,
            offset,
            Arc::clone(&self.fs),
            Arc::clone(&self.device),
        )))
    }

    /// 在这个目录里建一个新文件。名字已存在时返回 `None`。
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        if self.read_disk_inode(|dir| self.find_inode_id(name, dir)).is_some() {
            return None; // 已经有了
        }

        // 1. 分配并初始化一个新 inode
        let (new_id, block, offset) = {
            let mut fs = self.fs.lock();
            let id = fs.alloc_inode();
            let (b, o) = fs.disk_inode_pos(id);
            (id, b, o)
        };
        get_block_cache(block as usize, Arc::clone(&self.device))
            .lock()
            .modify(offset, |inode: &mut DiskInode| {
                inode.initialize(DiskInodeType::File);
            });

        // 2. 在目录里追加一条目录项。
        //    目录本身也是文件，所以"追加"就是先扩容再写。
        self.modify_disk_inode(|dir| {
            let old_size = dir.size as usize;
            let new_size = old_size + DIRENT_SZ;
            self.grow(dir, new_size as u32);
            let entry = DirEntry::new(name, new_id);
            dir.write_at(old_size, entry.as_bytes(), &self.device);
        });

        sync_all_blocks();
        self.find(name)
    }

    /// 列出目录里的所有名字。
    pub fn ls(&self) -> Vec<String> {
        self.read_disk_inode(|dir| {
            assert!(dir.is_dir(), "只能对目录 ls");
            let count = dir.size as usize / DIRENT_SZ;
            let mut names = Vec::with_capacity(count);
            let mut entry = DirEntry::empty();
            for i in 0..count {
                dir.read_at(i * DIRENT_SZ, entry.as_bytes_mut(), &self.device);
                names.push(String::from(entry.name()));
            }
            names
        })
    }

    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        self.read_disk_inode(|inode| inode.read_at(offset, buf, &self.device))
    }

    /// 写。不够大就先扩容。
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        self.modify_disk_inode(|inode| {
            self.grow(inode, (offset + buf.len()) as u32);
            inode.write_at(offset, buf, &self.device)
        })
    }

    /// 清空文件内容，把占用的数据块全部还给位图。
    pub fn clear(&self) {
        self.modify_disk_inode(|inode| {
            let freed = inode.clear_size(&self.device);
            let mut fs = self.fs.lock();
            for block in freed {
                fs.dealloc_data(block);
            }
        });
        sync_all_blocks();
    }

    /// 把文件扩到至少 `new_size` 字节，需要的新块从文件系统要。
    fn grow(&self, inode: &mut DiskInode, new_size: u32) {
        if new_size <= inode.size {
            return;
        }
        assert!(
            new_size as usize <= MAX_FILE_SIZE,
            "文件超过大小上限 {MAX_FILE_SIZE} 字节"
        );
        let needed = inode.blocks_needed(new_size);
        let mut blocks = Vec::with_capacity(needed as usize);
        {
            let mut fs = self.fs.lock();
            for _ in 0..needed {
                blocks.push(fs.alloc_data());
            }
        }
        inode.increase_size(new_size, blocks, &self.device);
    }
}

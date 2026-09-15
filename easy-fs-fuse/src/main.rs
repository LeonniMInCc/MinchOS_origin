//! 在宿主机上制作 CCNUOS 的文件系统镜像。
//!
//! 用法：`easy-fs-fuse <输出文件> [块数]`
//!
//! 这个工具和内核共用同一份 `easy-fs` crate —— 格式化逻辑只写一遍，
//! 两边不可能出现"制作工具和内核对布局的理解不一致"这种问题。
//! 它实现 `BlockDevice` 的方式就是一个普通文件。

use easy_fs::{
    clear_block_cache, get_block_cache, sync_all_blocks, BlockDevice, SuperBlock, BLOCK_SZ,
};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

/// 用一个普通文件当块设备。
struct FileDisk(Mutex<std::fs::File>);

impl BlockDevice for FileDisk {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        let mut f = self.0.lock().unwrap();
        f.seek(SeekFrom::Start((block_id * BLOCK_SZ) as u64)).unwrap();
        assert_eq!(f.read(buf).unwrap(), BLOCK_SZ, "读块 {block_id} 不完整");
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) {
        let mut f = self.0.lock().unwrap();
        f.seek(SeekFrom::Start((block_id * BLOCK_SZ) as u64)).unwrap();
        assert_eq!(f.write(buf).unwrap(), BLOCK_SZ, "写块 {block_id} 不完整");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: {} <输出文件> [块数，默认 4096]", args[0]);
        std::process::exit(2);
    }
    let path = &args[1];
    let total_blocks: u32 = args
        .get(2)
        .map(|s| s.parse().expect("块数必须是数字"))
        .unwrap_or(4096);

    // 各区大小。这里的划分很朴素：位图各一块（够管 4096 个对象），
    // inode 区给 32 块，剩下全是数据。
    let inode_bitmap_blocks = 1u32;
    let inode_area_blocks = 32u32;
    let data_bitmap_blocks = 1u32;
    let meta_blocks = 1 + inode_bitmap_blocks + inode_area_blocks + data_bitmap_blocks;
    assert!(
        total_blocks > meta_blocks,
        "镜像太小：至少要 {} 块",
        meta_blocks + 1
    );
    let data_area_blocks = total_blocks - meta_blocks;

    // 先把文件撑到目标大小并全部清零 —— 位图必须从"全空闲"开始。
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap_or_else(|e| panic!("打不开 {path}：{e}"));
    file.set_len((total_blocks as usize * BLOCK_SZ) as u64).unwrap();

    let device: Arc<dyn BlockDevice> = Arc::new(FileDisk(Mutex::new(file)));

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

    // 写回策略意味着数据还在内存里，必须显式落盘
    sync_all_blocks();
    clear_block_cache();

    // 自己读一遍验证，别把一个坏镜像交出去
    let sb = easy_fs::read_super_block(&device).expect("刚写的超级块自己读不回来");
    println!("镜像已生成：{path}");
    println!("  总块数        {:>6}  ({} KiB)", sb.total_blocks, sb.total_blocks as usize * BLOCK_SZ / 1024);
    println!("  超级块        块 0");
    println!("  索引节点位图  块 1..{}", sb.inode_area_start());
    println!("  索引节点区    块 {}..{}", sb.inode_area_start(), sb.data_bitmap_start());
    println!("  数据块位图    块 {}..{}", sb.data_bitmap_start(), sb.data_area_start());
    println!("  数据块区      块 {}..{}", sb.data_area_start(), sb.total_blocks);
    println!("  魔数          {:#010x}", sb.magic());
}

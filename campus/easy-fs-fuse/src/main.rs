//! 在宿主机上制作 CCNUOS 的文件系统镜像。
//!
//! 用法：`easy-fs-fuse <输出镜像> <用户程序目录> [块数]`
//!
//! 它把目录下所有 `*.elf` 打包进一个 easy-fs 镜像。内核启动后挂载这块盘，
//! `initproc` 和所有命令都从这里加载 —— 第 13 周之前它们是被
//! `.incbin` 嵌进内核镜像里的。
//!
//! 这个工具和内核**共用同一份 `easy-fs` crate**，所以不可能出现
//! "制作工具和内核对磁盘布局的理解不一致"这种问题。

use easy_fs::{clear_block_cache, sync_all_blocks, BlockDevice, EasyFileSystem, BLOCK_SZ};
use std::fs::{read_dir, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

/// 用一个普通文件当块设备。
struct FileDisk(Mutex<std::fs::File>);

impl BlockDevice for FileDisk {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        let mut f = self.0.lock().unwrap();
        f.seek(SeekFrom::Start((block_id * BLOCK_SZ) as u64))
            .unwrap();
        assert_eq!(f.read(buf).unwrap(), BLOCK_SZ, "读块 {block_id} 不完整");
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) {
        let mut f = self.0.lock().unwrap();
        f.seek(SeekFrom::Start((block_id * BLOCK_SZ) as u64))
            .unwrap();
        assert_eq!(f.write(buf).unwrap(), BLOCK_SZ, "写块 {block_id} 不完整");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "用法: {} <输出镜像> <用户程序目录> [块数，默认 4096]",
            args[0]
        );
        std::process::exit(2);
    }
    let (image, src_dir) = (&args[1], &args[2]);
    let total_blocks: u32 = args
        .get(3)
        .map(|s| s.parse().expect("块数必须是数字"))
        .unwrap_or(4096);

    // 先把文件撑到目标大小 —— 位图和 inode 区必须从全零开始
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(image)
        .unwrap_or_else(|e| panic!("打不开 {image}：{e}"));
    file.set_len((total_blocks as usize * BLOCK_SZ) as u64)
        .unwrap();

    let device: Arc<dyn BlockDevice> = Arc::new(FileDisk(Mutex::new(file)));
    let fs = EasyFileSystem::format(Arc::clone(&device), total_blocks, 1, 32);
    let root = EasyFileSystem::root_inode(&fs);

    // 收集要打包的程序，按名字排序让镜像内容可复现
    let mut programs: Vec<(String, std::path::PathBuf)> = read_dir(src_dir)
        .unwrap_or_else(|e| panic!("读不了目录 {src_dir}：{e}"))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()? != "elf" {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            Some((name, path))
        })
        .collect();
    programs.sort();

    assert!(!programs.is_empty(), "{src_dir} 下没有找到任何 *.elf");

    println!("打包 {} 个程序到 {image}：", programs.len());
    let mut total_bytes = 0usize;
    for (name, path) in &programs {
        let mut data = Vec::new();
        std::fs::File::open(path)
            .unwrap()
            .read_to_end(&mut data)
            .unwrap();

        let inode = root
            .create(name)
            .unwrap_or_else(|| panic!("创建 {name} 失败（重名？）"));
        inode.write_at(0, &data);

        // 立刻读回来核对。制作工具交出一个坏镜像的代价太大了 ——
        // 内核启动失败时，你根本分不清是内核的问题还是镜像的问题。
        let mut back = vec![0u8; data.len()];
        assert_eq!(
            inode.read_at(0, &mut back),
            data.len(),
            "{name} 回读长度不对"
        );
        assert_eq!(back, data, "{name} 回读内容不一致");

        total_bytes += data.len();
        println!("  {name:<20} {:>7} 字节", data.len());
    }

    sync_all_blocks();
    clear_block_cache();

    // 重新挂载一遍，确认交出去的是个能用的镜像
    let fs = EasyFileSystem::open(Arc::clone(&device)).expect("刚做好的镜像自己挂不上");
    let root = EasyFileSystem::root_inode(&fs);
    let listed = root.ls();
    assert_eq!(listed.len(), programs.len(), "重新挂载后程序数量对不上");

    let (inodes, data_used, data_total) = fs.lock().usage();
    println!(
        "完成：{} 个文件，共 {} KiB；inode {} 个，数据块 {}/{}（{} KiB / {} KiB）",
        listed.len(),
        total_bytes / 1024,
        inodes,
        data_used,
        data_total,
        data_used * BLOCK_SZ / 1024,
        total_blocks as usize * BLOCK_SZ / 1024,
    );
}

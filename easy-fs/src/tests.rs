//! 宿主机上的单元测试。
//!
//! 这些测试**不需要 QEMU、不需要内核**：块设备用一段内存模拟，
//! `cargo test` 秒级跑完，断言失败时有完整的信息。
//!
//! 这正是把文件系统做成独立 crate 的全部意义。

use super::*;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use std::string::ToString;
use std::sync::Mutex as StdMutex;

/// 一个用内存模拟的块设备，顺便统计真正落到"磁盘"上的读写次数。
///
/// 统计这件事很关键：块缓存的价值就在于**减少设备 I/O**，
/// 而这个数字是唯一能直接证明它的证据。
struct MemDisk {
    blocks: StdMutex<Vec<[u8; BLOCK_SZ]>>,
    reads: StdMutex<usize>,
    writes: StdMutex<usize>,
}

impl MemDisk {
    fn new(n: usize) -> Arc<Self> {
        Arc::new(Self {
            blocks: StdMutex::new(vec![[0u8; BLOCK_SZ]; n]),
            reads: StdMutex::new(0),
            writes: StdMutex::new(0),
        })
    }
    fn reads(&self) -> usize {
        *self.reads.lock().unwrap()
    }
    fn writes(&self) -> usize {
        *self.writes.lock().unwrap()
    }
}

impl BlockDevice for MemDisk {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        *self.reads.lock().unwrap() += 1;
        buf.copy_from_slice(&self.blocks.lock().unwrap()[block_id]);
    }
    fn write_block(&self, block_id: usize, buf: &[u8]) {
        *self.writes.lock().unwrap() += 1;
        self.blocks.lock().unwrap()[block_id].copy_from_slice(buf);
    }
}

/// 块缓存是个全局单例，所以测试之间必须串行、且每次从干净状态开始。
static TEST_LOCK: StdMutex<()> = StdMutex::new(());

fn with_clean_cache<T>(f: impl FnOnce() -> T) -> T {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_block_cache();
    let result = f();
    clear_block_cache();
    result
}

#[test]
fn 块设备读写往返() {
    with_clean_cache(|| {
        let disk = MemDisk::new(8);
        let dev: Arc<dyn BlockDevice> = disk.clone();

        let mut buf = [0u8; BLOCK_SZ];
        buf[0] = 0xAB;
        buf[BLOCK_SZ - 1] = 0xCD;
        dev.write_block(3, &buf);

        let mut back = [0u8; BLOCK_SZ];
        dev.read_block(3, &mut back);
        assert_eq!(back[0], 0xAB);
        assert_eq!(back[BLOCK_SZ - 1], 0xCD);
        // 别的块不受影响
        dev.read_block(2, &mut back);
        assert!(back.iter().all(|&b| b == 0));
    });
}

#[test]
fn 缓存命中不再碰设备() {
    with_clean_cache(|| {
        let disk = MemDisk::new(8);
        let dev: Arc<dyn BlockDevice> = disk.clone();

        // 第一次访问：必须去读磁盘
        get_block_cache(1, dev.clone()).lock().read(0, |v: &u32| *v);
        assert_eq!(disk.reads(), 1, "首次访问应该读一次磁盘");

        // 再访问 10 次：全部命中，磁盘一次都不碰
        for _ in 0..10 {
            get_block_cache(1, dev.clone()).lock().read(0, |v: &u32| *v);
        }
        assert_eq!(disk.reads(), 1, "命中的访问不该产生磁盘 I/O");

        let (hits, misses, _) = block_cache_stats();
        assert_eq!((hits, misses), (10, 1));
    });
}

#[test]
fn 多次修改只回写一次() {
    with_clean_cache(|| {
        let disk = MemDisk::new(8);
        let dev: Arc<dyn BlockDevice> = disk.clone();

        // 改同一个块 20 次
        for i in 0..20u32 {
            get_block_cache(2, dev.clone())
                .lock()
                .modify(0, |v: &mut u32| *v = i);
        }
        // 写回策略：此刻还一个字节都没落盘
        assert_eq!(disk.writes(), 0, "写回策略下，修改不该立刻产生磁盘写");

        sync_all_blocks();
        assert_eq!(disk.writes(), 1, "20 次修改应该合并成 1 次磁盘写");

        // 内容确实落盘了
        let mut buf = [0u8; BLOCK_SZ];
        disk.read_block(2, &mut buf);
        assert_eq!(u32::from_ne_bytes(buf[..4].try_into().unwrap()), 19);
    });
}

#[test]
fn 淘汰时自动回写() {
    with_clean_cache(|| {
        let disk = MemDisk::new(64);
        let dev: Arc<dyn BlockDevice> = disk.clone();

        // 弄脏第 0 块，然后访问足够多的别的块把它挤出去
        get_block_cache(0, dev.clone())
            .lock()
            .modify(0, |v: &mut u32| *v = 0xDEAD);
        assert_eq!(disk.writes(), 0);

        for id in 1..=20 {
            get_block_cache(id, dev.clone())
                .lock()
                .read(0, |v: &u32| *v);
        }

        let (_, _, evictions) = block_cache_stats();
        assert!(
            evictions > 0,
            "访问 21 个块应该触发淘汰（缓存只有 16 个槽）"
        );
        assert!(disk.writes() >= 1, "被淘汰的脏块应该自动写回");

        // 而且写回的内容是对的
        let mut buf = [0u8; BLOCK_SZ];
        disk.read_block(0, &mut buf);
        assert_eq!(u32::from_ne_bytes(buf[..4].try_into().unwrap()), 0xDEAD);
    });
}

#[test]
fn 位图分配与回收() {
    with_clean_cache(|| {
        let disk = MemDisk::new(8);
        let dev: Arc<dyn BlockDevice> = disk.clone();
        let bitmap = Bitmap::new(1, 1); // 第 1 块，共 1 块 = 4096 位

        assert_eq!(bitmap.maximum(), BITS_PER_BLOCK);
        assert_eq!(bitmap.allocated(&dev), 0);

        // 连续分配应该拿到连续编号
        let ids: Vec<usize> = (0..5).map(|_| bitmap.alloc(&dev).unwrap()).collect();
        assert_eq!(ids, vec![0, 1, 2, 3, 4]);
        assert_eq!(bitmap.allocated(&dev), 5);

        // 回收中间一个，下次分配应该优先补这个洞
        bitmap.dealloc(&dev, 2);
        assert_eq!(bitmap.allocated(&dev), 4);
        assert_eq!(bitmap.alloc(&dev), Some(2), "应该复用刚回收的编号");
    });
}

#[test]
fn 位图用尽返回_none() {
    with_clean_cache(|| {
        let disk = MemDisk::new(8);
        let dev: Arc<dyn BlockDevice> = disk.clone();
        // 只给 1 位可用的位图不好构造，改用整块但全部分配完
        let bitmap = Bitmap::new(1, 1);

        for _ in 0..BITS_PER_BLOCK {
            assert!(bitmap.alloc(&dev).is_some());
        }
        assert_eq!(bitmap.allocated(&dev), BITS_PER_BLOCK);
        assert_eq!(bitmap.alloc(&dev), None, "用尽后应该返回 None 而不是 panic");
    });
}

#[test]
#[should_panic(expected = "double free")]
fn 位图重复回收会_panic() {
    with_clean_cache(|| {
        let disk = MemDisk::new(8);
        let dev: Arc<dyn BlockDevice> = disk.clone();
        let bitmap = Bitmap::new(1, 1);

        let id = bitmap.alloc(&dev).unwrap();
        bitmap.dealloc(&dev, id);
        bitmap.dealloc(&dev, id); // 第二次应该炸
    });
}

#[test]
fn 超级块写入与校验() {
    with_clean_cache(|| {
        let disk = MemDisk::new(64);
        let dev: Arc<dyn BlockDevice> = disk.clone();

        // 未格式化的盘：魔数不对，挂载应该失败
        assert!(
            read_super_block(&dev).is_none(),
            "空盘不该被当成合法文件系统"
        );

        get_block_cache(0, dev.clone())
            .lock()
            .modify(0, |sb: &mut SuperBlock| {
                sb.initialize(64, 1, 8, 1, 53);
            });
        sync_all_blocks();
        clear_block_cache(); // 强制下次从"磁盘"重读

        let sb = read_super_block(&dev).expect("格式化后应该能挂载");
        assert_eq!(sb.magic(), EFS_MAGIC);
        assert_eq!(sb.total_blocks, 64);
        // 各区起点是算出来的，核对一遍
        assert_eq!(sb.inode_area_start(), 2);
        assert_eq!(sb.data_bitmap_start(), 10);
        assert_eq!(sb.data_area_start(), 11);
        assert_eq!(sb.data_area_start() + sb.data_area_blocks, 64);
    });
}

// ---------------------------------------------------------------------------
// 文件系统
// ---------------------------------------------------------------------------

/// 格式化一块新盘并返回 `(文件系统, 根目录, 磁盘)`。
fn fresh_fs(blocks: u32) -> (Arc<Mutex<EasyFileSystem>>, Inode, Arc<MemDisk>) {
    let disk = MemDisk::new(blocks as usize);
    let fs = EasyFileSystem::format(disk.clone(), blocks, 1, 8);
    let root = EasyFileSystem::root_inode(&fs);
    (fs, root, disk)
}

#[test]
fn 格式化后根目录为空() {
    with_clean_cache(|| {
        let (fs, root, _) = fresh_fs(256);
        assert!(root.is_dir());
        assert_eq!(root.size(), 0);
        assert!(root.ls().is_empty());
        // 只有根目录这一个 inode 被占用
        let (inodes, _, _) = fs.lock().usage();
        assert_eq!(inodes, 1);
    });
}

#[test]
fn 建文件写读往返() {
    with_clean_cache(|| {
        let (_, root, _) = fresh_fs(256);

        let f = root.create("hello.txt").expect("创建失败");
        assert_eq!(f.size(), 0);

        let text = b"hello, ccnuos file system!";
        assert_eq!(f.write_at(0, text), text.len());
        assert_eq!(f.size(), text.len());

        let mut buf = [0u8; 64];
        let n = f.read_at(0, &mut buf);
        assert_eq!(n, text.len());
        assert_eq!(&buf[..n], text);

        // 从偏移读
        let n = f.read_at(7, &mut buf);
        assert_eq!(&buf[..n], b"ccnuos file system!");

        // 读超出文件末尾应该返回 0
        assert_eq!(f.read_at(1000, &mut buf), 0);
    });
}

#[test]
fn 重名创建返回_none() {
    with_clean_cache(|| {
        let (_, root, _) = fresh_fs(256);
        assert!(root.create("a").is_some());
        assert!(root.create("a").is_none(), "重名不该建出第二个");
        assert_eq!(root.ls(), vec!["a".to_string()]);
    });
}

#[test]
fn 目录列表与查找() {
    with_clean_cache(|| {
        let (_, root, _) = fresh_fs(256);
        for name in ["alpha", "beta", "gamma"] {
            root.create(name).unwrap();
        }
        assert_eq!(root.ls(), vec!["alpha", "beta", "gamma"]);
        assert!(root.find("beta").is_some());
        assert!(root.find("delta").is_none());
    });
}

#[test]
fn 大文件跨越间接块() {
    with_clean_cache(|| {
        // 27 个直接块只够 13.5 KiB，写 40 KiB 必然要用上间接块
        let (_, root, _) = fresh_fs(512);
        let f = root.create("big").unwrap();

        let size = 40 * 1024;
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        assert_eq!(f.write_at(0, &data), size);
        assert_eq!(f.size(), size);

        // 整体读回来逐字节核对
        let mut back = vec![0u8; size];
        assert_eq!(f.read_at(0, &mut back), size);
        assert_eq!(back, data, "跨间接块的内容不一致");

        // 跨块边界的局部读也要对
        let mut chunk = [0u8; 100];
        let off = 27 * BLOCK_SZ - 50; // 正好骑在直接块和间接块的交界上
        f.read_at(off, &mut chunk);
        assert_eq!(&chunk[..], &data[off..off + 100]);
    });
}

#[test]
fn 清空后归还数据块() {
    with_clean_cache(|| {
        let (fs, root, _) = fresh_fs(512);
        let (_, before, _) = fs.lock().usage();

        let f = root.create("tmp").unwrap();
        f.write_at(0, &vec![0xAB; 20 * 1024]);
        let (_, after_write, _) = fs.lock().usage();
        assert!(after_write > before, "写入应该占用数据块");

        f.clear();
        assert_eq!(f.size(), 0);
        let (_, after_clear, _) = fs.lock().usage();
        // 目录项占的那一块还在（目录也是文件），但文件自己的块该还回来
        assert!(
            after_clear < after_write,
            "clear 之后数据块没归还：{after_write} → {after_clear}"
        );
    });
}

#[test]
fn 重新挂载后数据还在() {
    with_clean_cache(|| {
        let disk = MemDisk::new(512);
        let text = b"survives a remount";

        {
            let fs = EasyFileSystem::format(disk.clone(), 512, 1, 8);
            let root = EasyFileSystem::root_inode(&fs);
            let f = root.create("persist").unwrap();
            f.write_at(0, text);
            sync_all_blocks();
        }

        // 清空缓存，模拟重启 —— 之后所有数据只能来自"磁盘"
        clear_block_cache();

        let fs = EasyFileSystem::open(disk.clone()).expect("重新挂载失败");
        let root = EasyFileSystem::root_inode(&fs);
        assert_eq!(root.ls(), vec!["persist".to_string()]);

        let f = root.find("persist").expect("重启后文件不见了");
        let mut buf = [0u8; 32];
        let n = f.read_at(0, &mut buf);
        assert_eq!(&buf[..n], text);
    });
}

#[test]
fn 新分配的数据块是干净的() {
    with_clean_cache(|| {
        let (_, root, _) = fresh_fs(512);

        // 写一个文件、清空、再建一个新文件 —— 新文件不该看到旧内容
        let a = root.create("a").unwrap();
        a.write_at(0, &vec![0xEE; 4096]);
        a.clear();

        let b = root.create("b").unwrap();
        b.write_at(0, &[0u8; 8]); // 只写 8 字节，但会占一整块
        let mut buf = [0u8; 8];
        b.read_at(0, &mut buf);
        assert!(
            buf.iter().all(|&x| x == 0),
            "新文件里出现了上一个文件的残留"
        );
    });
}

#[test]
fn 多个文件后根目录仍然完好() {
    with_clean_cache(|| {
        // 复现内核里的场景：镜像里已经有二十来个文件，
        // 启动时再创建一个（bootcount），然后还要能找到原来的程序。
        let disk = MemDisk::new(4096);
        let fs = EasyFileSystem::format(disk.clone(), 4096, 1, 32);
        let root = EasyFileSystem::root_inode(&fs);

        for i in 0..21 {
            let name = std::format!("prog{i:02}");
            let f = root
                .create(&name)
                .unwrap_or_else(|| panic!("创建 {name} 失败"));
            f.write_at(0, &vec![i as u8; 9000]); // 和真实程序差不多大
        }
        assert_eq!(root.ls().len(), 21);
        assert!(root.is_dir(), "建了 21 个文件之后根目录就不是目录了");

        // 再建一个（模拟 bootcount）
        let counter = root.create("bootcount").expect("创建 bootcount 失败");
        counter.write_at(0, &[1u8]);

        assert!(root.is_dir(), "建 bootcount 之后根目录被写坏了");
        assert_eq!(root.ls().len(), 22);

        // 原来的程序还要能找到、内容还要对
        let f = root.find("prog05").expect("找不到 prog05");
        let mut buf = [0u8; 16];
        f.read_at(0, &mut buf);
        assert!(buf.iter().all(|&b| b == 5), "prog05 的内容被破坏了");
    });
}

/// 写回缓存的契约：**不调 `sync_all_blocks`，数据就不在磁盘上。**
///
/// 这条看起来是废话，但它正是第 16 周抓到的那个 bug 的根因 ——
/// 内核关机时漏了这一步，于是用户写的文件重启后变成空文件。
/// crate 这边一直是对的，错的是调用方。
///
/// 所以这个测试盯的不是"sync 能不能工作"（`重新挂载后数据还在` 已经验了），
/// 而是**不 sync 的后果有多安静**：没有报错、没有返回值异常，
/// 只是数据悄悄地不见了。把它钉死在这里，下次有人重构缓存时能看到代价。
#[test]
fn 不调用_sync_数据就只在内存里() {
    with_clean_cache(|| {
        let disk = MemDisk::new(512);
        let text = b"this must not reach the disk yet";

        let fs = EasyFileSystem::format(disk.clone(), 512, 1, 8);
        let root = EasyFileSystem::root_inode(&fs);
        let f = root.create("ghost").unwrap();
        f.write_at(0, text);

        // 绕开缓存，直接翻"磁盘"上的字节
        let on_disk = |needle: &[u8]| {
            disk.blocks
                .lock()
                .unwrap()
                .iter()
                .any(|b| b.windows(needle.len()).any(|w| w == needle))
        };

        assert!(
            !on_disk(text),
            "还没 sync，数据就已经落盘了 —— 那说明这不是写回缓存"
        );

        let before = disk.writes();
        let n = sync_all_blocks();
        assert!(n > 0, "有脏块没写回，sync_all_blocks 却报告 0");
        assert_eq!(
            disk.writes() - before,
            n,
            "报告的写回块数和设备上真实发生的写次数对不上"
        );
        assert!(on_disk(text), "sync 之后数据仍然不在磁盘上");

        // 再同步一次应该什么都不做 —— 脏标记已经清掉了
        assert_eq!(sync_all_blocks(), 0, "重复 sync 不该再写盘");
    });
}

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
            get_block_cache(id, dev.clone()).lock().read(0, |v: &u32| *v);
        }

        let (_, _, evictions) = block_cache_stats();
        assert!(evictions > 0, "访问 21 个块应该触发淘汰（缓存只有 16 个槽）");
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
        assert!(read_super_block(&dev).is_none(), "空盘不该被当成合法文件系统");

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

//! 块缓存。
//!
//! # 为什么必须有这一层
//!
//! 磁盘比内存慢几个数量级。而文件系统的访问模式极度集中：超级块、位图、
//! inode 这些元数据会被反复读，一次文件操作可能要碰同一个块十几次。
//!
//! 所以在文件系统和块设备之间插一层缓存：
//!
//! ```text
//!   文件系统 ──▶ 块缓存 ──▶ BlockDevice ──▶ 磁盘
//!                 ↑
//!            命中就不下去了
//! ```
//!
//! # 两个关键设计
//!
//! **一、写回而不是写穿。** 修改只改内存里的副本，打上"脏"标记，
//! 等这个缓存项被淘汰时才真正写磁盘。好处是连续多次修改同一个块
//! 只产生一次磁盘写。代价是掉电会丢数据 —— 所以必须提供
//! [`sync_all_blocks`] 让上层在关键时刻强制落盘。
//!
//! **二、用 `Drop` 触发回写。** 缓存项被淘汰时自动写回，
//! 不需要在每个淘汰点手写一遍。和页帧、PID、内核栈是同一个套路 ——
//! 这个项目里 RAII 已经用了第四次了。

use crate::block_dev::BlockDevice;
use crate::mutex::Mutex;
use crate::BLOCK_SZ;
use alloc::collections::VecDeque;
use alloc::sync::Arc;

/// 最多缓存多少个块。
///
/// 16 个块 = 8 KiB。故意开得小：这样 LRU 淘汰和脏块回写这两条路径
/// 在日常使用中就会被真实走到，而不是一组从没被测试过的分支。
/// （和第 11 章把管道缓冲区开成 32 字节是同一个考虑。）
const CACHE_LIMIT: usize = 16;

/// 一个被缓存的块。
pub struct BlockCache {
    data: [u8; BLOCK_SZ],
    block_id: usize,
    device: Arc<dyn BlockDevice>,
    /// 内存里的副本和磁盘上的不一致了吗
    dirty: bool,
}

impl BlockCache {
    /// 从磁盘读进来。
    fn new(block_id: usize, device: Arc<dyn BlockDevice>) -> Self {
        let mut data = [0u8; BLOCK_SZ];
        device.read_block(block_id, &mut data);
        Self {
            data,
            block_id,
            device,
            dirty: false,
        }
    }

    fn addr_of_offset(&self, offset: usize) -> usize {
        &self.data[offset] as *const _ as usize
    }

    /// 把块内 `offset` 处的数据当作一个 `T` 来读。
    ///
    /// # Panics
    ///
    /// `offset + size_of::<T>()` 超出块边界时 panic。一个 `T` 横跨两个块
    /// 是没有意义的 —— 那两个块在磁盘上相邻，但在内存里是两个独立的缓存项。
    pub fn get_ref<T>(&self, offset: usize) -> &T {
        let size = core::mem::size_of::<T>();
        assert!(
            offset + size <= BLOCK_SZ,
            "读取跨越了块边界：offset={offset}, size={size}"
        );
        // SAFETY: 上面刚检查过范围。对齐由调用方通过 offset 保证。
        unsafe { &*(self.addr_of_offset(offset) as *const T) }
    }

    /// 同 [`Self::get_ref`]，但取可变引用，并把这个块标记为脏。
    pub fn get_mut<T>(&mut self, offset: usize) -> &mut T {
        let size = core::mem::size_of::<T>();
        assert!(
            offset + size <= BLOCK_SZ,
            "写入跨越了块边界：offset={offset}, size={size}"
        );
        self.dirty = true;
        // SAFETY: 同 get_ref。
        unsafe { &mut *(self.addr_of_offset(offset) as *mut T) }
    }

    /// 读一个结构体并对它做点什么。
    pub fn read<T, V>(&self, offset: usize, f: impl FnOnce(&T) -> V) -> V {
        f(self.get_ref(offset))
    }

    /// 修改一个结构体。
    pub fn modify<T, V>(&mut self, offset: usize, f: impl FnOnce(&mut T) -> V) -> V {
        f(self.get_mut(offset))
    }

    /// 如果是脏的，写回磁盘。返回是否真的写了。
    pub fn sync(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        self.device.write_block(self.block_id, &self.data);
        self.dirty = false;
        true
    }
}

impl Drop for BlockCache {
    fn drop(&mut self) {
        // 缓存项被淘汰（或整个文件系统卸载）时自动落盘。
        // 忘记回写是文件系统里最典型的数据丢失来源，交给编译器管更稳妥。
        self.sync();
    }
}

/// 缓存池。
struct BlockCacheManager {
    /// 越靠后越新。淘汰时从前往后找第一个没人在用的。
    queue: VecDeque<(usize, Arc<Mutex<BlockCache>>)>,
    hits: usize,
    misses: usize,
    evictions: usize,
}

impl BlockCacheManager {
    const fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    fn get(&mut self, block_id: usize, device: Arc<dyn BlockDevice>) -> Arc<Mutex<BlockCache>> {
        if let Some(idx) = self.queue.iter().position(|(id, _)| *id == block_id) {
            self.hits += 1;
            // 命中：挪到队尾（最近使用）
            let entry = self.queue.remove(idx).unwrap();
            let cache = Arc::clone(&entry.1);
            self.queue.push_back(entry);
            return cache;
        }

        self.misses += 1;

        if self.queue.len() == CACHE_LIMIT {
            // 淘汰最久没用、且**当前没人持有**的那个。
            //
            // `Arc::strong_count == 1` 意味着只有缓存池自己拿着它。
            // 有人正拿着的块不能扔 —— 扔了会触发 Drop 回写，
            // 而持有者手里那份还在被修改，两边就打架了。
            let victim = self
                .queue
                .iter()
                .position(|(_, c)| Arc::strong_count(c) == 1)
                .expect("块缓存全被占用，无法淘汰（上层持有的块太多了）");
            self.queue.remove(victim); // Drop 在这里触发回写
            self.evictions += 1;
        }

        let cache = Arc::new(Mutex::new(BlockCache::new(block_id, device)));
        self.queue.push_back((block_id, Arc::clone(&cache)));
        cache
    }

    /// 返回真正写回了几个块。
    fn sync_all(&self) -> usize {
        self.queue
            .iter()
            .filter(|(_, cache)| cache.lock().sync())
            .count()
    }

    fn clear(&mut self) {
        self.queue.clear(); // Drop 逐个触发回写
        self.hits = 0;
        self.misses = 0;
        self.evictions = 0;
    }
}

static MANAGER: Mutex<BlockCacheManager> = Mutex::new(BlockCacheManager::new());

/// 取一个块的缓存。命中就直接返回内存里的副本。
pub fn get_block_cache(block_id: usize, device: Arc<dyn BlockDevice>) -> Arc<Mutex<BlockCache>> {
    MANAGER.lock().get(block_id, device)
}

/// 把所有脏块强制写回磁盘，返回真正写回了几个块。
///
/// 写回策略意味着数据可能只在内存里。关机前、或者任何"必须确保落盘"的
/// 时刻，都要调它。
///
/// # 漏调它的后果是静默丢数据
///
/// 内核第 12–15 周一直漏了关机时的这一步。表现是：文件在，内容是空的。
/// 而且同一次运行里读得到（缓存命中），所以演示全都"正常"。
/// 详见 `os/src/task/processor.rs` 里 `sync_disk` 的注释。
pub fn sync_all_blocks() -> usize {
    MANAGER.lock().sync_all()
}

/// 写回所有脏块并清空缓存。
///
/// 卸载文件系统、或者换一个块设备时必须调用 —— 因为缓存只按**块号**
/// 索引，并不区分设备。单设备场景下这没问题，但换设备时旧数据会串台。
/// 这是一个有意的简化，写在这里免得以后有人踩。
pub fn clear_block_cache() {
    MANAGER.lock().clear();
}

/// `(命中, 未命中, 淘汰次数)`。只用于观察缓存行为。
pub fn block_cache_stats() -> (usize, usize, usize) {
    let m = MANAGER.lock();
    (m.hits, m.misses, m.evictions)
}

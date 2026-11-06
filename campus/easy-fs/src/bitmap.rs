//! 位图：管理"哪些索引节点/数据块已经被占用"。
//!
//! 每一位对应一个可分配的对象，0 = 空闲，1 = 已用。
//! 分配就是"找第一个 0 并置 1"，回收就是"把某一位清 0"。
//!
//! 用位图而不是链表，是因为它**紧凑**：一个 512 字节的块能管 4096 个对象。
//! 代价是分配要扫描，但位运算扫 4096 位只需要 64 次 `u64` 比较。

use crate::block_cache::get_block_cache;
use crate::block_dev::BlockDevice;
use crate::BITS_PER_BLOCK;
use alloc::sync::Arc;

/// 一个位图块：512 字节 = 64 个 u64 = 4096 位。
///
/// 用 `u64` 而不是 `u8` 是为了让"这一组有没有空位"变成一次比较
/// （`!= u64::MAX`），而不是八次。
type BitmapBlock = [u64; 64];

const U64_PER_BLOCK: usize = 64;
const BITS_PER_U64: usize = 64;

pub struct Bitmap {
    start_block_id: usize,
    blocks: usize,
}

impl Bitmap {
    pub fn new(start_block_id: usize, blocks: usize) -> Self {
        Self {
            start_block_id,
            blocks,
        }
    }

    /// 这张位图最多能管多少个对象。
    pub fn maximum(&self) -> usize {
        self.blocks * BITS_PER_BLOCK
    }

    /// 分配一个，返回它的编号。没有空位返回 `None`。
    pub fn alloc(&self, device: &Arc<dyn BlockDevice>) -> Option<usize> {
        for block_offset in 0..self.blocks {
            let found = get_block_cache(self.start_block_id + block_offset, Arc::clone(device))
                .lock()
                .modify(0, |bits: &mut BitmapBlock| {
                    // 先找有空位的那一组，再在组里找具体哪一位。
                    // 两级查找把大部分工作变成了 u64 比较。
                    let group = bits.iter().position(|g| *g != u64::MAX)?;
                    let bit = bits[group].trailing_ones() as usize;
                    bits[group] |= 1u64 << bit;
                    Some(block_offset * BITS_PER_BLOCK + group * BITS_PER_U64 + bit)
                });
            if found.is_some() {
                return found;
            }
        }
        None
    }

    /// 回收编号为 `bit` 的对象。
    pub fn dealloc(&self, device: &Arc<dyn BlockDevice>, bit: usize) {
        assert!(bit < self.maximum(), "回收了一个越界的编号 {bit}");
        let (block_offset, group, idx) = decompose(bit);

        get_block_cache(self.start_block_id + block_offset, Arc::clone(device))
            .lock()
            .modify(0, |bits: &mut BitmapBlock| {
                // 回收一个本来就空闲的对象，说明上层记账错了。
                // 放过去的话会造成"同一个块被分配两次"，数据直接互相覆盖。
                assert!(
                    bits[group] & (1u64 << idx) != 0,
                    "重复回收编号 {bit}（double free）"
                );
                bits[group] &= !(1u64 << idx);
            });
    }

    /// 已经分配出去多少个。只用于统计和测试。
    pub fn allocated(&self, device: &Arc<dyn BlockDevice>) -> usize {
        (0..self.blocks)
            .map(|offset| {
                get_block_cache(self.start_block_id + offset, Arc::clone(device))
                    .lock()
                    .read(0, |bits: &BitmapBlock| {
                        bits.iter().map(|g| g.count_ones() as usize).sum::<usize>()
                    })
            })
            .sum()
    }
}

/// 把编号拆成 `(第几块, 第几个 u64, 第几位)`。
fn decompose(bit: usize) -> (usize, usize, usize) {
    let block_offset = bit / BITS_PER_BLOCK;
    let within_block = bit % BITS_PER_BLOCK;
    (
        block_offset,
        within_block / BITS_PER_U64,
        within_block % BITS_PER_U64,
    )
}

const _: () = assert!(U64_PER_BLOCK * BITS_PER_U64 == BITS_PER_BLOCK);

//! 内核堆分配器：伙伴系统 (buddy system)。
//!
//! # 为什么是伙伴系统
//!
//! 内核需要一个能反复分配/释放任意大小、且不会随时间推移碎片化到不可用的
//! 分配器。伙伴系统用一个极简的规则做到了这点：**所有块的大小都是 2 的幂，
//! 一个块只能和它唯一的"伙伴"合并**。
//!
//! 而伙伴的地址有一个漂亮的性质 —— 把自己地址的第 `k` 位翻转就是伙伴的地址：
//!
//! ```text
//!   块大小 2^k = 16，块 A 在 0x1230，块 B 在 0x1240
//!   0x1230 ^ 0x10 = 0x1220   ← A 的伙伴（不是 B！）
//!   0x1240 ^ 0x10 = 0x1250   ← B 的伙伴
//! ```
//!
//! 一次异或就能算出伙伴在哪，完全不需要额外记账。代价是内部碎片：
//! 申请 33 字节会实打实占掉 64 字节。对内核元数据这种分配模式，这个代价划算。
//!
//! # 数据结构
//!
//! `free_list[k]` 是所有大小为 `2^k` 的空闲块组成的链表。
//! 分配就是"找一个够大的块，一路劈到需要的大小"；
//! 释放就是"挂回去，然后尽可能和伙伴合并成更大的块"。

use crate::config::KERNEL_HEAP_SIZE;
use crate::sync::UPSafeCell;
use core::alloc::{GlobalAlloc, Layout};
use core::cmp::{max, min};
use core::mem::size_of;
use core::ptr;

/// 支持的最大块大小为 `2^(MAX_ORDER-1)` 字节。
/// 32 意味着单次最大分配 2 GiB，对内核堆绰绰有余。
const MAX_ORDER: usize = 32;

// ---------------------------------------------------------------------------
// 侵入式空闲链表
// ---------------------------------------------------------------------------

/// 空闲块链表。
///
/// **关键技巧：next 指针就存在空闲块自己的内存里。**
/// 空闲块反正没人用，借它开头的 8 个字节存链表指针，分配器本身就不需要
/// 任何额外的元数据空间了 —— 这也是为什么最小块必须 ≥ `size_of::<usize>()`。
struct FreeList {
    head: *mut usize,
}

impl FreeList {
    const fn new() -> Self {
        Self {
            head: ptr::null_mut(),
        }
    }

    fn is_empty(&self) -> bool {
        self.head.is_null()
    }

    /// 把一个块挂到链表头。
    ///
    /// # Safety
    ///
    /// `block` 必须指向一块至少 `size_of::<usize>()` 字节、当前无人使用的内存。
    unsafe fn push(&mut self, block: *mut usize) {
        *block = self.head as usize;
        self.head = block;
    }

    /// 从链表头摘一个块下来。
    ///
    /// # Safety
    ///
    /// 链表中的每个块都必须仍然有效（没有被别处释放或改写）。
    unsafe fn pop(&mut self) -> Option<*mut usize> {
        if self.head.is_null() {
            return None;
        }
        let block = self.head;
        self.head = *block as *mut usize;
        Some(block)
    }

    /// 从链表中摘掉**指定**的块。合并伙伴时用来检查"伙伴是否空闲"。
    ///
    /// 返回是否找到。这里用"指向 next 槽位的指针"来遍历，就不必为
    /// 头节点单独写一个分支。
    ///
    /// # Safety
    ///
    /// 同 [`FreeList::pop`]。
    unsafe fn remove(&mut self, target: *mut usize) -> bool {
        let mut slot: *mut *mut usize = &mut self.head;
        while !(*slot).is_null() {
            let block = *slot;
            if block == target {
                *slot = *block as *mut usize;
                return true;
            }
            // 下一个候选槽位，就是当前块里存放 next 指针的那 8 个字节
            slot = block as *mut *mut usize;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// 伙伴分配器
// ---------------------------------------------------------------------------

pub struct BuddyAllocator {
    /// `free_list[k]` 保存所有大小为 `2^k` 字节的空闲块
    free_list: [FreeList; MAX_ORDER],
    /// 已分配的字节数，按**实际占用的块大小**计（含内部碎片）
    allocated: usize,
    /// 交给分配器管理的总字节数
    total: usize,
}

impl BuddyAllocator {
    pub const fn empty() -> Self {
        Self {
            free_list: [const { FreeList::new() }; MAX_ORDER],
            allocated: 0,
            total: 0,
        }
    }

    /// 把 `[start, end)` 这段内存交给分配器管理。
    ///
    /// 一段任意的内存不见得能正好切成若干 2 的幂次块，所以这里贪心地切：
    /// 每次取"当前地址的对齐能力"和"剩余空间"里较小的那个 2 的幂。
    ///
    /// 之所以受地址对齐限制，是因为伙伴系统要求大小为 `2^k` 的块必须
    /// `2^k` 对齐 —— 否则异或算出的"伙伴地址"就不再落在块边界上。
    ///
    /// # Safety
    ///
    /// `[start, end)` 必须是一段当前完全空闲、且在分配器生命周期内
    /// 一直有效的内存。
    unsafe fn add_to_heap(&mut self, start: usize, end: usize) {
        let mut current = align_up(start, size_of::<usize>());
        let end = align_down(end, size_of::<usize>());

        while current + size_of::<usize>() <= end {
            // 当前地址末尾有几个 0，就最多能对齐到 2 的几次方
            let align_order = current.trailing_zeros().min(MAX_ORDER as u32 - 1);
            let align_limit = 1usize << align_order;
            let space_limit = prev_power_of_two(end - current);
            let size = min(align_limit, space_limit);

            self.free_list[size.trailing_zeros() as usize].push(current as *mut usize);
            current += size;
            self.total += size;
        }
    }

    /// 一次分配请求实际占用的块大小。
    ///
    /// 三个下界取最大：请求的字节数、请求的对齐、以及空闲链表指针需要的
    /// 8 字节。再向上取到 2 的幂 —— 伙伴系统只认 2 的幂。
    fn block_size(layout: &Layout) -> usize {
        max(
            layout.size().next_power_of_two(),
            max(layout.align(), size_of::<usize>()),
        )
    }

    /// 分配。失败返回空指针（由 `GlobalAlloc` 约定）。
    ///
    /// # Safety
    ///
    /// 由 [`GlobalAlloc`] 的契约保证。
    unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let size = Self::block_size(&layout);
        let class = size.trailing_zeros() as usize;

        // 从 class 往上找第一个非空的空闲链表
        let Some(mut order) = (class..MAX_ORDER).find(|&i| !self.free_list[i].is_empty()) else {
            return ptr::null_mut(); // 堆耗尽
        };

        // 自顶向下对半劈，直到劈出要的大小。
        // 每劈一次，两个半块互为伙伴，都挂进下一级链表。
        while order > class {
            let block = self.free_list[order]
                .pop()
                .expect("刚判定非空的链表却弹不出块");
            let half = 1usize << (order - 1);
            let buddy = (block as usize + half) as *mut usize;
            self.free_list[order - 1].push(buddy);
            self.free_list[order - 1].push(block);
            order -= 1;
        }

        let block = self.free_list[class].pop().expect("劈完之后本级必然有块");
        self.allocated += size;
        block as *mut u8
    }

    /// 释放，并尽可能与伙伴合并。
    ///
    /// # Safety
    ///
    /// `ptr` 必须来自本分配器的 [`BuddyAllocator::alloc`]，且 `layout`
    /// 与当初分配时一致 —— 否则算出的块大小不对，会把堆结构彻底搞坏。
    unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        let size = Self::block_size(&layout);
        let mut class = size.trailing_zeros() as usize;
        let mut block = ptr as usize;

        // 只要伙伴也空闲，就合并成更大的块，然后接着往上试。
        // `remove` 返回 true 既表示"伙伴是空闲的"，也顺手把它摘下来了。
        while class + 1 < MAX_ORDER {
            let buddy = block ^ (1usize << class);
            if self.free_list[class].remove(buddy as *mut usize) {
                block = min(block, buddy); // 合并后的块从两者中较低的地址开始
                class += 1;
            } else {
                break;
            }
        }

        self.free_list[class].push(block as *mut usize);
        self.allocated -= size;
    }
}

// ---------------------------------------------------------------------------
// 位运算小工具
// ---------------------------------------------------------------------------

/// 向上对齐。`align` 必须是 2 的幂。
///
/// `!(align - 1)` 是一个低 `log2(align)` 位为 0 的掩码；先加上 `align - 1`
/// 再抹掉低位，就等于"不小于 addr 的最小对齐地址"。
const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

/// 向下对齐。`align` 必须是 2 的幂。
const fn align_down(addr: usize, align: usize) -> usize {
    addr & !(align - 1)
}

/// 不大于 `n` 的最大 2 的幂。
///
/// 前导零的个数决定了最高位 1 在哪。`n` 必须大于 0 —— 调用点由
/// `current + size_of::<usize>() <= end` 的循环条件保证。
const fn prev_power_of_two(n: usize) -> usize {
    1usize << (usize::BITS - 1 - n.leading_zeros())
}

// ---------------------------------------------------------------------------
// 接入 Rust 的 alloc
// ---------------------------------------------------------------------------

/// 全局分配器。挂上 `#[global_allocator]` 之后，`alloc` crate 里的
/// `Box` / `Vec` / `String` / `BTreeMap` 就全都能用了。
struct LockedHeap(UPSafeCell<BuddyAllocator>);

unsafe impl GlobalAlloc for LockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0.exclusive_access().alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.0.exclusive_access().dealloc(ptr, layout)
    }
}

#[global_allocator]
// SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
// 分配器本身在内核态被调用，而内核不可抢占（sstatus.SIE = 0），
// 所以不会出现"分配到一半被打断、然后又来一次分配"的重入。
static HEAP_ALLOCATOR: LockedHeap = LockedHeap(unsafe { UPSafeCell::new(BuddyAllocator::empty()) });

/// 内核堆空间。
///
/// 是个静态数组，所以实际躺在 `.bss` 段里 —— 由 `rust_main` 开头的
/// `clear_bss` 负责清零。这也意味着内核镜像加载后会立刻占掉
/// [`KERNEL_HEAP_SIZE`] 这么多物理内存。
static mut HEAP_SPACE: [u8; KERNEL_HEAP_SIZE] = [0; KERNEL_HEAP_SIZE];

/// 堆的地址范围。
fn heap_range() -> core::ops::Range<usize> {
    let start = &raw const HEAP_SPACE as usize;
    start..start + KERNEL_HEAP_SIZE
}

/// 当前堆用量：`(已分配字节, 总容量)`。
///
/// 注意函数返回时借用就释放了 —— 如果把 `exclusive_access()` 的结果
/// 一直握在手里再去分配内存，`RefCell` 会立刻 panic。
pub fn stats() -> (usize, usize) {
    let allocator = HEAP_ALLOCATOR.0.exclusive_access();
    (allocator.allocated, allocator.total)
}

pub fn init() {
    let range = heap_range();
    // SAFETY: HEAP_SPACE 是一块静态分配、生命周期贯穿整个内核、
    // 且除分配器外无人访问的内存。
    unsafe {
        HEAP_ALLOCATOR
            .0
            .exclusive_access()
            .add_to_heap(range.start, range.end);
    }
    let (_, total) = stats();
    info!(
        "内核堆就绪：[{:#012x}, {:#012x})  {} KiB",
        range.start,
        range.end,
        total / 1024
    );
}

// ---------------------------------------------------------------------------
// 自检
// ---------------------------------------------------------------------------

/// 堆自检。
///
/// 内核没有测试框架（`cargo test` 需要 std），所以正确性验证只能做成
/// 开机自检。这个函数验证三件事：分配到的地址确实在我们划的堆里、
/// 各种容器能正常工作、以及**全部释放后用量精确回到基线**（没有泄漏）。
pub fn heap_test() {
    use alloc::boxed::Box;
    use alloc::collections::BTreeMap;
    use alloc::string::String;
    use alloc::vec::Vec;

    let range = heap_range();
    let (baseline, total) = stats();

    // --- 内部碎片：申请 33 字节，实际会占掉 64 ---
    let tiny: Vec<u8> = Vec::with_capacity(33);
    let (used, _) = stats();
    debug!(
        "申请 33 字节 → 实际占用 {} 字节（伙伴系统只认 2 的幂）",
        used - baseline
    );
    drop(tiny);

    // --- Box：验证分配到的地址确实落在堆内 ---
    let boxed = Box::new(0x5A5Au32);
    assert_eq!(*boxed, 0x5A5A);
    let addr = boxed.as_ref() as *const u32 as usize;
    assert!(range.contains(&addr), "Box 分配到了堆外：{:#x}", addr);
    debug!("Box<u32>      @ {:#012x}", addr);

    // --- Vec：500 次 push 会触发多轮扩容（分配新块 + 拷贝 + 释放旧块），
    //     正好压一压分配器的劈开/合并路径 ---
    let mut v: Vec<usize> = Vec::new();
    for i in 0..500 {
        v.push(i);
    }
    for (i, val) in v.iter().enumerate() {
        assert_eq!(i, *val, "Vec 内容被写坏了");
    }
    let vaddr = v.as_ptr() as usize;
    assert!(range.contains(&vaddr));
    debug!("Vec<usize>×500 @ {:#012x}", vaddr);

    // --- String / BTreeMap：验证 alloc crate 的容器整体可用 ---
    let mut s = String::from("CCNUOS");
    s.push_str(" 第二周");
    assert_eq!(s, "CCNUOS 第二周");

    let mut map = BTreeMap::new();
    for i in 0..100u32 {
        map.insert(i, i * i);
    }
    assert_eq!(map[&99], 99 * 99);
    debug!("BTreeMap 100 项，map[&99] = {}", map[&99]);

    let (peak, _) = stats();

    drop(boxed);
    drop(v);
    drop(s);
    drop(map);

    // --- 泄漏检查：全部释放后必须精确回到基线 ---
    let (after, _) = stats();
    assert_eq!(
        after,
        baseline,
        "内存泄漏：释放后仍占用 {} 字节",
        after - baseline
    );

    info!(
        "堆自检通过：容量 {} KiB，峰值占用 {} B，释放后回落至 {} B",
        total / 1024,
        peak,
        after
    );
}

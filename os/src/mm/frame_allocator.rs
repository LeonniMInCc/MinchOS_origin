//! 物理页帧分配器。
//!
//! # 和第 2 章的内核堆是两回事
//!
//! 这一点容易混淆，先分清楚：
//!
//! | | 内核堆（第 2 章） | 页帧分配器（本章） |
//! |---|---|---|
//! | 管什么 | 内核 `.bss` 里那 3 MiB | `[ekernel, MEMORY_END)` 的全部物理内存 |
//! | 分配单位 | 任意字节数 | 固定 4 KiB 一页 |
//! | 给谁用 | 内核自己的数据结构 | 用户进程的内存、页表本身 |
//! | 算法 | 伙伴系统 | 栈式 |
//!
//! 两者在地址空间里互不重叠。页帧分配器的元数据（那个 `recycled` 向量）
//! 倒是住在内核堆上 —— 所以初始化顺序必须是**先堆后页帧**。
//!
//! # 为什么这次用栈式而不是伙伴系统
//!
//! 页帧全都是固定的 4 KiB，不存在"要分配 33 字节"这种需求，
//! 也就不存在碎片问题。那伙伴系统那套劈开/合并的机制就纯属多余了。
//!
//! **数据结构的复杂度应该由需求决定，而不是由"我会写什么"决定。**

use super::address::PhysPageNum;
use crate::config::MEMORY_END;
use crate::sync::UPSafeCell;
use alloc::vec::Vec;

/// 一个物理页帧的所有权凭证。
///
/// # RAII：让编译器替我们回收内存
///
/// 页帧泄漏是内核里最烦人的一类 bug：不会立刻出错，只是可用内存
/// 一点点变少，跑几个小时后突然分配失败，此时现场早已和起因无关。
///
/// 所以这里不提供"裸的 `alloc` / `free` 一对函数"，而是让 `alloc`
/// 返回一个带 [`Drop`] 的凭证。**持有它就等于持有那一页，
/// 它被丢弃时那一页自动归还** —— 忘记释放在语法上就不可能发生。
///
/// 第 7 周的 `MemorySet` 会把一大批 `FrameTracker` 收进 `Vec`，
/// 于是"销毁一个地址空间"就自动变成了"归还它占用的全部物理页"。
pub struct FrameTracker {
    pub ppn: PhysPageNum,
}

impl FrameTracker {
    fn new(ppn: PhysPageNum) -> Self {
        // 新分配的页可能残留着上一个使用者的数据。不清零的话，
        // 用户进程就能读到别的进程刚释放的内存 —— 一个经典的信息泄露漏洞。
        ppn.get_bytes_array().fill(0);
        Self { ppn }
    }
}

impl Drop for FrameTracker {
    fn drop(&mut self) {
        frame_dealloc(self.ppn);
    }
}

impl core::fmt::Debug for FrameTracker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "FrameTracker({:?})", self.ppn)
    }
}

/// 栈式页帧分配器。
///
/// ```text
///   [ ......已分配过并已回收...... ][ current ][ 从未分配过 ][ end )
///            ↑ recycled 里记着这些
/// ```
///
/// - 分配：先看 `recycled` 有没有回收来的，没有就从 `current` 往后取一个新的
/// - 回收：压进 `recycled`
///
/// 优先复用回收的页，是为了让 `current` 增长得尽量慢 —— 这样
/// "从未分配过"的那一段就能一直保持连续，将来要分配大块连续内存时更容易。
struct StackFrameAllocator {
    /// 管辖区间的起点，只用来统计
    start: usize,
    /// 下一个从未被分配过的页号
    current: usize,
    /// 可用物理内存的上界（不含）
    end: usize,
    /// 已回收、可立即复用的页号
    recycled: Vec<usize>,
}

impl StackFrameAllocator {
    const fn empty() -> Self {
        Self {
            start: 0,
            current: 0,
            end: 0,
            recycled: Vec::new(),
        }
    }

    fn init(&mut self, start: PhysPageNum, end: PhysPageNum) {
        assert!(start.0 < end.0, "可用物理内存区间为空");
        self.start = start.0;
        self.current = start.0;
        self.end = end.0;
    }

    fn alloc(&mut self) -> Option<PhysPageNum> {
        if let Some(ppn) = self.recycled.pop() {
            return Some(ppn.into());
        }
        if self.current == self.end {
            return None; // 物理内存耗尽
        }
        let ppn = self.current;
        self.current += 1;
        Some(ppn.into())
    }

    fn dealloc(&mut self, ppn: PhysPageNum) {
        let n = ppn.0;
        // 两种情况都说明调用方搞错了，而且都是"double free"家族的错误。
        // 与其让它悄悄污染分配器状态，不如立刻崩在现场。
        assert!(
            n >= self.start && n < self.current,
            "回收了一个从未分配过的页 {:?}",
            ppn
        );
        assert!(
            !self.recycled.contains(&n),
            "重复回收同一个页 {:?}（double free）",
            ppn
        );
        self.recycled.push(n);
    }

    /// 当前正被占用的页数。
    fn allocated(&self) -> usize {
        (self.current - self.start) - self.recycled.len()
    }

    /// 管辖的总页数。
    fn total(&self) -> usize {
        self.end - self.start
    }
}

static FRAME_ALLOCATOR: UPSafeCell<StackFrameAllocator> =
    // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
    unsafe { UPSafeCell::new(StackFrameAllocator::empty()) };

/// 初始化：把 `[ekernel, MEMORY_END)` 这一段物理内存交给分配器。
pub fn init() {
    extern "C" {
        static ekernel: u8;
    }
    // 起点是内核镜像的终点（向上取整到页边界）。
    //
    // 第 6–7 周这里还要额外避开"用户程序加载区"，因为那时程序直接躺在
    // 固定的物理地址上。第 8 周有了地址空间之后，用户程序的每一页都是
    // 从这个分配器里要的，不再有任何预留区域 —— 这个函数因此变简单了。
    let kernel_end = &raw const ekernel as usize;
    let start = super::address::PhysAddr::from(kernel_end).ceil();
    let end = super::address::PhysAddr::from(MEMORY_END).floor();

    FRAME_ALLOCATOR.exclusive_access().init(start, end);

    let (allocated, total) = stats();
    info!(
        "物理页帧分配器就绪：{:?} .. {:?}，共 {} 页（{} MiB），已用 {}",
        start,
        end,
        total,
        total * 4 / 1024,
        allocated
    );
}

/// 分配一个物理页帧。返回的凭证被丢弃时自动归还。
pub fn frame_alloc() -> Option<FrameTracker> {
    FRAME_ALLOCATOR
        .exclusive_access()
        .alloc()
        .map(FrameTracker::new)
}

/// 归还一个物理页帧。只由 [`FrameTracker::drop`] 调用。
fn frame_dealloc(ppn: PhysPageNum) {
    FRAME_ALLOCATOR.exclusive_access().dealloc(ppn);
}

/// `(已分配页数, 总页数)`。
pub fn stats() -> (usize, usize) {
    let a = FRAME_ALLOCATOR.exclusive_access();
    (a.allocated(), a.total())
}

/// 页帧分配器自检。
pub fn frame_test() {
    let (baseline, total) = stats();

    // --- 分配若干页，页号必须互不相同 ---
    let mut frames: Vec<FrameTracker> = Vec::new();
    for _ in 0..5 {
        frames.push(frame_alloc().expect("分配页帧失败"));
    }
    assert_eq!(stats().0 - baseline, 5, "分配计数不对");
    for i in 0..frames.len() {
        for j in (i + 1)..frames.len() {
            assert_ne!(frames[i].ppn, frames[j].ppn, "同一个页帧被分配了两次");
        }
    }

    // --- 新分配的页必须是干净的 ---
    assert!(
        frames[0].ppn.get_bytes_array().iter().all(|&b| b == 0),
        "新分配的页帧没有清零"
    );

    // --- 把一页写脏，回收，再分配 ---
    // 应该拿回同一页（栈式分配器优先复用），而且它必须已被清零 ——
    // 否则新进程就能读到上一个进程刚释放的内存。
    let dirty = frames[4].ppn;
    frames[4].ppn.get_bytes_array().fill(0xAB);
    frames.pop(); // 触发 Drop → 归还

    let reused = frame_alloc().expect("分配页帧失败");
    assert_eq!(reused.ppn, dirty, "回收的页帧没有被优先复用");
    assert!(
        reused.ppn.get_bytes_array().iter().all(|&b| b == 0),
        "复用的页帧残留了上一个使用者的数据 —— 这是信息泄露"
    );
    debug!("页帧 {:?} 回收后被复用，且已清零", dirty);
    drop(reused);

    // --- 泄漏检查 ---
    drop(frames);
    let (after, _) = stats();
    assert_eq!(after, baseline, "页帧泄漏：{} 页没有归还", after - baseline);

    info!(
        "页帧自检通过：容量 {} 页（{} MiB），分配/复用/清零/回收均正确",
        total,
        total * 4 / 1024
    );
}

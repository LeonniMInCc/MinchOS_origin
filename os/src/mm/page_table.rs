//! SV39 多级页表。
//!
//! # 一次地址翻译是怎么走的
//!
//! ```text
//!   虚拟地址 0x1234_5678
//!   ┌────────┬────────┬────────┬──────────┐
//!   │ VPN[2] │ VPN[1] │ VPN[0] │  偏移     │
//!   └───┬────┴───┬────┴───┬────┴────┬─────┘
//!       │        │        │         │
//!  satp─┴→[根页表]│        │         │
//!          └ PTE ─┴→[一级页表]       │
//!                    └ PTE ─┴→[二级页表]
//!                              └ PTE ─→ 物理页号 ──┐
//!                                                 ├→ 物理地址
//!                                          偏移 ──┘
//! ```
//!
//! 每一级页表都是**整整一个物理页**：512 项 × 8 字节 = 4096 字节。
//! 这不是巧合 —— SV39 特意把 VPN 切成 9 位一段，就是为了让每级页表
//! 正好占一页，从而可以用页帧分配器来分配页表本身。
//!
//! # 页表项 (PTE) 的位布局
//!
//! ```text
//!   63      54 53                    10 9   8 7 6 5 4 3 2 1 0
//!   ┌─────────┬────────────────────────┬─────┬─┬─┬─┬─┬─┬─┬─┬─┐
//!   │ 保留     │      物理页号 (44 位)   │ RSW │D│A│G│U│X│W│R│V│
//!   └─────────┴────────────────────────┴─────┴─┴─┴─┴─┴─┴─┴─┴─┘
//! ```

use super::address::{PhysAddr, PhysPageNum, VirtAddr, VirtPageNum};
use super::frame_allocator::{frame_alloc, FrameTracker};
use crate::config::PAGE_SIZE;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::{self, Debug, Formatter};
use core::ops::BitOr;

/// 页表项的标志位。
///
/// 手写而不是用 `bitflags` crate —— 八个位而已，不值得为它引入依赖。
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct PTEFlags(u8);

impl PTEFlags {
    /// Valid：这一项有效。为 0 时整项无意义，访问会触发缺页异常。
    pub const V: Self = Self(1 << 0);
    /// Readable
    pub const R: Self = Self(1 << 1);
    /// Writable
    pub const W: Self = Self(1 << 2);
    /// eXecutable
    pub const X: Self = Self(1 << 3);
    /// User：U 态能否访问。**内核的页必须不带 U，用户的页必须带 U** ——
    /// 这一位就是内存隔离的硬件开关。
    pub const U: Self = Self(1 << 4);
    /// Global：所有地址空间共享，换页表时不必刷 TLB。
    pub const G: Self = Self(1 << 5);
    /// Accessed：被访问过。硬件自动置位，用于页面置换算法。
    pub const A: Self = Self(1 << 6);
    /// Dirty：被写过。硬件自动置位，用于判断换出时要不要写回。
    pub const D: Self = Self(1 << 7);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// 是否包含 `other` 的全部位。
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// 是否不含任何 R/W/X —— 这样的项是**指向下一级页表的指针**，
    /// 而不是最终的物理页映射。SV39 用这个约定区分两者。
    pub const fn is_directory(self) -> bool {
        self.0 & (Self::R.0 | Self::W.0 | Self::X.0) == 0
    }
}

impl BitOr for PTEFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl Debug for PTEFlags {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // 按 RISC-V 手册的顺序打印，没置位的用 '-' 占位，便于对齐比较
        for (bit, ch) in [
            (Self::D, 'D'),
            (Self::A, 'A'),
            (Self::G, 'G'),
            (Self::U, 'U'),
            (Self::X, 'X'),
            (Self::W, 'W'),
            (Self::R, 'R'),
            (Self::V, 'V'),
        ] {
            write!(f, "{}", if self.contains(bit) { ch } else { '-' })?;
        }
        Ok(())
    }
}

/// 一个页表项。
#[derive(Copy, Clone, Default)]
#[repr(C)]
pub struct PageTableEntry {
    pub bits: usize,
}

impl PageTableEntry {
    pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
        Self {
            bits: (ppn.0 << 10) | flags.bits() as usize,
        }
    }

    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    /// 取出物理页号：第 10~53 位，共 44 位。
    pub fn ppn(&self) -> PhysPageNum {
        ((self.bits >> 10) & ((1usize << 44) - 1)).into()
    }

    pub fn flags(&self) -> PTEFlags {
        PTEFlags::from_bits(self.bits as u8)
    }

    pub fn is_valid(&self) -> bool {
        self.flags().contains(PTEFlags::V)
    }
}

impl Debug for PageTableEntry {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "PTE({:?} {:?})", self.ppn(), self.flags())
    }
}

/// 一棵 SV39 三级页表。
pub struct PageTable {
    root_ppn: PhysPageNum,
    /// 这棵树占用的所有物理页帧（含根页表）。
    ///
    /// 靠 RAII 保证：`PageTable` 被丢弃时，它撑起的整棵页表树
    /// 占用的物理页全部自动归还。页表本身也是要占内存的，
    /// 这一点初学时很容易忽略。
    frames: Vec<FrameTracker>,
}

impl PageTable {
    /// 新建一棵空页表。会立刻消耗一个物理页帧作为根页表。
    pub fn new() -> Self {
        let root = frame_alloc().expect("没有物理页帧可用于新建页表");
        Self {
            root_ppn: root.ppn,
            frames: alloc::vec![root],
        }
    }

    /// 走到 `vpn` 对应的**叶子**页表项，路上缺哪级就建哪级。
    ///
    /// 返回 `None` 只在物理内存耗尽时发生。
    fn find_pte_create(&mut self, vpn: VirtPageNum) -> Option<&'static mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;

        // 前两级是"目录"，缺了就现建
        for &idx in &idxs[..2] {
            let pte = ppn.pte_at(idx);
            if !pte.is_valid() {
                let frame = frame_alloc()?;
                // 中间级的 PTE 只带 V，不带 R/W/X ——
                // SV39 用"没有 R/W/X"来表示"这一项指向下一级页表"。
                // 带上 R/W/X 的话它就变成了一个大页映射，语义完全不同。
                *pte = PageTableEntry::new(frame.ppn, PTEFlags::V);
                self.frames.push(frame);
            }
            ppn = pte.ppn();
        }

        // 第三级是叶子，交给调用方去写
        Some(ppn.pte_at(idxs[2]))
    }

    /// 走到 `vpn` 对应的叶子页表项，路上缺了就返回 `None`（不创建）。
    fn find_pte(&self, vpn: VirtPageNum) -> Option<&'static mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;

        for &idx in &idxs[..2] {
            let pte = ppn.pte_at(idx);
            if !pte.is_valid() {
                return None;
            }
            ppn = pte.ppn();
        }

        Some(ppn.pte_at(idxs[2]))
    }

    /// 建立映射 `vpn → ppn`。
    ///
    /// 重复映射同一个 `vpn` 是逻辑错误，直接 panic —— 这种错误
    /// 如果放过去，表现是"某个页神秘地指向了别的地方"，极难定位。
    pub fn map(&mut self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        let pte = self
            .find_pte_create(vpn)
            .expect("物理内存耗尽，无法建立映射");
        assert!(!pte.is_valid(), "{:?} 已经被映射过了", vpn);
        *pte = PageTableEntry::new(ppn, flags | PTEFlags::V);
    }

    /// 解除映射。
    ///
    /// 注意这里只清掉叶子项，**不回收中间级页表**。中间级会一直留到
    /// 整棵 `PageTable` 被丢弃。这是个有意的取舍：回收中间级需要
    /// 维护每级的引用计数，复杂度不小，而收益（省几页内存）有限。
    pub fn unmap(&mut self, vpn: VirtPageNum) {
        let pte = self.find_pte(vpn).expect("解除一个不存在的映射");
        assert!(pte.is_valid(), "{:?} 本来就没有映射", vpn);
        *pte = PageTableEntry::empty();
    }

    /// 查询 `vpn` 的映射。没有映射时返回 `None`。
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.find_pte(vpn)
            .filter(|pte| pte.is_valid())
            .map(|pte| *pte)
    }

    /// 这棵页表占用的物理页帧数（含根页表和所有中间级）。
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// 构造写进 `satp` 的值，第 7 周开启分页时用。
    ///
    /// ```text
    ///   63    60 59        44 43            0
    ///   ┌───────┬────────────┬───────────────┐
    ///   │ MODE  │    ASID    │   根页表 PPN   │
    ///   └───────┴────────────┴───────────────┘
    ///     8 = SV39
    /// ```
    pub fn token(&self) -> usize {
        (8usize << 60) | self.root_ppn.0
    }
}

impl Default for PageTable {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// 跨地址空间访问
// ---------------------------------------------------------------------------

impl PageTable {
    /// 由一个 `satp` 值构造出一个**只读句柄**，用来查询别的地址空间。
    ///
    /// 它的 `frames` 是空的 —— 这个句柄不拥有任何页帧，`Drop` 时什么都不做。
    /// 所以它只能用来 `translate`，绝不能用来 `map`（那会让新分配的中间级
    /// 页表无人认领，直接泄漏）。
    pub fn from_token(satp: usize) -> Self {
        Self {
            root_ppn: PhysPageNum::from(satp & ((1usize << 44) - 1)),
            frames: Vec::new(),
        }
    }
}

/// 把用户地址空间里的一段缓冲区，翻译成内核可以直接读写的若干片段。
///
/// # 为什么第 8 周必须有这个函数
///
/// 第 7 周内核和用户共用一个地址空间，内核拿到用户指针可以直接解引用
/// （代价是必须打开 `sstatus.SUM`）。现在两者的地址空间彻底分开了 ——
/// 用户传来的 `0x10234` 在内核页表里根本不存在，解引用只会缺页。
///
/// 所以内核必须**手动走一遍用户的页表**，把虚拟地址翻译成物理地址，
/// 再通过恒等映射去访问。`SUM` 也因此可以关掉了。
///
/// # 返回值为什么是"若干片段"
///
/// 用户的缓冲区在它自己的地址空间里是连续的，但对应的物理页可能散落在
/// 内存各处。所以只能按页切开，返回一组片段。
///
/// # 安全检查
///
/// 返回 `None` 表示这段缓冲区不合法。检查有三条：
///
/// 1. `ptr + len` 不能溢出
/// 2. 每一页都必须在用户页表里**有映射**
/// 3. 每一页都必须带 `U` 位
///
/// 第 3 条是关键。没有它，用户就能把一个内核地址（比如跳板页）传进来，
/// 让内核代自己去读 —— 内核访存是不受 `U` 位约束的。
///
/// 注意这三条检查已经完全取代了前几章那个手写的
/// `is_accessible_by`：不再需要知道任何区间常量，
/// **"用户能不能访问这块内存"这个问题直接问页表就好了。**
pub fn translated_byte_buffer(
    token: usize,
    ptr: usize,
    len: usize,
) -> Option<Vec<&'static mut [u8]>> {
    if len == 0 {
        return Some(Vec::new());
    }
    let end = ptr.checked_add(len)?;

    let page_table = PageTable::from_token(token);
    let mut buffers = Vec::new();
    let mut cur = ptr;

    while cur < end {
        let va = VirtAddr::from(cur);
        let pte = page_table.translate(va.floor())?;
        if !pte.flags().contains(PTEFlags::U | PTEFlags::R) {
            return None; // 没映射、或者是一个用户无权访问的内核页
        }
        let offset = va.page_offset();
        let chunk = (PAGE_SIZE - offset).min(end - cur);
        buffers.push(pte.ppn().bytes_at(offset, chunk));
        cur += chunk;
    }

    Some(buffers)
}

// ---------------------------------------------------------------------------
// 自检
// ---------------------------------------------------------------------------

/// 页表自检。
///
/// 除了验证"映射进去能翻译回来"，这个测试还刻意把**页表自身的空间代价**
/// 量出来了 —— 这是初学时最容易忽略的一点：页表不是免费的，它自己也占内存。
pub fn page_table_test() {
    use super::address::VirtPageNum;
    use super::frame_allocator::{frame_alloc, stats};

    let (baseline, _) = stats();

    // --- 新建一棵空页表：立刻消耗 1 页作为根页表 ---
    let mut pt = PageTable::new();
    assert_eq!(stats().0 - baseline, 1, "新建页表应该只占 1 页（根页表）");
    debug!("PageTable::new()        → 累计占用 {} 页", pt.frame_count());

    // 准备四个目标物理页
    let f0 = frame_alloc().unwrap();
    let f1 = frame_alloc().unwrap();
    let f2 = frame_alloc().unwrap();
    let f3 = frame_alloc().unwrap();
    let mark = stats().0;

    let rw = PTEFlags::R | PTEFlags::W | PTEFlags::U;
    let rx = PTEFlags::R | PTEFlags::X | PTEFlags::U;

    // VPN 0x10000 拆开是 [VPN2=0, VPN1=128, VPN0=0]
    let vpn0 = VirtPageNum::from(0x10000);

    // --- 第一次映射：一路上两级中间页表都得现建 ---
    pt.map(vpn0, f0.ppn, rw);
    assert_eq!(stats().0 - mark, 2, "首次映射应新建 2 级中间页表");
    debug!(
        "map(VPN:0x10000)        → 累计占用 {} 页（+2 中间级）",
        pt.frame_count()
    );

    // --- 相邻的一页：VPN0 变成 1，前两级完全复用，一页都不用加 ---
    pt.map(vpn0.next(), f1.ppn, rw);
    assert_eq!(stats().0 - mark, 2, "相邻页应复用现有页表，不新增页帧");
    debug!(
        "map(VPN:0x10001)        → 累计占用 {} 页（+0，复用）",
        pt.frame_count()
    );

    // --- 隔 512 页：VPN1 变成 129，需要一张新的末级页表 ---
    let vpn_far = VirtPageNum::from(0x10000 + 512);
    pt.map(vpn_far, f2.ppn, rx);
    assert_eq!(stats().0 - mark, 3, "跨越 VPN1 应新建 1 级页表");
    debug!(
        "map(VPN:0x10200)        → 累计占用 {} 页（+1 末级）",
        pt.frame_count()
    );

    // --- 隔 512×512 页：VPN2 变成 1，两级都得新建 ---
    let vpn_farther = VirtPageNum::from(0x10000 + 512 * 512);
    pt.map(vpn_farther, f3.ppn, rw);
    assert_eq!(stats().0 - mark, 5, "跨越 VPN2 应新建 2 级页表");
    debug!(
        "map(VPN:0x50000)        → 累计占用 {} 页（+2）",
        pt.frame_count()
    );

    // --- 翻译回来，地址和权限都要对得上 ---
    for (vpn, ppn, flags, name) in [
        (vpn0, f0.ppn, rw, "vpn0"),
        (vpn0.next(), f1.ppn, rw, "vpn0+1"),
        (vpn_far, f2.ppn, rx, "vpn0+512"),
        (vpn_farther, f3.ppn, rw, "vpn0+512*512"),
    ] {
        let pte = pt
            .translate(vpn)
            .unwrap_or_else(|| panic!("{name} 翻译失败"));
        assert_eq!(pte.ppn(), ppn, "{name} 翻译出的物理页号不对");
        assert!(
            pte.flags().contains(flags | PTEFlags::V),
            "{name} 权限位不对"
        );
    }
    // 没映射过的地址必须翻译失败
    assert!(
        pt.translate(VirtPageNum::from(0x99999)).is_none(),
        "未映射的页不该能翻译"
    );
    debug!("四个映射全部翻译正确，未映射的页正确地翻译失败");

    // --- 解除映射 ---
    pt.unmap(vpn0);
    assert!(pt.translate(vpn0).is_none(), "unmap 之后不该还能翻译");
    // 其他映射不受影响
    assert!(
        pt.translate(vpn0.next()).is_some(),
        "unmap 误伤了相邻的映射"
    );

    // 第 7 周就是把这个值写进 satp，分页即刻生效。
    // 最高 4 位 8 表示 SV39 模式，低 44 位是根页表的物理页号。
    debug!("这棵页表的 satp = {:#x}（MODE=8 即 SV39）", pt.token());

    let leaked_before_drop = stats().0;
    let table_pages = pt.frame_count();

    // --- 丢弃整棵页表：它占用的全部页帧（根 + 中间级）自动归还 ---
    drop(pt);
    assert_eq!(
        leaked_before_drop - stats().0,
        table_pages,
        "丢弃页表后，它占用的页帧没有全部归还"
    );

    drop((f0, f1, f2, f3));
    assert_eq!(stats().0, baseline, "页表测试结束后有页帧泄漏");

    info!(
        "页表自检通过：SV39 三级映射、权限位、解除映射、RAII 回收均正确（该页表共占 {} 页）",
        table_pages
    );
}

/// 从用户地址空间里读出一个以 `\0` 结尾的字符串。
///
/// `sys_exec` 用它取程序名。和 [`translated_byte_buffer`] 一样，
/// 内核不能直接解引用用户指针，只能逐字节翻译。
///
/// 逐字节查页表确实慢，但 `exec` 的路径名很短，而且这条路一年也走不了几次 ——
/// **别为不在热路径上的代码牺牲清晰度。**
///
/// 返回 `None` 表示遇到了不可访问的页（同样要求带 `U`）。
/// 长度上限防的是用户传来一段没有 `\0` 的内存，让内核一直读下去。
pub fn translated_str(token: usize, ptr: usize, max_len: usize) -> Option<String> {
    let page_table = PageTable::from_token(token);
    let mut bytes = Vec::new();
    for i in 0..max_len {
        let va = VirtAddr::from(ptr.checked_add(i)?);
        let pte = page_table.translate(va.floor())?;
        if !pte.flags().contains(PTEFlags::U | PTEFlags::R) {
            return None;
        }
        let byte = pte.ppn().get_bytes_array()[va.page_offset()];
        if byte == 0 {
            return String::from_utf8(bytes).ok();
        }
        bytes.push(byte);
    }
    None // 读满上限还没遇到 \0
}

/// 把用户地址空间里的一个 `T` 借出来给内核写。
///
/// `sys_waitpid` 用它回填退出码。
pub fn translated_refmut<T>(token: usize, ptr: usize) -> Option<&'static mut T> {
    let page_table = PageTable::from_token(token);
    let va = VirtAddr::from(ptr);
    // 只处理不跨页的情况。跨页的话物理地址不连续，不能当成一个 T 来写 ——
    // 与其悄悄写坏，不如明确拒绝。
    if va.page_offset() + core::mem::size_of::<T>() > PAGE_SIZE {
        return None;
    }
    let pte = page_table.translate(va.floor())?;
    if !pte.flags().contains(PTEFlags::U | PTEFlags::W) {
        return None;
    }
    let pa: PhysAddr = pte.ppn().into();
    // SAFETY: 上面已验证该页在用户空间可写、且 T 不跨页；
    // 内核恒等映射了全部物理内存，所以物理地址可以直接当指针用。
    Some(unsafe { &mut *((pa.0 + va.page_offset()) as *mut T) })
}

/// 用户缓冲区：一段在用户地址空间里连续、但物理上分散的内存。
///
/// [`translated_byte_buffer`] 返回的是按页切开的若干片段。直接把
/// `Vec<&mut [u8]>` 传来传去很容易写错下标，所以包一层，
/// 只暴露"总长度"和"逐字节访问"这两件真正需要的事。
pub struct UserBuffer {
    buffers: Vec<&'static mut [u8]>,
}

impl UserBuffer {
    pub fn new(buffers: Vec<&'static mut [u8]>) -> Self {
        Self { buffers }
    }

    pub fn len(&self) -> usize {
        self.buffers.iter().map(|b| b.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 按顺序遍历每个字节（只读）。`write` 用它把用户数据送出去。
    pub fn bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.buffers.iter().flat_map(|b| b.iter().copied())
    }

    /// 按顺序遍历每个字节（可写）。`read` 用它把数据填进用户内存。
    pub fn bytes_mut(&mut self) -> impl Iterator<Item = &mut u8> {
        self.buffers.iter_mut().flat_map(|b| b.iter_mut())
    }
}

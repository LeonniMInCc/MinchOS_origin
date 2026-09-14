//! 地址空间。
//!
//! 前六章里，"地址"只有一种含义：物理地址。从这一章起，内核和用户程序
//! 看到的地址都要经过页表翻译 —— 同一个虚拟地址在不同的地址空间里
//! 可以指向完全不同的物理内存。
//!
//! # 本章的策略：先让语义完全不变
//!
//! 开启分页是个一步到位就很容易翻车的改动。所以这一章刻意保守：
//! **建立一个把所有物理内存恒等映射（虚拟地址 = 物理地址）的内核地址空间**，
//! 然后打开分页。系统的行为应该和第 6 章一模一样 —— 只是所有访存
//! 都多走了一趟 MMU。
//!
//! 这样做有两个好处：
//!
//! 1. 出了问题，一定是在"开启分页"这件事本身上，而不是混在别的改动里
//! 2. 即便如此，我们**已经拿到了真正的硬件保护**：内核的页不带 `U` 位，
//!    用户程序碰它会被 MMU 挡下来。第 6 章之前这是做不到的
//!
//! 第 8 章才会给每个进程建独立的地址空间，那时才需要跳板页。
//!
//! # 本章之后的物理内存布局
//!
//! ```text
//!   0x80200000  内核镜像 (.text/.rodata/.data/.bss，含内核堆与内核栈)
//!   0x80800000  用户程序槽位  16 × 128 KiB          ← 带 U 位
//!   0x80a00000  用户栈        16 × 8 KiB            ← 带 U 位
//!   0x80a20000  空闲物理页帧
//!   0x88000000  内存终点
//! ```

use super::address::{PhysPageNum, VirtAddr, VirtPageNum};
use super::frame_allocator::{frame_alloc, FrameTracker};
use super::page_table::{PTEFlags, PageTable, PageTableEntry};
use crate::config::{
    app_region_end, user_stack_region_end, APP_BASE_ADDRESS, MEMORY_END, PAGE_SIZE,
    USER_STACK_BASE,
};
use crate::csr::{satp, sstatus, sstatus_bits};
use crate::sync::UPSafeCell;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::arch::asm;

// ---------------------------------------------------------------------------
// 虚拟页号区间
// ---------------------------------------------------------------------------

/// 左闭右开的虚拟页号区间 `[start, end)`。
#[derive(Copy, Clone, Debug)]
pub struct VPNRange {
    pub start: VirtPageNum,
    pub end: VirtPageNum,
}

impl VPNRange {
    pub fn new(start: VirtPageNum, end: VirtPageNum) -> Self {
        assert!(start <= end, "非法的页号区间 {:?}..{:?}", start, end);
        Self { start, end }
    }

    pub fn iter(&self) -> impl Iterator<Item = VirtPageNum> {
        (self.start.0..self.end.0).map(VirtPageNum)
    }

    pub fn len(&self) -> usize {
        self.end.0 - self.start.0
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------------------
// 映射权限
// ---------------------------------------------------------------------------

/// 一段内存的访问权限。
///
/// 这是 [`PTEFlags`] 的一个子集 —— 只保留调用方该关心的四位，
/// 把 `V`（有效位，由 `map` 自动加上）和 `A`/`D`（硬件自己维护）藏起来。
/// **位的编号和 `PTEFlags` 一致**，所以转换就是原样拿走。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct MapPermission(u8);

impl MapPermission {
    pub const R: Self = Self(1 << 1);
    pub const W: Self = Self(1 << 2);
    pub const X: Self = Self(1 << 3);
    /// 用户态可访问。**这一位就是内存隔离的硬件开关。**
    pub const U: Self = Self(1 << 4);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for MapPermission {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl From<MapPermission> for PTEFlags {
    fn from(p: MapPermission) -> Self {
        PTEFlags::from_bits(p.0)
    }
}

/// 一段内存怎么落到物理页上。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MapType {
    /// 恒等映射：虚拟页号 = 物理页号。
    ///
    /// 内核用这种方式映射自己和整个物理内存 —— 这样内核代码里
    /// 拿到一个物理地址就能直接解引用，不必先翻译。
    Identical,
    /// 每一页单独从页帧分配器要一张物理页。
    ///
    /// 用户进程的内存用这种方式。第 8 章会大量用到。
    Framed,
}

// ---------------------------------------------------------------------------
// 逻辑段
// ---------------------------------------------------------------------------

/// 一段连续的、权限相同的虚拟内存。
///
/// 地址空间就是若干个这样的段拼起来的。把"段"作为一级抽象，
/// 好处是回收时只要丢掉整个段，它占用的物理页帧就跟着 `FrameTracker`
/// 一起归还了。
pub struct MapArea {
    vpn_range: VPNRange,
    /// `Framed` 模式下每个虚拟页对应的物理页帧凭证。
    /// `Identical` 模式下为空 —— 恒等映射不占用任何新的物理页。
    data_frames: BTreeMap<VirtPageNum, FrameTracker>,
    map_type: MapType,
    map_perm: MapPermission,
}

impl MapArea {
    pub fn new(
        start_va: VirtAddr,
        end_va: VirtAddr,
        map_type: MapType,
        map_perm: MapPermission,
    ) -> Self {
        Self {
            // 起点向下取整、终点向上取整：页是最小粒度，
            // 宁可多映射一点也不能少映射
            vpn_range: VPNRange::new(start_va.floor(), end_va.ceil()),
            data_frames: BTreeMap::new(),
            map_type,
            map_perm,
        }
    }

    fn map_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        let ppn = match self.map_type {
            MapType::Identical => PhysPageNum(vpn.0),
            MapType::Framed => {
                let frame = frame_alloc().expect("物理内存耗尽，无法建立映射");
                let ppn = frame.ppn;
                self.data_frames.insert(vpn, frame);
                ppn
            }
        };
        page_table.map(vpn, ppn, self.map_perm.into());
    }

    pub fn map(&mut self, page_table: &mut PageTable) {
        // 先把区间拷出来：否则 for 循环借着 self，而 map_one 要 &mut self
        let range = self.vpn_range;
        for vpn in range.iter() {
            self.map_one(page_table, vpn);
        }
    }

    /// 把 `data` 按页拷进这个段。只对 [`MapType::Framed`] 有意义。
    ///
    /// `data` 可以比整个段短 —— 多出来的部分保持为 0，那正是 `.bss`。
    pub fn copy_data(&mut self, data: &[u8]) {
        assert_eq!(
            self.map_type,
            MapType::Framed,
            "恒等映射的段不需要拷贝数据，直接写目标地址即可"
        );
        assert!(
            data.len() <= self.vpn_range.len() * PAGE_SIZE,
            "数据 {} 字节，装不进 {} 页",
            data.len(),
            self.vpn_range.len()
        );

        for (i, vpn) in self.vpn_range.iter().enumerate() {
            let start = i * PAGE_SIZE;
            if start >= data.len() {
                break; // 剩下的页保持全 0
            }
            let end = (start + PAGE_SIZE).min(data.len());
            let src = &data[start..end];
            let ppn = self.data_frames[&vpn].ppn;
            ppn.get_bytes_array()[..src.len()].copy_from_slice(src);
        }
    }

    pub fn range(&self) -> VPNRange {
        self.vpn_range
    }

    pub fn permission(&self) -> MapPermission {
        self.map_perm
    }
}

// ---------------------------------------------------------------------------
// 地址空间
// ---------------------------------------------------------------------------

/// 一个地址空间：一棵页表，加上它所包含的若干逻辑段。
pub struct MemorySet {
    page_table: PageTable,
    areas: Vec<MapArea>,
}

impl MemorySet {
    pub fn new_bare() -> Self {
        Self {
            page_table: PageTable::new(),
            areas: Vec::new(),
        }
    }

    /// 加入一个段，并可选地往里拷数据。
    pub fn push(&mut self, mut area: MapArea, data: Option<&[u8]>) {
        // 空段（比如内核里没有任何全局变量时的 .data）直接跳过，
        // 免得在 areas 里留下一堆什么都不映射的条目
        if area.range().is_empty() {
            return;
        }
        area.map(&mut self.page_table);
        if let Some(data) = data {
            area.copy_data(data);
        }
        self.areas.push(area);
    }

    /// 加入一个按页分配的段。第 8 章建用户地址空间时的主力。
    pub fn insert_framed_area(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        perm: MapPermission,
    ) {
        self.push(
            MapArea::new(start_va, end_va, MapType::Framed, perm),
            None,
        );
    }

    pub fn token(&self) -> usize {
        self.page_table.token()
    }

    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.page_table.translate(vpn)
    }

    /// 这个地址空间的页表本身占了多少物理页。
    pub fn page_table_frames(&self) -> usize {
        self.page_table.frame_count()
    }

    /// 构造内核地址空间：把所有物理内存恒等映射进来。
    ///
    /// 每一段的权限都是按它的实际用途给的最小集合：
    ///
    /// | 段 | 权限 | 理由 |
    /// |---|---|---|
    /// | `.text` | `R X` | 代码要能执行，但**不可写** —— 防止内核代码被改 |
    /// | `.rodata` | `R` | 只读数据，写它是 bug |
    /// | `.data` / `.bss` | `R W` | 可读写，但**不可执行** |
    /// | 用户程序区 | `R W X U` | 用户代码，本章还不区分段 |
    /// | 用户栈区 | `R W U` | 栈上不该有可执行代码 |
    /// | 空闲页帧 | `R W` | 内核要能读写它分出去的任何一页 |
    ///
    /// 这已经比第 6 章强了：以前整个物理内存对所有人都是 RWX。
    pub fn new_kernel() -> Self {
        extern "C" {
            static stext: u8;
            static etext: u8;
            static srodata: u8;
            static erodata: u8;
            static sdata: u8;
            static edata: u8;
            static sbss_with_stack: u8;
            static ebss: u8;
            static ekernel: u8;
        }
        /// 取链接符号的地址。`&raw const` 只取址不解引用，所以不需要 `unsafe`。
        macro_rules! sym {
            ($s:ident) => {
                &raw const $s as usize
            };
        }

        let mut set = Self::new_bare();
        let (r, w, x, u) = (
            MapPermission::R,
            MapPermission::W,
            MapPermission::X,
            MapPermission::U,
        );

        let mut add = |name: &str, start: usize, end: usize, perm: MapPermission| {
            if start >= end {
                return; // 空段（比如没有任何全局变量时的 .data）
            }
            debug!(
                "  内核空间 {:<12} [{:#012x}, {:#012x})  {:?}",
                name, start, end, perm
            );
            set.push(
                MapArea::new(start.into(), end.into(), MapType::Identical, perm),
                None,
            );
        };

        add(".text", sym!(stext), sym!(etext), r | x);
        add(".rodata", sym!(srodata), sym!(erodata), r);
        add(".data", sym!(sdata), sym!(edata), r | w);
        // 注意用 sbss_with_stack 而不是 sbss：启动栈也在 .bss 里，
        // 不映射它的话，开启分页后第一次压栈就会缺页
        add(".bss+stack", sym!(sbss_with_stack), sym!(ebss), r | w);
        add("空洞", sym!(ekernel), APP_BASE_ADDRESS, r | w);
        add("用户程序区", APP_BASE_ADDRESS, app_region_end(), r | w | x | u);
        add("用户栈区", USER_STACK_BASE, user_stack_region_end(), r | w | u);
        add("空闲页帧", user_stack_region_end(), MEMORY_END, r | w);

        set
    }

    /// 让这个地址空间生效。
    ///
    /// # Safety
    ///
    /// **这是整个内核里最危险的一个函数。**
    ///
    /// `csrw satp` 执行完之后，取下一条指令就要走新页表了。如果新页表里
    /// 没有"当前正在执行的这段代码"，CPU 会立刻取指失败 —— 而且此时
    /// 连 trap 处理程序都可能取不到，直接原地打转。
    ///
    /// 调用方必须保证：当前的 `pc`、`sp`、以及接下来要访问的一切，
    /// 在新地址空间里都有正确的映射。对内核地址空间来说，这由
    /// [`MemorySet::new_kernel`] 的恒等映射保证。
    pub unsafe fn activate(&self) {
        let token = self.token();
        satp::write(token);
        // 换页表之后必须刷 TLB。TLB 里缓存着旧页表的翻译结果，
        // 不刷的话 CPU 可能继续用旧映射，行为完全不可预测。
        asm!("sfence.vma", options(nostack));
    }
}

// ---------------------------------------------------------------------------
// 全局的内核地址空间
// ---------------------------------------------------------------------------

static KERNEL_SPACE: UPSafeCell<Option<MemorySet>> =
    // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
    unsafe { UPSafeCell::new(None) };

/// 建立并激活内核地址空间。**此后所有访存都要经过 MMU。**
pub fn init_kernel_space() {
    info!("建立内核地址空间（全部恒等映射）：");
    let space = MemorySet::new_kernel();

    let token = space.token();
    let frames = space.page_table_frames();
    let areas = space.areas.len();

    // 先存起来再激活：MemorySet 一旦被丢弃，它的 FrameTracker 就会
    // 归还页表占用的物理页 —— 而 satp 还指着那些页。
    *KERNEL_SPACE.exclusive_access() = Some(space);

    // SAFETY: new_kernel 恒等映射了内核的 .text（可执行）和 .bss（含启动栈），
    // 所以切换后的 pc 和 sp 都仍然有效。
    unsafe {
        KERNEL_SPACE.exclusive_access().as_ref().unwrap().activate();
    }

    // 允许 S 态访问带 U 位的页。
    //
    // 本章内核和用户**共用一个地址空间**，用户程序区和用户栈区都带 U 位，
    // 而内核要往里装载程序、要读用户传来的系统调用缓冲区 —— 不开 SUM
    // 的话，第一次装载就会 StorePageFault。
    //
    // 代价是失去了一层保护：内核不小心解引用用户指针不再会被硬件拦下。
    // 第 8 章给每个进程建独立地址空间后，内核改用手动查页表访问用户内存，
    // 这一位就能关掉。
    //
    // SAFETY: 只置位 SUM，不改动 sstatus 的其他位。
    unsafe {
        sstatus::write(sstatus::read() | sstatus_bits::SUM);
    }

    // 能打印出这一行，就说明分页已经开启且内核活着 —— 本章的验收标准。
    info!(
        "分页已开启：satp = {:#x}，{} 个逻辑段，页表自身占 {} 页（已开 SUM）",
        token, areas, frames
    );
}

/// 内核地址空间的 satp 值。第 8 章从用户态陷入内核时要用它切回来。
#[allow(dead_code)]
pub fn kernel_token() -> usize {
    KERNEL_SPACE
        .exclusive_access()
        .as_ref()
        .expect("内核地址空间还没建立")
        .token()
}

// ---------------------------------------------------------------------------
// 自检
// ---------------------------------------------------------------------------

/// 地址空间自检。
///
/// 重点验证 [`MapType::Framed`] 这条路径 —— 内核自己全用恒等映射，
/// 而第 8 章的用户地址空间全靠 Framed。趁现在把它压一遍，
/// 免得下一章同时调试两件新东西。
pub fn memory_set_test() {
    use super::frame_allocator::stats;

    let (baseline, _) = stats();

    let mut set = MemorySet::new_bare();

    // 挑一个和内核完全无关的虚拟地址区间 —— 正好说明"虚拟地址可以
    // 和物理地址毫无关系"，这是恒等映射看不出来的
    let start = VirtAddr::from(0x1_0000_0000usize);
    let end = VirtAddr::from(0x1_0000_0000usize + 3 * PAGE_SIZE);
    let perm = MapPermission::R | MapPermission::W | MapPermission::U;

    // 往里写一段横跨两页的数据。走正式接口 push(area, Some(data))，
    // 它会先建立映射再拷数据 —— 顺序不能反，没映射就没有物理页可写。
    let mut data = Vec::new();
    for i in 0..(PAGE_SIZE + 100) {
        data.push((i % 251) as u8);
    }
    set.push(
        MapArea::new(start, end, MapType::Framed, perm),
        Some(&data),
    );

    // 再插一段不带数据的，顺便验证 insert_framed_area 这条路
    let stack_lo = VirtAddr::from(0x2_0000_0000usize);
    let stack_hi = VirtAddr::from(0x2_0000_0000usize + 2 * PAGE_SIZE);
    set.insert_framed_area(stack_lo, stack_hi, perm);

    // 段的自述必须和我们要求的一致
    let last = set.areas.last().unwrap();
    assert_eq!(last.range().len(), 2, "第二个段应该正好 2 页");
    assert!(last.permission().contains(MapPermission::U), "第二个段应带 U 位");
    assert!(
        set.translate(stack_lo.floor()).is_some(),
        "insert_framed_area 没有真正建立映射"
    );

    // --- 逐页翻译，并通过物理地址读回内容 ---
    for (i, vpn) in VPNRange::new(start.floor(), end.ceil()).iter().enumerate() {
        let pte = set
            .translate(vpn)
            .unwrap_or_else(|| panic!("第 {} 页翻译失败", i));
        assert!(
            pte.flags().contains(PTEFlags::U | PTEFlags::R | PTEFlags::W),
            "第 {} 页权限不对",
            i
        );

        let bytes = pte.ppn().get_bytes_array();
        let offset = i * PAGE_SIZE;
        for (j, &b) in bytes.iter().enumerate() {
            let expected = data.get(offset + j).copied().unwrap_or(0);
            assert_eq!(b, expected, "第 {} 页第 {} 字节内容不对", i, j);
        }
    }
    debug!("Framed 映射：3 页翻译正确，跨页数据完整，尾部自动补零");

    // 3 页数据 + 2 页栈 + 页表自身
    let used = stats().0 - baseline;
    debug!(
        "这个地址空间共占 {} 页（5 页数据 + {} 页页表）",
        used,
        used - 5
    );

    drop(set);
    assert_eq!(
        stats().0,
        baseline,
        "地址空间销毁后页帧没有全部归还"
    );

    info!("地址空间自检通过：Framed 映射、跨页拷贝、RAII 整体回收均正确");
}

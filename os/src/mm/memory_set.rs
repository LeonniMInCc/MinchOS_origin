//! 地址空间。
//!
//! 第 7 章建了**一个**恒等映射的内核地址空间，内核和用户共用它。
//! 第 8 章起，**每个进程都有自己的地址空间**，内核也有自己的 ——
//! 这才是真正的内存隔离。
//!
//! # 一个用户地址空间长什么样
//!
//! ```text
//!   0x00010000  ELF 各段    代码 R-X U / 只读 R-- U / 数据 RW- U
//!               ...
//!               [guard page]           ← 不映射，栈写穿立刻缺页
//!               用户栈       RW- U
//!               ...
//!   ...FFFE000  TrapContext  RW-       ← 没有 U！只有内核碰它
//!   ...FFFF000  跳板页       R-X       ← 没有 U！两个空间同一个地址
//! ```
//!
//! # 内核地址空间
//!
//! ```text
//!   0x80200000  .text        R-X       ← 内核代码不可写
//!               .rodata      R--
//!               .data/.bss   RW-       ← 数据段不可执行
//!               可用物理内存  RW-       ← 恒等映射，内核靠它读写任意物理页
//!               ...
//!               任务 N 内核栈 RW-  ┐
//!               [guard page]      ├ 从跳板页往下依次排布
//!               任务 0 内核栈 RW-  ┘
//!   ...FFFF000  跳板页       R-X       ← 和每个用户空间同一个地址
//! ```
//!
//! 注意两个空间里都没有任何一页是既可写又可执行的，
//! 也没有任何一页是内核和用户都能访问的（跳板页不带 `U`）。

use super::address::{PhysAddr, PhysPageNum, VirtAddr, VirtPageNum};
use super::elf::{flags_str, ElfFile, PF_R, PF_W, PF_X, PT_LOAD};
use super::frame_allocator::{frame_alloc, FrameTracker};
use super::page_table::{PTEFlags, PageTable, PageTableEntry};
use crate::config::{
    kernel_stack_position, MEMORY_END, PAGE_SIZE, TRAMPOLINE, TRAP_CONTEXT, USER_STACK_SIZE,
};
use crate::csr::satp;
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

    /// 解除这个段的全部映射，并归还它占用的物理页帧。
    pub fn unmap(&mut self, page_table: &mut PageTable) {
        let range = self.vpn_range;
        for vpn in range.iter() {
            // 先丢掉 FrameTracker（触发 Drop 归还页帧），再清页表项。
            // 顺序反过来也能工作，但"先断引用再断映射"更符合直觉。
            self.data_frames.remove(&vpn);
            page_table.unmap(vpn);
        }
    }

    /// 按另一个段的形状造一个新段：范围、类型、权限都一样，
    /// 但**不共享任何物理页帧**。
    ///
    /// `fork` 用它 —— 子进程要的是一份独立的副本，而不是共享内存。
    /// （真实系统会用写时复制推迟这次拷贝，那是另一个话题。）
    pub fn from_another(other: &Self) -> Self {
        Self {
            vpn_range: other.vpn_range,
            data_frames: BTreeMap::new(),
            map_type: other.map_type,
            map_perm: other.map_perm,
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
        // 内核空间里**没有任何一页带 U** —— 这正是隔离的意义所在
        let (r, w, x) = (MapPermission::R, MapPermission::W, MapPermission::X);

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
        // 剩下的全部物理内存。内核必须能读写它分出去的任何一页 ——
        // 比如往用户进程的页帧里装载程序、读用户传来的缓冲区。
        add("可用物理内存", sym!(ekernel), MEMORY_END, r | w);

        // 设备寄存器（MMIO）。它不是"内存"，但 CPU 要用普通的 load/store
        // 去访问，所以同样得在页表里有映射。恒等映射 + R|W，不带 X 也不带 U。
        let (mmio_start, mmio_end) = crate::drivers::MMIO_REGION;
        add("MMIO 设备寄存器", mmio_start, mmio_end, r | w);

        // 跳板页。内核地址空间也要映射它：trap_return 里那条
        // `jr restore_va` 是在内核地址空间里执行的。
        set.map_trampoline();

        set
    }

    /// 把跳板页映射到 [`TRAMPOLINE`]。
    ///
    /// **每一个**地址空间都必须调用它 —— 内核的和每个进程的。
    /// 这是整章设计的支点：换页表的那段代码在切换前后位于同一个虚拟地址。
    ///
    /// 注意它直接操作 `page_table` 而不走 [`MemorySet::push`]：
    /// 跳板页指向的物理页是内核 `.text` 的一部分，不是从页帧分配器要来的，
    /// 所以不该被任何 [`MapArea`] "拥有" —— 否则进程退出时会把内核代码
    /// 所在的物理页"归还"给分配器。
    ///
    /// 权限是 `R | X`，**没有 `U`**：这段代码运行在 S 态。
    /// 给了 `U` 的话，用户程序就能直接跳进 `__restore` 自己切页表了。
    fn map_trampoline(&mut self) {
        extern "C" {
            static strampoline: u8;
        }
        self.page_table.map(
            VirtAddr::from(TRAMPOLINE).floor(),
            PhysAddr::from(&raw const strampoline as usize).floor(),
            PTEFlags::R | PTEFlags::X,
        );
    }

    /// 从一个 ELF 文件构造用户地址空间。
    ///
    /// 返回 `(地址空间, 用户栈顶, 入口地址)`。
    ///
    /// 布局（从低到高）：
    ///
    /// ```text
    ///   0x10000      ELF 各段（代码 R-X U、只读数据 R-- U、数据 RW- U）
    ///   ...
    ///   [guard page]  ← 不映射，栈写穿就缺页
    ///   用户栈        RW- U
    ///   ...
    ///   TRAP_CONTEXT  RW-   ← 没有 U！只有内核碰它
    ///   TRAMPOLINE    R-X   ← 没有 U！
    /// ```
    pub fn from_elf(elf_data: &[u8]) -> (Self, usize, usize) {
        let mut set = Self::new_bare();
        set.map_trampoline();

        let elf = ElfFile::parse(elf_data).expect("用户程序不是合法的 ELF");
        let (r, w, x, u) = (
            MapPermission::R,
            MapPermission::W,
            MapPermission::X,
            MapPermission::U,
        );

        let mut max_end = VirtAddr::from(0usize);
        for ph in elf.program_headers().filter(|p| p.p_type == PT_LOAD) {
            // 段的权限直接来自 ELF —— 这是第 7 章解析程序头的回报：
            // 代码段拿不到写权限，数据段拿不到执行权限。
            let mut perm = u;
            if ph.flags & PF_R != 0 {
                perm = perm | r;
            }
            if ph.flags & PF_W != 0 {
                perm = perm | w;
            }
            if ph.flags & PF_X != 0 {
                perm = perm | x;
            }

            let start_va = VirtAddr::from(ph.vaddr);
            let end_va = VirtAddr::from(ph.vaddr + ph.memsz);
            let area = MapArea::new(start_va, end_va, MapType::Framed, perm);
            max_end = VirtAddr::from(area.range().end);

            let [fr, fw, fx] = flags_str(ph.flags);
            debug!(
                "    段 {}{}{}  [{:#x}, {:#x})  {} 页",
                fr,
                fw,
                fx,
                ph.vaddr,
                ph.vaddr + ph.memsz,
                area.range().len()
            );

            // memsz 比 filesz 大的那部分是 .bss，新分配的页帧本来就是
            // 全 0（见 FrameTracker::new），所以不用额外处理
            set.push(area, elf.segment_data(&ph));
        }

        // 用户栈：和程序段之间隔一页 guard page
        let stack_bottom = max_end.0 + PAGE_SIZE;
        let stack_top = stack_bottom + USER_STACK_SIZE;
        set.insert_framed_area(stack_bottom.into(), stack_top.into(), r | w | u);

        // TrapContext 页。**不给 U** —— 它是内核用来保存/恢复这个进程现场的
        // 地方，进程自己碰不得。给了 U 的话，用户程序就能改写自己的
        // sepc/sstatus，等于可以任意跳转、甚至提权。
        set.push(
            MapArea::new(
                TRAP_CONTEXT.into(),
                TRAMPOLINE.into(),
                MapType::Framed,
                r | w,
            ),
            None,
        );

        (set, stack_top, elf.entry())
    }

    /// 深拷贝一个用户地址空间。`fork` 的核心。
    ///
    /// 逐段复制：形状（虚拟地址范围、权限）照搬，物理页帧全部重新分配，
    /// 然后把内容一页一页拷过去。结果是两个**布局完全相同、内容完全相同、
    /// 但互不影响**的地址空间。
    ///
    /// 这也解释了为什么 `fork` 在真实系统里要用写时复制优化：
    /// 这里每 fork 一次就要实打实拷贝整个地址空间，而绝大多数子进程
    /// 紧接着就会 `exec`，把刚拷来的东西整个丢掉。
    pub fn from_existed_user(user_space: &Self) -> Self {
        let mut set = Self::new_bare();
        set.map_trampoline();

        for area in user_space.areas.iter() {
            set.push(MapArea::from_another(area), None);
            // 新段已经映射好了，逐页把内容搬过去
            for vpn in area.range().iter() {
                let src = user_space.translate(vpn).expect("源地址空间缺映射").ppn();
                let dst = set.translate(vpn).expect("新地址空间缺映射").ppn();
                dst.get_bytes_array().copy_from_slice(src.get_bytes_array());
            }
        }
        set
    }

    /// 按起始虚拟页号删掉一个段。回收内核栈时用。
    pub fn remove_area_with_start_vpn(&mut self, start_vpn: VirtPageNum) {
        if let Some(idx) = self
            .areas
            .iter()
            .position(|area| area.range().start == start_vpn)
        {
            let mut area = self.areas.remove(idx);
            area.unmap(&mut self.page_table);
        }
    }

    /// 丢掉所有逻辑段，归还它们占用的物理页帧，但**保留页表本身**。
    ///
    /// `exec` 和进程退出时用。为什么不直接丢掉整个 `MemorySet`？
    /// 因为进程退出后它还要作为僵尸挂在父进程的子进程列表里等 wait，
    /// 而那期间它占着的用户内存应该立刻还回去 —— 一个已经死掉的进程
    /// 没理由继续占着几 MiB 物理内存。
    pub fn recycle_data_pages(&mut self) {
        self.areas.clear();
    }

    /// 在**内核地址空间**里给 PID 为 `pid` 的进程分配内核栈，返回栈顶。
    ///
    /// 相邻两个栈之间天然隔着一页不映射的空隙（见
    /// [`crate::config::kernel_stack_position`]），那就是 guard page。
    pub fn map_kernel_stack(&mut self, pid: usize) -> usize {
        let (bottom, top) = kernel_stack_position(pid);
        self.insert_framed_area(
            bottom.into(),
            top.into(),
            MapPermission::R | MapPermission::W,
        );
        top
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

    // 注意这里**没有**打开 sstatus.SUM。
    //
    // 第 7 章内核和用户共用一个地址空间，内核要碰带 U 位的页，不得不开它。
    // 现在两者的地址空间彻底分开了，内核访问用户内存改走
    // `translated_byte_buffer`（手动查用户页表 + 恒等映射），
    // 于是 SUM 可以保持关闭 —— 内核不小心解引用一个用户指针时，
    // 硬件会立刻拦下来。第 7 章欠的那笔账在这里还清了。
    info!(
        "分页已开启：satp = {:#x}，{} 个逻辑段，页表自身占 {} 页（SUM 保持关闭）",
        token, areas, frames
    );
}

/// 在内核地址空间里给 PID 为 `pid` 的进程映射内核栈，返回栈顶（虚拟地址）。
pub fn map_kernel_stack(pid: usize) -> usize {
    KERNEL_SPACE
        .exclusive_access()
        .as_mut()
        .expect("内核地址空间还没建立")
        .map_kernel_stack(pid)
}

/// 解除 PID 为 `pid` 的进程的内核栈映射，归还物理页帧。
pub fn unmap_kernel_stack(pid: usize) {
    let (bottom, _) = kernel_stack_position(pid);
    KERNEL_SPACE
        .exclusive_access()
        .as_mut()
        .expect("内核地址空间还没建立")
        .remove_area_with_start_vpn(VirtAddr::from(bottom).floor());
}

/// 内核地址空间的 `satp`。每个进程的 TrapContext 里都存着它的副本，
/// `__alltraps` 靠它从用户页表切回内核页表。
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

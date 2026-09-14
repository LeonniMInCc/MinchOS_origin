//! 地址与页号的类型封装。
//!
//! # 为什么值得为四个 `usize` 单独定义四个类型
//!
//! 内存管理代码里同时存在四种"数"：
//!
//! | 类型 | 含义 | 典型值 |
//! |---|---|---|
//! | [`PhysAddr`] | 物理地址 | `0x80215abc` |
//! | [`VirtAddr`] | 虚拟地址 | `0x10000abc` |
//! | [`PhysPageNum`] | 物理页号 = 物理地址 >> 12 | `0x80215` |
//! | [`VirtPageNum`] | 虚拟页号 | `0x10000` |
//!
//! 它们全都是 `usize`。如果直接用 `usize` 写，把物理地址当虚拟地址用、
//! 或者忘了移 12 位，编译器一句话都不会说 —— 而这类 bug 的表现是
//! "页表建好了但访存就崩"，极难定位。
//!
//! 包一层 newtype 之后，这些错误全部变成编译错误。**代价是几十行样板，
//! 收益是第 7-8 周少熬几个通宵。**
//!
//! # SV39 的地址结构
//!
//! ```text
//!   虚拟地址 (39 位有效)
//!   ┌─────────┬─────────┬─────────┬────────────┐
//!   │ VPN[2]  │ VPN[1]  │ VPN[0]  │   页内偏移  │
//!   │  9 位   │  9 位   │  9 位   │   12 位     │
//!   └─────────┴─────────┴─────────┴────────────┘
//!    38    30 29     21 20     12 11          0
//!
//!   三级页表，每级 512 项（2^9），每项 8 字节，正好一页 4 KiB。
//! ```

use crate::config::{PAGE_SIZE, PAGE_SIZE_BITS};
use core::fmt::{self, Debug, Formatter};

/// SV39 的物理地址宽度：56 位。
const PA_WIDTH_SV39: usize = 56;
/// SV39 的虚拟地址宽度：39 位。
const VA_WIDTH_SV39: usize = 39;
/// 物理页号宽度：56 - 12 = 44 位。
const PPN_WIDTH_SV39: usize = PA_WIDTH_SV39 - PAGE_SIZE_BITS;
/// 虚拟页号宽度：39 - 12 = 27 位。
const VPN_WIDTH_SV39: usize = VA_WIDTH_SV39 - PAGE_SIZE_BITS;
/// 一页里能放多少个页表项：4096 / 8 = 512 = 2^9。
///
/// 这个 512 和 VPN 每级 9 位是同一件事的两面 —— SV39 特意这么设计，
/// 好让每一级页表正好占满一个物理页。
pub const PTES_PER_PAGE: usize = 512;

/// 物理地址。
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Default)]
pub struct PhysAddr(pub usize);

/// 虚拟地址。
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Default)]
pub struct VirtAddr(pub usize);

/// 物理页号。
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Default)]
pub struct PhysPageNum(pub usize);

/// 虚拟页号。
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Default)]
pub struct VirtPageNum(pub usize);

// ---------------------------------------------------------------------------
// Debug：打印时带上类型前缀，日志里一眼能分清是哪一种
// ---------------------------------------------------------------------------

impl Debug for PhysAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "PA:{:#x}", self.0)
    }
}
impl Debug for VirtAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "VA:{:#x}", self.0)
    }
}
impl Debug for PhysPageNum {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "PPN:{:#x}", self.0)
    }
}
impl Debug for VirtPageNum {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "VPN:{:#x}", self.0)
    }
}

// ---------------------------------------------------------------------------
// usize ⇄ 四种类型
// ---------------------------------------------------------------------------

// 从 usize 构造时**截断**到规范宽度：SV39 只认 56/39 位，
// 高位必须丢掉，否则算出来的页号会带上垃圾。
impl From<usize> for PhysAddr {
    fn from(v: usize) -> Self {
        Self(v & ((1 << PA_WIDTH_SV39) - 1))
    }
}
impl From<usize> for PhysPageNum {
    fn from(v: usize) -> Self {
        Self(v & ((1 << PPN_WIDTH_SV39) - 1))
    }
}
impl From<usize> for VirtAddr {
    fn from(v: usize) -> Self {
        Self(v & ((1 << VA_WIDTH_SV39) - 1))
    }
}
impl From<usize> for VirtPageNum {
    fn from(v: usize) -> Self {
        Self(v & ((1 << VPN_WIDTH_SV39) - 1))
    }
}

impl From<PhysAddr> for usize {
    fn from(v: PhysAddr) -> Self {
        v.0
    }
}
impl From<PhysPageNum> for usize {
    fn from(v: PhysPageNum) -> Self {
        v.0
    }
}
impl From<VirtAddr> for usize {
    fn from(v: VirtAddr) -> Self {
        v.0
    }
}
impl From<VirtPageNum> for usize {
    fn from(v: VirtPageNum) -> Self {
        v.0
    }
}

// ---------------------------------------------------------------------------
// 地址 ⇄ 页号
// ---------------------------------------------------------------------------

impl VirtAddr {
    /// 所在页的页号（向下取整）。
    pub fn floor(&self) -> VirtPageNum {
        VirtPageNum(self.0 / PAGE_SIZE)
    }

    /// 向上取整的页号。用来算"覆盖某个区间需要到哪一页为止"。
    pub fn ceil(&self) -> VirtPageNum {
        if self.0 == 0 {
            VirtPageNum(0)
        } else {
            VirtPageNum((self.0 - 1 + PAGE_SIZE) / PAGE_SIZE)
        }
    }

    /// 页内偏移，即低 12 位。
    pub fn page_offset(&self) -> usize {
        self.0 & (PAGE_SIZE - 1)
    }

    /// 是否正好落在页边界上。
    pub fn aligned(&self) -> bool {
        self.page_offset() == 0
    }
}

impl PhysAddr {
    pub fn floor(&self) -> PhysPageNum {
        PhysPageNum(self.0 / PAGE_SIZE)
    }

    pub fn ceil(&self) -> PhysPageNum {
        if self.0 == 0 {
            PhysPageNum(0)
        } else {
            PhysPageNum((self.0 - 1 + PAGE_SIZE) / PAGE_SIZE)
        }
    }

    pub fn page_offset(&self) -> usize {
        self.0 & (PAGE_SIZE - 1)
    }

    pub fn aligned(&self) -> bool {
        self.page_offset() == 0
    }
}

// 地址 → 页号只在对齐时才有唯一含义，所以要求对齐。
// 不对齐说明调用方把"某个地址"和"某一页"搞混了 —— 这正是我们想抓的错误。
impl From<PhysAddr> for PhysPageNum {
    fn from(v: PhysAddr) -> Self {
        assert!(v.aligned(), "{:?} 没有页对齐，不能直接当作页号", v);
        v.floor()
    }
}
impl From<VirtAddr> for VirtPageNum {
    fn from(v: VirtAddr) -> Self {
        assert!(v.aligned(), "{:?} 没有页对齐，不能直接当作页号", v);
        v.floor()
    }
}

// 页号 → 地址永远是页首地址，无条件成立
impl From<PhysPageNum> for PhysAddr {
    fn from(v: PhysPageNum) -> Self {
        Self(v.0 << PAGE_SIZE_BITS)
    }
}
impl From<VirtPageNum> for VirtAddr {
    fn from(v: VirtPageNum) -> Self {
        Self(v.0 << PAGE_SIZE_BITS)
    }
}

// ---------------------------------------------------------------------------
// 三级页表的索引
// ---------------------------------------------------------------------------

impl VirtPageNum {
    /// 拆出三级页表的索引：`[VPN[2], VPN[1], VPN[0]]`，每个 9 位。
    ///
    /// 页表查找就是拿这三个数依次去三层页表里查，
    /// 最后一层查到的项里存着目标物理页号。
    pub fn indexes(&self) -> [usize; 3] {
        let mut vpn = self.0;
        let mut idx = [0usize; 3];
        // 从最低一级往回填：VPN[0] 在最低 9 位
        for i in (0..3).rev() {
            idx[i] = vpn & 0x1ff; // 取低 9 位
            vpn >>= 9;
        }
        idx
    }

    /// 下一个虚拟页。
    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }
}

impl PhysPageNum {
    /// 把这一页**当作页表**来看：512 个 8 字节的页表项。
    ///
    /// # 这为什么能工作
    ///
    /// 我们直接拿物理页号当地址去访问。第 6 周还没开分页，
    /// 虚拟地址等于物理地址，所以显然可行。
    ///
    /// 第 7 周开启分页之后它依然可行 —— 因为内核会把整个物理内存
    /// **恒等映射**进自己的地址空间（虚拟地址 = 物理地址）。
    /// 这是内核能随意读写任意物理页的前提，也是"内核地址空间"和
    /// "用户地址空间"最大的结构差异。
    ///
    /// # Safety 说明
    ///
    /// 返回 `'static` 生命周期是一个刻意的谎言：这块内存的实际寿命
    /// 由持有对应 [`crate::mm::FrameTracker`] 的人决定。这里用
    /// `'static` 是为了让调用方写起来方便，代价是**调用方必须保证
    /// 不会在帧被回收之后继续使用返回的引用**。
    pub fn get_pte_array(&self) -> &'static mut [super::page_table::PageTableEntry] {
        let pa: PhysAddr = (*self).into();
        unsafe { core::slice::from_raw_parts_mut(pa.0 as *mut _, PTES_PER_PAGE) }
    }

    /// 这一页（作为页表）里第 `idx` 项。
    ///
    /// 写成直接算指针而不是 `&mut self.get_pte_array()[idx]`，
    /// 是因为后者返回的引用会被借用检查器绑到那个临时切片上，
    /// 拿不到调用方需要的 `'static`。与其和生命周期较劲，
    /// 不如把"我就是要这个地址上的那一项"直接说出来。
    pub fn pte_at(&self, idx: usize) -> &'static mut super::page_table::PageTableEntry {
        assert!(idx < PTES_PER_PAGE, "页表项下标 {} 越界", idx);
        let pa: PhysAddr = (*self).into();
        // SAFETY: 物理页有 4096 字节，容得下 512 个 8 字节的页表项；
        // idx 已经过范围检查。生命周期见 get_pte_array 的说明。
        unsafe { &mut *(pa.0 as *mut super::page_table::PageTableEntry).add(idx) }
    }

    /// 把这一页当作 4096 个字节来看。装载用户程序、清零页面时用。
    pub fn get_bytes_array(&self) -> &'static mut [u8] {
        let pa: PhysAddr = (*self).into();
        unsafe { core::slice::from_raw_parts_mut(pa.0 as *mut u8, PAGE_SIZE) }
    }

    /// 这一页里 `[offset, offset + len)` 这一段。
    ///
    /// 和 `&mut get_bytes_array()[a..b]` 的区别同 [`PhysPageNum::pte_at`]：
    /// 后者返回的引用会被借用检查器绑到临时切片上，拿不到 `'static`。
    pub fn bytes_at(&self, offset: usize, len: usize) -> &'static mut [u8] {
        assert!(
            offset + len <= PAGE_SIZE,
            "[{}, {}) 超出了一页的范围",
            offset,
            offset + len
        );
        let pa: PhysAddr = (*self).into();
        // SAFETY: 上面刚断言过范围不超过一页。生命周期见 get_bytes_array 的说明。
        unsafe { core::slice::from_raw_parts_mut((pa.0 + offset) as *mut u8, len) }
    }

    /// 把这一页的开头当作一个 `T`。
    pub fn get_mut<T>(&self) -> &'static mut T {
        let pa: PhysAddr = (*self).into();
        unsafe { &mut *(pa.0 as *mut T) }
    }

    /// 下一个物理页。
    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }
}

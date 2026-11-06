//! 内存管理子系统。
//!
//! - 第 2 周：内核堆。让内核自己能用上动态数据结构
//! - **第 6 周（现在）**：物理页帧分配器 + SV39 多级页表
//! - 第 7 周：地址空间抽象，开启分页
//! - 第 8 周：跳板页，用户/内核地址空间彻底隔离
//!
//! # 三个层次，别搞混
//!
//! | 模块 | 管什么 | 单位 |
//! |---|---|---|
//! | [`heap_allocator`] | 内核 `.bss` 里那 3 MiB | 任意字节 |
//! | [`frame_allocator`] | `[ekernel, MEMORY_END)` 全部物理内存 | 4 KiB 一页 |
//! | [`page_table`] | 虚拟地址 → 物理地址的映射关系 | 页表项 |
//!
//! 页帧分配器的元数据住在内核堆上，页表的每一级又是从页帧分配器要来的页。
//! 所以初始化顺序是固定的：**堆 → 页帧 → 页表**。

mod address;
pub mod elf;
mod frame_allocator;
mod heap_allocator;
mod memory_set;
mod page_table;

#[allow(unused_imports)]
pub use address::{PhysAddr, PhysPageNum, VirtAddr, VirtPageNum};
#[allow(unused_imports)]
pub use frame_allocator::{frame_alloc, record_baseline, report_leak, FrameTracker};
#[allow(unused_imports)]
pub use memory_set::{
    kernel_token, map_kernel_stack, unmap_kernel_stack, MapArea, MapPermission, MapType, MemorySet,
    VPNRange,
};
#[allow(unused_imports)]
pub use page_table::{
    translated_byte_buffer, translated_refmut, translated_str, PTEFlags, PageTable, PageTableEntry,
    UserBuffer,
};

/// 初始化内存管理子系统。
///
/// 必须在 `clear_bss()` 之后调用 —— 堆空间就在 `.bss` 里。
pub fn init() {
    heap_allocator::init();
    frame_allocator::init();
}

/// 建立并激活内核地址空间。此后所有访存都要经过 MMU。
///
/// 必须在 [`init`] 之后调用 —— 页表的每一级都是从页帧分配器要来的。
pub fn enable_paging() {
    memory_set::init_kernel_space();
}

/// 开机自检。
///
/// 内核跑不了 `cargo test`（测试框架依赖 std），所以正确性验证
/// 只能做成开机自检。
pub fn self_test() {
    heap_allocator::heap_test();
    frame_allocator::frame_test();
    page_table::page_table_test();
    memory_set::memory_set_test();
}

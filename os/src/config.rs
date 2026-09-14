//! 内核可调参数。
//!
//! 散落各处的魔数很难维护，统一放这里。

/// 内核堆大小：3 MiB。
///
/// 够装下进程表、页表、文件描述符表这些元数据了。注意它是一个静态数组，
/// 整块落在 `.bss` 段里 —— 所以这个数字直接决定内核镜像加载后占多少物理内存。
/// 第 1 章那张布局表里 `.bss` 还是 0 字节，这一章之后会一下子涨到 3 MiB。
pub const KERNEL_HEAP_SIZE: usize = 3 * 1024 * 1024;

/// 用户程序区的起始地址。
///
/// 本章没有分页，所有程序**同时**躺在物理内存里，各占一个
/// [`APP_SIZE_LIMIT`] 大小的槽位：
///
/// ```text
///   程序 0 → 0x80400000
///   程序 1 → 0x80420000
///   程序 2 → 0x80440000
/// ```
///
/// 每个程序在链接时就被写死了自己槽位的地址（见 `tools/build-user.py`）。
///
/// 第 7 周引入分页后，这会变成**虚拟地址**，所有程序都可以以为
/// 自己独占 `0x80400000`，而实际占用互不相同的物理页。
/// 这个地址必须**在内核镜像之上**。内核的 `.bss` 里有 3 MiB 的堆，
/// 加上代码、数据、各任务的栈，镜像一直长到 `0x8056xxxx` 附近 ——
/// 所以起点要留出足够余量。`loader::load_apps` 开头有一条断言
/// 在每次开机时核对这件事。
pub const APP_BASE_ADDRESS: usize = 0x8080_0000;

/// 单个用户程序的槽位大小：128 KiB。
pub const APP_SIZE_LIMIT: usize = 0x2_0000;

/// 用户程序区的终点（不含）。
///
/// 物理页帧分配器必须从这里之后开始 —— 否则它会把用户程序正躺着的
/// 内存当成空闲页分出去。
pub const fn app_region_end() -> usize {
    APP_BASE_ADDRESS + MAX_APP_NUM * APP_SIZE_LIMIT
}

/// 第 `app_id` 个程序的槽位基址。
///
/// 必须和 `tools/build-user.py` 里的计算完全一致 —— 内核算错了，
/// 程序就会被装到一个和它链接地址不符的位置上，启动即跑飞。
pub const fn app_base(app_id: usize) -> usize {
    APP_BASE_ADDRESS + app_id * APP_SIZE_LIMIT
}

/// 用户栈大小：8 KiB。
pub const USER_STACK_SIZE: usize = 4096 * 2;

/// 内核栈大小：8 KiB。用户程序陷入内核后，内核在这上面运行。
pub const KERNEL_STACK_SIZE: usize = 4096 * 2;

/// 最多能链接进内核的用户程序数量。
pub const MAX_APP_NUM: usize = 16;

/// 页大小：4 KiB。SV39 的基本单位。
pub const PAGE_SIZE: usize = 4096;

/// 页内偏移占的位数。`4096 = 2^12`，所以是 12。
pub const PAGE_SIZE_BITS: usize = 12;

/// 物理内存的终点。
///
/// QEMU virt 默认给 128 MiB 内存，起点 `0x80000000`，所以终点是 `0x88000000`。
/// `[ekernel, MEMORY_END)` 这一段就是可以分给用户进程的物理页。
///
/// 换到别的平台必须改 —— 多报了会分配出不存在的内存，
/// 少报了则白白浪费。
pub const MEMORY_END: usize = 0x8800_0000;

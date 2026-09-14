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
pub const APP_BASE_ADDRESS: usize = 0x8040_0000;

/// 单个用户程序的槽位大小：128 KiB。
pub const APP_SIZE_LIMIT: usize = 0x2_0000;

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

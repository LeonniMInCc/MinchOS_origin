//! 用户程序装载。
//!
//! 第 3 周的批处理是"用一个装一个"，这一章改成**开机一次性全部装入**。
//! 这是"多道程序"的物理前提：多个程序同时待在内存里，才谈得上轮流执行。
//!
//! # 内存布局
//!
//! ```text
//!   0x80400000  ┌────────────────┐
//!               │   程序 0 槽位   │  128 KiB
//!   0x80420000  ├────────────────┤
//!               │   程序 1 槽位   │  128 KiB
//!   0x80440000  ├────────────────┤
//!               │      ...       │
//!               └────────────────┘
//! ```
//!
//! 每个程序还各自拥有一个内核栈和一个用户栈（都是内核 `.bss` 里的静态数组）。
//!
//! **为什么每个程序要有自己的内核栈？** 因为它们的执行可以交错：程序 A 陷入
//! 内核、还没处理完就切到了程序 B。A 在内核里的执行状态（局部变量、返回地址）
//! 必须原样留在它自己的内核栈上，等它被调度回来时继续。共用一个栈会互相覆盖。

use crate::config::{
    app_base, app_region_end, APP_BASE_ADDRESS, APP_SIZE_LIMIT, KERNEL_STACK_SIZE, MAX_APP_NUM,
    USER_STACK_SIZE,
};
use crate::trap::TrapContext;
use core::arch::asm;
use core::mem::size_of;

// ---------------------------------------------------------------------------
// 每个程序一套栈
// ---------------------------------------------------------------------------

#[repr(align(4096))]
struct KernelStack {
    #[allow(dead_code)] // 只取地址；实际读写由 CPU 通过 sp 完成
    data: [u8; KERNEL_STACK_SIZE],
}

#[repr(align(4096))]
struct UserStack {
    #[allow(dead_code)]
    data: [u8; USER_STACK_SIZE],
}

static mut KERNEL_STACKS: [KernelStack; MAX_APP_NUM] = [const {
    KernelStack {
        data: [0; KERNEL_STACK_SIZE],
    }
}; MAX_APP_NUM];

static mut USER_STACKS: [UserStack; MAX_APP_NUM] = [const {
    UserStack {
        data: [0; USER_STACK_SIZE],
    }
}; MAX_APP_NUM];

/// 第 `app_id` 个栈的低地址端。
///
/// 写成"取数组基址再偏移"而不是 `&raw const STACKS[app_id]`：
/// 对 `static mut` 做下标索引会被编译器判定为"使用可变静态量"，需要
/// `unsafe`；而取整个数组的地址则不需要。绕过去之后 `unsafe` 就只
/// 落在真正危险的那一步 —— 指针偏移 —— 上，范围更小、更好审查。
fn stack_base<T>(stacks: *const T, app_id: usize) -> usize {
    assert!(app_id < MAX_APP_NUM, "任务编号 {app_id} 越界");
    // SAFETY: 上面刚断言过 app_id 在数组范围内。
    unsafe { stacks.add(app_id) as usize }
}

fn kernel_stack_top(app_id: usize) -> usize {
    stack_base((&raw const KERNEL_STACKS).cast::<KernelStack>(), app_id) + KERNEL_STACK_SIZE
}

fn user_stack_top(app_id: usize) -> usize {
    stack_base((&raw const USER_STACKS).cast::<UserStack>(), app_id) + USER_STACK_SIZE
}

fn user_stack_bottom(app_id: usize) -> usize {
    stack_base((&raw const USER_STACKS).cast::<UserStack>(), app_id)
}

// ---------------------------------------------------------------------------
// 程序表
// ---------------------------------------------------------------------------

extern "C" {
    /// 由 `build.rs` 生成的 `link_app.S` 提供。
    static _num_app: usize;
}

/// 指向 `link_app.S` 里那张表：`[个数, start0, start1, ..., startN, endN]`。
fn app_table() -> *const usize {
    &raw const _num_app
}

/// 链接进内核的用户程序数量。
pub fn num_app() -> usize {
    // SAFETY: link_app.S 保证表头就是一个 usize 的计数。
    unsafe { app_table().read_volatile() }
}

/// 第 `app_id` 个程序在内核镜像里的字节范围。
fn app_bytes(app_id: usize) -> (usize, usize) {
    // SAFETY: app_id < num_app() 由调用方保证；表长由 num_app 决定。
    unsafe {
        let starts = app_table().add(1);
        (starts.add(app_id).read_volatile(), starts.add(app_id + 1).read_volatile())
    }
}

/// 把全部用户程序装入各自的槽位。开机时调用一次。
pub fn load_apps() {
    let n = num_app();
    assert!(n <= MAX_APP_NUM, "用户程序数量 {n} 超过上限 {MAX_APP_NUM}");

    // 用户程序区绝不能和内核镜像重叠。
    //
    // 这条断言是补上的 —— 早期版本里 APP_BASE_ADDRESS 恰好落在内核堆
    // 中间，而它一直没出事，只是因为堆从低端分配、用量又小，
    // 那一片碰巧从没被真正用到。这种"靠运气成立"的正确性迟早要还账：
    // 内核堆用量一旦涨上去，装载用户程序就会直接踩碎堆，
    // 而症状会出现在完全无关的地方。
    //
    // 现在每次开机都核对一遍，代价是一条比较指令。
    extern "C" {
        static ekernel: u8;
    }
    let kernel_end = &raw const ekernel as usize;
    assert!(
        kernel_end <= APP_BASE_ADDRESS,
        "内核镜像终点 {:#x} 越过了用户程序区起点 {:#x}，请调大 APP_BASE_ADDRESS",
        kernel_end,
        APP_BASE_ADDRESS
    );

    info!(
        "装载 {} 个用户程序（内核终点 {:#x}，程序区 {:#x}..{:#x}）：",
        n,
        kernel_end,
        APP_BASE_ADDRESS,
        app_region_end()
    );

    for app_id in 0..n {
        let (src_start, src_end) = app_bytes(app_id);
        let size = src_end - src_start;
        let base = app_base(app_id);
        assert!(
            size <= APP_SIZE_LIMIT,
            "程序 {app_id} 有 {size} 字节，超过槽位上限 {APP_SIZE_LIMIT} 字节"
        );

        // SAFETY: 槽位是我们独占的物理内存区域，此刻还没有任何程序在运行。
        unsafe {
            // 先清空整个槽位：程序的 .bss 段在镜像里不占空间，
            // 但运行时必须是 0。不清的话，用户程序的 clear_bss 之外的
            // 区域会留着上电时的随机内容。
            core::slice::from_raw_parts_mut(base as *mut u8, APP_SIZE_LIMIT).fill(0);

            let src = core::slice::from_raw_parts(src_start as *const u8, size);
            let dst = core::slice::from_raw_parts_mut(base as *mut u8, size);
            dst.copy_from_slice(src);
        }

        info!("  [{}] {:>6} 字节 → {:#012x}", app_id, size, base);
    }

    // 我们刚把一大堆数据当作**代码**写进了内存。CPU 的取指单元可能还
    // 缓存着这些地址的旧内容，`fence.i` 让它丢弃指令缓存重新取指。
    //
    // 第 3 周是每装一个程序刷一次，现在一次性装完统一刷一次就够了。
    //
    // SAFETY: fence.i 只影响取指缓存，没有其他副作用。
    unsafe {
        asm!("fence.i");
    }
}

/// 构造第 `app_id` 个程序的初始 [`TrapContext`]，压到它自己的内核栈上，
/// 返回压入后的栈顶地址。
///
/// 这个地址就是该任务的初始 `sp` —— 任务第一次被调度时，`__switch` 会把
/// 它装进 `sp` 然后跳到 `__restore`，于是 `__restore` 正好看到这份上下文。
pub fn init_app_cx(app_id: usize) -> usize {
    let cx = TrapContext::app_init_context(app_base(app_id), user_stack_top(app_id));
    let ptr = (kernel_stack_top(app_id) - size_of::<TrapContext>()) as *mut TrapContext;
    // SAFETY: 内核栈 8 KiB，减去 TrapContext 的 272 字节后仍在栈内。
    unsafe {
        ptr.write(cx);
    }
    ptr as usize
}

/// 第 `app_id` 个程序能否合法访问 `[start, start + len)`。
///
/// 内核在系统调用入口用它验证用户传来的指针。本章依然没有分页，
/// 这是唯一的防线 —— 而且现在还多了一重意义：**程序 A 不能读写程序 B 的槽位**。
pub fn is_accessible_by(app_id: usize, start: usize, len: usize) -> bool {
    // 先防整数溢出：用户传 (start=usize::MAX-8, len=64) 的话，
    // start + len 会绕回一个很小的数，让后面的比较全部失效。
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let base = app_base(app_id);
    let in_own_slot = start >= base && end <= base + APP_SIZE_LIMIT;
    let in_own_stack = start >= user_stack_bottom(app_id) && end <= user_stack_top(app_id);
    in_own_slot || in_own_stack
}

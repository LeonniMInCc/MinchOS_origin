//! 用户程序装载。
//!
//! 第 7 周起改为解析 **ELF** 而不是拷贝裸字节。区别在于 ELF 告诉了我们
//! 两件之前只能靠假定的事：程序的入口在哪，以及哪一段是代码、哪一段是数据。
//!
//! # 内存布局
//!
//! ```text
//!   0x80200000  内核镜像（.text/.rodata/.data/.bss，含内核堆与内核栈）
//!   0x80800000  用户程序槽位  16 × 128 KiB
//!   0x80a00000  用户栈        16 × 8 KiB
//!   0x80a20000  空闲物理页帧
//! ```
//!
//! **用户栈从第 7 周起不再放在内核 `.bss` 里。** 开启分页后，
//! "用户能不能访问某一页"由页表项的 `U` 位决定，而内核 `.bss` 整体
//! 是不带 `U` 的 —— 用户栈混在里面就没法单独开权限。
//!
//! 内核栈仍然留在 `.bss`：它本来就只有内核能碰。

use crate::config::{
    app_base, app_region_end, user_stack_range, APP_BASE_ADDRESS, APP_SIZE_LIMIT,
    KERNEL_STACK_SIZE, MAX_APP_NUM, USER_STACK_BASE,
};
use crate::mm::elf::{flags_str, ElfFile, PT_LOAD};
use crate::trap::TrapContext;
use core::arch::asm;
use core::mem::size_of;

// ---------------------------------------------------------------------------
// 内核栈（每任务一个，留在内核 .bss 里）
// ---------------------------------------------------------------------------

#[repr(align(4096))]
struct KernelStack {
    #[allow(dead_code)] // 只取地址；实际读写由 CPU 通过 sp 完成
    data: [u8; KERNEL_STACK_SIZE],
}

static mut KERNEL_STACKS: [KernelStack; MAX_APP_NUM] = [const {
    KernelStack {
        data: [0; KERNEL_STACK_SIZE],
    }
}; MAX_APP_NUM];

fn kernel_stack_top(app_id: usize) -> usize {
    assert!(app_id < MAX_APP_NUM, "任务编号 {app_id} 越界");
    let base = (&raw const KERNEL_STACKS).cast::<KernelStack>();
    // SAFETY: 上面刚断言过 app_id 在数组范围内。
    unsafe { base.add(app_id) as usize + KERNEL_STACK_SIZE }
}

// ---------------------------------------------------------------------------
// 程序表
// ---------------------------------------------------------------------------

extern "C" {
    /// 由 `build.rs` 生成的 `link_app.S` 提供。
    static _num_app: usize;
}

fn app_table() -> *const usize {
    &raw const _num_app
}

/// 链接进内核的用户程序数量。
pub fn num_app() -> usize {
    // SAFETY: link_app.S 保证表头就是一个 usize 的计数。
    unsafe { app_table().read_volatile() }
}

/// 第 `app_id` 个程序的 ELF 字节。
fn app_elf_bytes(app_id: usize) -> &'static [u8] {
    assert!(app_id < num_app(), "程序编号 {app_id} 越界");
    // SAFETY: link_app.S 保证了表的布局；这些字节嵌在内核 .data 段里，
    // 生命周期和内核一样长。
    unsafe {
        let starts = app_table().add(1);
        let start = starts.add(app_id).read_volatile();
        let end = starts.add(app_id + 1).read_volatile();
        core::slice::from_raw_parts(start as *const u8, end - start)
    }
}

fn parse_app(app_id: usize) -> ElfFile<'static> {
    ElfFile::parse(app_elf_bytes(app_id))
        .unwrap_or_else(|e| panic!("程序 {app_id} 不是合法的 ELF：{e}"))
}

/// 第 `app_id` 个程序的入口地址。
///
/// 之前几章是直接假定"入口 = 加载地址"。这个假定碰巧成立（链接脚本
/// 把 `.text.entry` 放在最前面），但它是个假定 —— 现在从 ELF 头里读，
/// 就不再依赖链接脚本的内部细节了。
pub fn app_entry(app_id: usize) -> usize {
    parse_app(app_id).entry()
}

/// 把全部用户程序装入各自的槽位。开机时调用一次。
pub fn load_apps() {
    let n = num_app();
    assert!(n <= MAX_APP_NUM, "用户程序数量 {n} 超过上限 {MAX_APP_NUM}");

    // 用户程序区绝不能和内核镜像重叠。见第 6 章 6.8 节 ——
    // 这条断言是补上的，早期版本靠运气才没出事。
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
        "装载 {} 个用户程序（内核终点 {:#x}，程序区 {:#x}..{:#x}，栈区 {:#x}..{:#x}）：",
        n,
        kernel_end,
        APP_BASE_ADDRESS,
        app_region_end(),
        USER_STACK_BASE,
        crate::config::user_stack_region_end()
    );

    for app_id in 0..n {
        load_one(app_id);
    }

    // 我们刚把一大堆数据当作**代码**写进了内存。CPU 的取指单元可能还
    // 缓存着这些地址的旧内容，`fence.i` 让它丢弃指令缓存重新取指。
    //
    // SAFETY: fence.i 只影响取指缓存，没有其他副作用。
    unsafe {
        asm!("fence.i");
    }
}

fn load_one(app_id: usize) {
    let elf = parse_app(app_id);
    let base = app_base(app_id);
    let limit = base + APP_SIZE_LIMIT;

    // 先清空整个槽位。ELF 里 `memsz > filesz` 的那部分就是 `.bss`，
    // 它在文件里不占空间但运行时必须是 0 —— 清空槽位正好一并解决。
    // SAFETY: 槽位是内核独占的物理内存，此刻没有程序在运行。
    unsafe {
        core::slice::from_raw_parts_mut(base as *mut u8, APP_SIZE_LIMIT).fill(0);
    }

    let mut segments = 0;
    for ph in elf.program_headers().filter(|p| p.p_type == PT_LOAD) {
        // 用户程序是不可信输入。一个构造出来的 ELF 完全可以把段的
        // 目标地址指到内核里 —— 必须在拷贝**之前**拦住。
        let seg_end = ph
            .vaddr
            .checked_add(ph.memsz)
            .expect("段的目标地址溢出");
        assert!(
            ph.vaddr >= base && seg_end <= limit,
            "程序 {} 的段目标 [{:#x}, {:#x}) 越出了它的槽位 [{:#x}, {:#x})",
            app_id,
            ph.vaddr,
            seg_end,
            base,
            limit
        );

        let src = elf
            .segment_data(&ph)
            .expect("段内容超出了 ELF 文件范围");
        // SAFETY: 上面已验证目标区间完全落在本程序的槽位内。
        unsafe {
            core::slice::from_raw_parts_mut(ph.vaddr as *mut u8, src.len()).copy_from_slice(src);
        }

        let [r, w, x] = flags_str(ph.flags);
        debug!(
            "    段 {}{}{}  [{:#012x}, {:#012x})  文件 {} 字节，内存 {} 字节",
            r, w, x, ph.vaddr, seg_end, ph.filesz, ph.memsz
        );
        segments += 1;
    }

    assert!(segments > 0, "程序 {app_id} 没有任何可加载的段");
    info!(
        "  [{}] {} 个段，入口 {:#012x}，槽位 {:#012x}",
        app_id,
        segments,
        elf.entry(),
        base
    );
}

/// 构造第 `app_id` 个程序的初始 [`TrapContext`]，压到它自己的内核栈上，
/// 返回压入后的栈顶地址。
pub fn init_app_cx(app_id: usize) -> usize {
    let (_, stack_top) = user_stack_range(app_id);
    let cx = TrapContext::app_init_context(app_entry(app_id), stack_top);
    let ptr = (kernel_stack_top(app_id) - size_of::<TrapContext>()) as *mut TrapContext;
    // SAFETY: 内核栈 8 KiB，减去 TrapContext 的 272 字节后仍在栈内。
    unsafe {
        ptr.write(cx);
    }
    ptr as usize
}

/// 第 `app_id` 个程序能否合法访问 `[start, start + len)`。
///
/// 内核在系统调用入口用它验证用户传来的指针。
///
/// 从第 7 周起，硬件（页表的 `U` 位）已经能挡住用户**直接**解引用内核
/// 地址了。但这个软件检查依然要留着：用户完全可以把一个合法的内核地址
/// 当作**系统调用参数**传进来，那条路径绕过了 MMU 的 U 位检查 ——
/// 因为访存是内核自己做的，而内核当然有权限。
pub fn is_accessible_by(app_id: usize, start: usize, len: usize) -> bool {
    // 先防整数溢出：用户传 (start=usize::MAX-8, len=64) 的话，
    // start + len 会绕回一个很小的数，让后面的比较全部失效。
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let base = app_base(app_id);
    let (stack_bottom, stack_top) = user_stack_range(app_id);
    let in_own_slot = start >= base && end <= base + APP_SIZE_LIMIT;
    let in_own_stack = start >= stack_bottom && end <= stack_top;
    in_own_slot || in_own_stack
}

//! # CCNUOS
//!
//! 一个用 Rust 从零编写的 RISC-V (SV39) 教学操作系统内核。
//!
//! ## 第一周：从上电到 Rust
//!
//! QEMU virt 机器上电后的控制流是：
//!
//! ```text
//!   QEMU 复位向量  ──▶  OpenSBI (M 态, 0x80000000)  ──▶  CCNUOS (S 态, 0x80200000)
//!                                                          │
//!                                            entry.asm: 支起启动栈
//!                                                          │
//!                                                    rust_main()
//! ```
//!
//! 到达 [`rust_main`] 时，我们手上只有一个能用的栈。`.bss` 还是脏的，
//! 堆不存在，中断没开，页表没建。这一周就做三件事：清 `.bss`、
//! 打通输出、把自己的内存布局看明白。

#![no_std] // 不链接标准库：std 依赖操作系统，而我们就是操作系统
#![no_main] // 不要 Rust 的 main 入口：启动流程由 entry.asm 接管

use core::arch::global_asm;

#[macro_use]
mod console;
#[macro_use]
mod logging;
mod lang_items;
mod sbi;

// 把入口汇编整段嵌进来，让它参与链接
global_asm!(include_str!("entry.asm"));

// 链接脚本导出的布局符号。
//
// 这些符号没有"值"可言 —— 我们要的是它们的**地址**。所以声明成
// `static u8` 而不是 `fn`：前者如实表达了"内存中的一个位置"，
// 后者虽然也能用，但会让每个取址点都变成一次函数指针到整数的转换。
extern "C" {
    static skernel: u8;
    static stext: u8;
    static etext: u8;
    static srodata: u8;
    static erodata: u8;
    static sdata: u8;
    static edata: u8;
    static sbss: u8;
    static ebss: u8;
    static ekernel: u8;
    static boot_stack_lower_bound: u8;
    static boot_stack_top: u8;
}

/// 取一个链接符号的地址。
///
/// 用 `&raw const` 而不是 `&`：前者产生裸指针，不要求指向的内存是
/// 有效初始化的 —— 这正是链接符号的情况。也正因为只取址不解引用，
/// 这里连 `unsafe` 都不需要。
macro_rules! sym_addr {
    ($sym:ident) => {
        &raw const $sym as usize
    };
}

const BANNER: &str = r#"
   ____   ____  _   _  _   _   ___   ____
  / ___| / ___|| \ | || | | | / _ \ / ___|
 | |    | |    |  \| || | | || | | |\___ \
 | |___ | |___ | |\  || |_| || |_| | ___) |
  \____| \____||_| \_| \___/  \___/ |____/
"#;

/// 清零 `.bss` 段。
///
/// C 和 Rust 都假定未初始化的全局变量是 0，但这个"假定"是由运行时
/// 启动代码负责兑现的。裸机上没人替我们做，只能自己动手 —— 而且必须
/// 在任何 Rust 代码读到全局变量之前完成。
///
/// 注意范围是 `sbss..ebss` 而不是 `sbss_with_stack..ebss`：启动栈虽然
/// 也在 `.bss` 里，但我们此刻正站在这个栈上，清了就等于锯自己坐的树枝。
fn clear_bss() {
    (sym_addr!(sbss)..sym_addr!(ebss)).for_each(|addr| unsafe {
        (addr as *mut u8).write_volatile(0);
    });
}

/// 打印一行内存区间信息：名字、起止地址、大小。
fn print_section(name: &str, start: usize, end: usize) {
    let size = end - start;
    info!(
        "  {:<16} [{:#012x}, {:#012x})  {:>7} B  ({:>4} KiB)",
        name,
        start,
        end,
        size,
        size / 1024
    );
}

/// 打印内核的内存布局。
///
/// 这张表在后面几周会反复用到：第 6 周做页帧分配器时，
/// `ekernel` 就是可用物理内存的起点。
fn print_memory_layout() {
    info!("内核内存布局：");
    print_section(".text", sym_addr!(stext), sym_addr!(etext));
    print_section(".rodata", sym_addr!(srodata), sym_addr!(erodata));
    print_section(".data", sym_addr!(sdata), sym_addr!(edata));
    print_section(".bss", sym_addr!(sbss), sym_addr!(ebss));
    print_section(
        "boot_stack",
        sym_addr!(boot_stack_lower_bound),
        sym_addr!(boot_stack_top),
    );
    info!(
        "  {:<16} [{:#012x}, {:#012x})  共 {} KiB",
        "kernel total",
        sym_addr!(skernel),
        sym_addr!(ekernel),
        (sym_addr!(ekernel) - sym_addr!(skernel)) / 1024
    );
}

/// 内核的 Rust 入口。由 `entry.asm` 在支起栈之后调用，永不返回。
#[no_mangle]
pub fn rust_main() -> ! {
    // TODO: 第 1 章任务 1：在进入任何依赖全局变量的逻辑前清零 .bss。
    // clear_bss();
    let _ = clear_bss as fn();

    // TODO: 第 1 章任务 2：用 SBI 串口输出接好 println!/info!，
    // 打印课程要求的启动横幅、S 态入口确认和内存布局。
    let _ = BANNER;
    println!("CCNUOS v0.1 starter");

    // TODO: 第 1 章任务 3：调用 print_memory_layout()，确认链接脚本导出的
    // .text/.rodata/.data/.bss/boot_stack/kernel total 区间都正确。
    let _ = print_memory_layout as fn();

    // 当前 starter 会正常关机，但不会通过本章 selfcheck。
    // 完成上面的 TODO 后，提交 ch01 分支让平台判题。
    sbi::shutdown(false)
}

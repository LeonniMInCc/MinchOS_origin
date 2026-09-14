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

// `alloc` 是 core 与 std 之间的那一层：提供 Box/Vec/String/BTreeMap，
// 但不假定操作系统存在 —— 只要求你提供一个 #[global_allocator]。
// 第 2 周做的正是这件事，见 mm::heap_allocator。
extern crate alloc;

use core::arch::global_asm;

// console 和 logging 导出宏，必须排在使用它们的模块之前
#[macro_use]
mod console;
#[macro_use]
mod logging;
mod batch;
mod config;
mod csr;
mod lang_items;
mod mm;
mod sbi;
mod sync;
mod syscall;
mod trap;

// 把入口汇编整段嵌进来，让它参与链接
global_asm!(include_str!("entry.asm"));
// 由 build.rs 生成：把所有用户程序的二进制嵌进内核的 .data 段
global_asm!(include_str!("link_app.S"));

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
    clear_bss();

    println!("{}", BANNER);
    println!("       CCNUOS v0.3.0 — RISC-V 64 (SV39) 教学操作系统");
    println!("       第三周：特权级切换 / 系统调用 / 批处理\n");

    info!("内核已进入 S 态，启动栈可用，.bss 已清零");
    print_memory_layout();

    mm::init();
    mm::heap_test();

    trap::init();
    batch::init();

    // 一去不回：直接 sret 进 U 态运行第一个用户程序。
    // 此后内核只在 trap 发生时才会重新获得执行权。
    batch::run_next_app()
}

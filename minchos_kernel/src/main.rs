#![no_std]
#![no_main]

#[macro_use]
mod console;
mod lang_items;
mod sbi;

use core::arch::global_asm;

global_asm!(include_str!("entry.asm"));

extern "C" {
    fn stext();
    fn etext();
    fn srodata();
    fn erodata();
    fn sdata();
    fn edata();
    fn sbss();
    fn ebss();
    fn boot_stack_lower_bound();
    fn boot_stack_top();
    fn skernel();
    fn ekernel();
}

fn clear_bss() {
    ((sbss as *const () as usize)..(ebss as *const () as usize)).for_each(|addr| unsafe {
        (addr as *mut u8).write_volatile(0);
    });
}

fn print_section(name: &str, start: usize, end: usize) {
    let size = end - start;
    println!(
        "[INFO ]   {:<16} [0x{:010x}, 0x{:010x})  {:>8} B  ({:>4} KiB)",
        name, start, end, size, size / 1024
    );
}

#[no_mangle]
pub fn rust_main() -> ! {
    clear_bss();

    println!("");
    println!("   ____   ____  _   _  _   _   ___   ____  ");
    println!("  / ___| / ___|| \\ | || | | | / _ \\ / ___| ");
    println!(" | |    | |    |  \\| || | | || | | |\\___ \\ ");
    println!(" | |___ | |___ | |\\  || |_| || |_| | ___) |");
    println!("  \\____| \\____||_| \\_| \\___/  \\___/ |____/ ");
    println!("");
    println!("       CCNUOS v0.1.0 — RISC-V 64 (SV39) 教学操作系统");
    println!("       第一周：裸机启动 / SBI 输出 / 内存布局");
    println!("");

    println!("[INFO ] 内核已进入 S 态，启动栈可用，.bss 已清零");
    println!("[INFO ] 内核内存布局：");
    print_section(".text",       stext       as *const () as usize, etext       as *const () as usize);
    print_section(".rodata",     srodata     as *const () as usize, erodata     as *const () as usize);
    print_section(".data",       sdata       as *const () as usize, edata       as *const () as usize);
    print_section(".bss",        sbss        as *const () as usize, ebss        as *const () as usize);
    print_section("boot_stack",  boot_stack_lower_bound as *const () as usize, boot_stack_top as *const () as usize);
    
    let kernel_size = (ekernel as *const () as usize) - (skernel as *const () as usize);
    println!("[INFO ]   {:<16} [0x{:010x}, 0x{:010x})  共 {} KiB",
        "kernel total",
        skernel as *const () as usize,
        ekernel as *const () as usize,
        kernel_size / 1024
    );

    println!("[INFO ] 第一周目标达成，主动关机");
    sbi::shutdown();
}

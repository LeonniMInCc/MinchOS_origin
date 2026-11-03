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
    (sbss as usize..ebss as usize).for_each(|addr| unsafe {
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
    println!("       CCNUOS v0.1.0 — RISC-V 64 (SV39)");
    println!("       ch01: bare-metal boot / SBI console / memory layout");
    println!("");

    println!("[INFO ] .bss section cleared, boot stack ready");
    println!("[INFO ] Kernel memory layout:");
    print_section(".text",       stext       as usize, etext       as usize);
    print_section(".rodata",     srodata     as usize, erodata     as usize);
    print_section(".data",       sdata       as usize, edata       as usize);
    print_section(".bss",        sbss        as usize, ebss        as usize);
    print_section("boot_stack",  boot_stack_lower_bound as usize, boot_stack_top as usize);
    print_section("kernel total", skernel    as usize, ekernel     as usize);

    println!("[INFO ] ch01 complete, shutting down");
    sbi::shutdown();
}

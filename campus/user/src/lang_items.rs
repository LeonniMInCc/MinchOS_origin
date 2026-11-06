//! 用户态的 panic handler。
//!
//! 和内核版的关键区别：用户程序 panic 了只能**结束自己**，
//! 不能关机。关机是内核的特权 —— 用户程序连那条 SBI 调用都发不出去
//! （`ecall` 到 SBI 需要 S 态，用户程序在 U 态执行会触发非法指令异常）。

use crate::syscall::sys_exit;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    match info.location() {
        Some(location) => {
            println!(
                "[用户程序 panic] {}:{}:{} — {}",
                location.file(),
                location.line(),
                location.column(),
                info.message()
            );
        }
        None => {
            println!("[用户程序 panic] {}", info.message());
        }
    }
    sys_exit(-1)
}

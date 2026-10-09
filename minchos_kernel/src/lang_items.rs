//! panic 处理器
//!
//! 裸机环境没有操作系统来兜底，Rust 要求我们自己定义
//! panic 发生时该做什么。这里打印出错信息后关机。

use crate::sbi::shutdown;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    if let Some(location) = info.location() {
        println!(
            "[kernel] Panicked at {}:{} {}",
            location.file(),
            location.line(),
            info.message(),
        );
    } else {
        println!("[kernel] Panicked: {}", info.message());
    }
    shutdown()
}

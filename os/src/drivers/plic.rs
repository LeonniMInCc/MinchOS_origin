//! PLIC：平台级中断控制器。
//!
//! 第 5 周的时钟中断是 CPU 内部产生的，不需要控制器。外设中断不一样 ——
//! 一块板子上可能有几十个设备，PLIC 负责收集它们的中断请求、按优先级
//! 排队，再转交给某个 hart 的某个特权级。
//!
//! ```text
//!   UART ──┐
//!   磁盘 ──┼──▶ PLIC ──▶ hart 0 的 S 态
//!   网卡 ──┘
//! ```
//!
//! 处理一次外设中断的标准流程是**认领 + 完成**两步：
//!
//! 1. `claim`：读一下寄存器，PLIC 告诉你是几号中断，并把它标记为"处理中"
//! 2. `complete`：处理完写回同一个号，PLIC 才允许它再次触发
//!
//! 少了第 2 步的后果很典型：中断只会来一次，之后设备再怎么叫都没反应。

use crate::config::PAGE_SIZE;

/// QEMU virt 机器上 PLIC 的位置。
const PLIC_BASE: usize = 0x0c00_0000;
const PLIC_SIZE: usize = 0x60_0000;

pub const PLIC_REGION: (usize, usize) = (PLIC_BASE, PLIC_BASE + PLIC_SIZE);

/// hart 0 的 S 态在 PLIC 眼里是 1 号上下文（0 号是 M 态）。
const S_CONTEXT: usize = 1;

/// QEMU virt 上串口的中断号。
pub const IRQ_UART0: u32 = 10;

fn write_reg(offset: usize, value: u32) {
    // SAFETY: PLIC 的 MMIO 区域已在内核地址空间里恒等映射。
    unsafe { ((PLIC_BASE + offset) as *mut u32).write_volatile(value) }
}

fn read_reg(offset: usize) -> u32 {
    // SAFETY: 同上。
    unsafe { ((PLIC_BASE + offset) as *const u32).read_volatile() }
}

/// 打开某个中断源，让它能送到 S 态。
pub fn enable(irq: u32) {
    // 优先级必须**大于 0**，0 表示"永不触发"。这是最容易漏的一步。
    write_reg(irq as usize * 4, 1);

    // 使能位：每个上下文一张位图
    let offset = 0x2000 + S_CONTEXT * 0x80 + (irq as usize / 32) * 4;
    let bit = 1u32 << (irq % 32);
    write_reg(offset, read_reg(offset) | bit);

    // 阈值 0 = 只要优先级大于 0 就放行
    write_reg(0x20_0000 + S_CONTEXT * 0x1000, 0);
}

/// 认领一个待处理的中断。返回 0 表示没有。
pub fn claim() -> u32 {
    read_reg(0x20_0000 + S_CONTEXT * 0x1000 + 4)
}

/// 告诉 PLIC 这个中断处理完了。
///
/// **不调它的话，同一个中断源再也不会触发第二次。**
pub fn complete(irq: u32) {
    write_reg(0x20_0000 + S_CONTEXT * 0x1000 + 4, irq);
}

const _: () = assert!(PLIC_SIZE.is_multiple_of(PAGE_SIZE));

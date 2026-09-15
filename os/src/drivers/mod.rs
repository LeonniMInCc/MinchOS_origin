//! 设备驱动。
//!
//! 第 12 周只有一个：virtio-blk 块设备。
//!
//! 在此之前，内核接触"外设"的方式都是请 SBI 固件代劳（串口、时钟）。
//! 这是第一次内核直接跟硬件寄存器对话。

pub mod plic;
pub mod uart;
mod virtio_blk;

use crate::sync::UPSafeCell;
use alloc::sync::Arc;
use easy_fs::BlockDevice;

pub use virtio_blk::MMIO_REGION;
pub use plic::PLIC_REGION;

static BLOCK_DEVICE: UPSafeCell<Option<Arc<dyn BlockDevice>>> =
    // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
    unsafe { UPSafeCell::new(None) };

/// 探测并初始化块设备。找不到时返回 `false`，内核继续跑（只是没有磁盘）。
pub fn init() -> bool {
    match virtio_blk::VirtIOBlk::probe() {
        Some(dev) => {
            *BLOCK_DEVICE.exclusive_access() = Some(Arc::new(dev));
            true
        }
        None => {
            warn!("没有找到 virtio-blk 设备（QEMU 命令行里加了 -drive 吗？）");
            false
        }
    }
}

/// 全局块设备。必须在 [`init`] 成功之后才能调。
pub fn block_device() -> Arc<dyn BlockDevice> {
    BLOCK_DEVICE
        .exclusive_access()
        .as_ref()
        .expect("块设备还没初始化")
        .clone()
}

/// 主动问一次 PLIC 有没有待处理的外设中断，有就处理掉。
///
/// # 为什么 idle 必须主动去问
///
/// 内核态 `sstatus.SIE = 0`（第 5 章的决定：内核不可抢占），
/// 所以中断在内核里只会**挂起**，不会被送达。
///
/// 平时这不成问题：内核很快就 `sret` 回用户态，挂起的中断立刻被处理。
/// 但当**所有进程都阻塞**时，CPU 停在 idle 的 `wfi` 上 ——
/// 没有用户态可回，挂起的中断就永远没人管，系统彻底僵住。
///
/// 所以 idle 要自己来这一趟。`wfi` 负责"别空转"，这个函数负责"别睡死"。
///
/// 返回是否真的处理了什么。
pub fn poll_external() -> bool {
    let irq = plic::claim();
    if irq == 0 {
        return false;
    }
    if irq == plic::IRQ_UART0 {
        uart::handle_interrupt();
    } else {
        warn!("收到未处理的外部中断 {}", irq);
    }
    plic::complete(irq);
    true
}

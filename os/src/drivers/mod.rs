//! 设备驱动。
//!
//! 第 12 周只有一个：virtio-blk 块设备。
//!
//! 在此之前，内核接触"外设"的方式都是请 SBI 固件代劳（串口、时钟）。
//! 这是第一次内核直接跟硬件寄存器对话。

mod virtio_blk;

use crate::sync::UPSafeCell;
use alloc::sync::Arc;
use easy_fs::BlockDevice;

pub use virtio_blk::MMIO_REGION;

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

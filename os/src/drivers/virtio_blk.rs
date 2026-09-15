//! virtio-blk 块设备驱动（MMIO 传输层，同步轮询）。
//!
//! 这是整个项目里第一个**真正的设备驱动**。前面所有的"外设"其实都是
//! 通过 SBI 请固件代劳的（串口、时钟），只有这里是内核直接跟硬件说话。
//!
//! # virtqueue：virtio 的核心
//!
//! 驱动和设备之间靠三张表通信，合称一个 virtqueue：
//!
//! ```text
//!   描述符表 (desc)    一个个"内存片段"：地址、长度、标志、下一个是谁
//!        ▲                    ▲
//!        │                    │
//!   可用环 (avail)        已用环 (used)
//!   驱动写，设备读        设备写，驱动读
//! ```
//!
//! 一次请求的流程：
//!
//! 1. 驱动在 **desc** 里串起几个片段（请求头 → 数据缓冲 → 状态字节）
//! 2. 把这条链的头部编号写进 **avail** 环，`avail.idx += 1`
//! 3. 往 `QueueNotify` 寄存器写一下，踢设备一脚
//! 4. 设备处理完，把编号放进 **used** 环，`used.idx += 1`
//! 5. 驱动看到 `used.idx` 变了，就知道好了
//!
//! 这套"环形队列 + 索引"的设计让驱动和设备可以完全异步地跑。
//! 我们这里为了简单用**轮询**等第 4 步 —— 反正文件系统操作本来就不频繁，
//! 而且引入设备中断需要 PLIC 驱动（第 14 章的事）。
//!
//! # DMA 缓冲区不能随便给
//!
//! 设备做 DMA 用的是**物理地址**，它不认识页表。内核的 `.text`/`.data`/`.bss`
//! 和整个物理内存都是**恒等映射**的（第 7 章），所以这些地方的指针数值
//! 本身就是物理地址，可以直接交给设备。
//!
//! **但内核栈不是。** 从第 8 章起，每个进程的内核栈是在内核地址空间的
//! 高地址处按需映射（Framed）出来的 —— 虚拟地址 `0xffff_ffff_ffff_c000`
//! 之类，和物理地址毫无关系。把一个栈上的缓冲区交给设备，
//! 设备会往那个根本不存在的物理地址写，而调用方看到的缓冲区**保持全零**。
//!
//! 这个 bug 极其隐蔽：内核初始化阶段跑在**启动栈**上（它在 `.bss` 里，
//! 是恒等映射的），一切正常；一旦进入进程上下文，同样的代码就开始读到
//! 全零的块。第 13 章就是这么被坑了一次。
//!
//! 解法是一块**静态的中转缓冲区**（bounce buffer）：DMA 只对着它做，
//! 前后再拷贝一次。真实驱动在缓冲区不可 DMA 时干的也是同一件事。

use crate::config::PAGE_SIZE;
use core::sync::atomic::{fence, Ordering};
use easy_fs::{BlockDevice, BLOCK_SZ};

// ---------------------------------------------------------------------------
// MMIO 寄存器
// ---------------------------------------------------------------------------

/// QEMU virt 机器把 8 个 virtio-mmio 槽位放在这里，每个占一页。
const MMIO_BASE: usize = 0x1000_1000;
const MMIO_STRIDE: usize = 0x1000;
const MMIO_SLOTS: usize = 8;

/// MMIO 区域的总范围，内核地址空间要把它映射进来。
pub const MMIO_REGION: (usize, usize) = (MMIO_BASE, MMIO_BASE + MMIO_SLOTS * MMIO_STRIDE);

const MAGIC: usize = 0x000;
const VERSION: usize = 0x004;
const DEVICE_ID: usize = 0x008;
const DEVICE_FEATURES: usize = 0x010;
const DEVICE_FEATURES_SEL: usize = 0x014;
const DRIVER_FEATURES: usize = 0x020;
const DRIVER_FEATURES_SEL: usize = 0x024;
const GUEST_PAGE_SIZE: usize = 0x028; // 仅 legacy
const QUEUE_SEL: usize = 0x030;
const QUEUE_NUM_MAX: usize = 0x034;
const QUEUE_NUM: usize = 0x038;
const QUEUE_ALIGN: usize = 0x03c; // 仅 legacy
const QUEUE_PFN: usize = 0x040; // 仅 legacy
const QUEUE_READY: usize = 0x044; // 仅 modern
const QUEUE_NOTIFY: usize = 0x050;
const STATUS: usize = 0x070;
const QUEUE_DESC_LOW: usize = 0x080; // 仅 modern
const QUEUE_DESC_HIGH: usize = 0x084;
const QUEUE_DRIVER_LOW: usize = 0x090;
const QUEUE_DRIVER_HIGH: usize = 0x094;
const QUEUE_DEVICE_LOW: usize = 0x0a0;
const QUEUE_DEVICE_HIGH: usize = 0x0a4;
const CONFIG: usize = 0x100;

/// `"virt"` 的小端 ASCII。
const VIRTIO_MAGIC: u32 = 0x7472_6976;
/// 设备类型 2 = 块设备。
const DEVICE_ID_BLOCK: u32 = 2;

// 设备状态位
const STATUS_ACKNOWLEDGE: u32 = 1;
const STATUS_DRIVER: u32 = 2;
const STATUS_DRIVER_OK: u32 = 4;
const STATUS_FEATURES_OK: u32 = 8;

// ---------------------------------------------------------------------------
// virtqueue 的内存布局
// ---------------------------------------------------------------------------

const QUEUE_SIZE: usize = 16;

const DESC_F_NEXT: u16 = 1;
/// 这个片段是**设备写、驱动读**的（也就是读磁盘时的数据缓冲）。
const DESC_F_WRITE: u16 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[repr(C)]
struct AvailRing {
    flags: u16,
    idx: u16,
    ring: [u16; QUEUE_SIZE],
    used_event: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UsedElem {
    id: u32,
    len: u32,
}

#[repr(C)]
struct UsedRing {
    flags: u16,
    idx: u16,
    ring: [UsedElem; QUEUE_SIZE],
    avail_event: u16,
}

/// 一整个 virtqueue 的内存。
///
/// 布局是 virtio 规范规定死的：描述符表和可用环挨着放，
/// **已用环必须对齐到一页**。所以中间要填一段空白。
///
/// 放在 `.bss` 里（而不是从页帧分配器要）有两个好处：
/// 它天然是物理连续的，而且因为恒等映射，它的虚拟地址就是物理地址。
#[repr(C, align(4096))]
struct VirtQueue {
    desc: [Descriptor; QUEUE_SIZE],
    avail: AvailRing,
    _pad: [u8; PAGE_SIZE - QUEUE_SIZE * 16 - (4 + QUEUE_SIZE * 2 + 2)],
    used: UsedRing,
}

static mut QUEUE: VirtQueue = VirtQueue {
    desc: [Descriptor {
        addr: 0,
        len: 0,
        flags: 0,
        next: 0,
    }; QUEUE_SIZE],
    avail: AvailRing {
        flags: 0,
        idx: 0,
        ring: [0; QUEUE_SIZE],
        used_event: 0,
    },
    _pad: [0; PAGE_SIZE - QUEUE_SIZE * 16 - (4 + QUEUE_SIZE * 2 + 2)],
    used: UsedRing {
        flags: 0,
        idx: 0,
        ring: [UsedElem { id: 0, len: 0 }; QUEUE_SIZE],
        avail_event: 0,
    },
};

// 编译期核对：已用环必须正好落在第二页的开头。
const _: () = assert!(core::mem::offset_of!(VirtQueue, used) == PAGE_SIZE);

// ---------------------------------------------------------------------------
// 块请求
// ---------------------------------------------------------------------------

const BLK_T_IN: u32 = 0; // 读
const BLK_T_OUT: u32 = 1; // 写

#[repr(C)]
struct BlkReqHeader {
    type_: u32,
    reserved: u32,
    sector: u64,
}

static mut REQ_HEADER: BlkReqHeader = BlkReqHeader {
    type_: 0,
    reserved: 0,
    sector: 0,
};

/// 设备写回的状态字节：0 = 成功。
static mut REQ_STATUS: u8 = 0xff;

/// DMA 中转缓冲区。
///
/// 调用方给的缓冲区可能在内核栈上，而内核栈不是恒等映射的
/// （见模块文档）。所以一律先中转一次：读的时候设备写这里、我们再拷出去，
/// 写的时候我们先拷进来、再让设备读。
///
/// 它在 `.bss` 里，所以地址即物理地址，而且天然不跨页（512 字节且对齐）。
#[repr(C, align(512))]
struct Bounce([u8; BLOCK_SZ]);

static mut BOUNCE: Bounce = Bounce([0; BLOCK_SZ]);

// ---------------------------------------------------------------------------
// 驱动
// ---------------------------------------------------------------------------

pub struct VirtIOBlk {
    base: usize,
    /// MMIO 传输层版本：1 = legacy，2 = modern。两者的队列配置方式不同。
    version: u32,
    /// 容量，单位 512 字节扇区。
    capacity: u64,
}

impl VirtIOBlk {
    fn read_reg(&self, offset: usize) -> u32 {
        // SAFETY: base 是探测出来的合法 MMIO 槽位，已在内核地址空间里映射。
        unsafe { ((self.base + offset) as *const u32).read_volatile() }
    }

    fn write_reg(&self, offset: usize, value: u32) {
        // SAFETY: 同上。
        unsafe { ((self.base + offset) as *mut u32).write_volatile(value) }
    }

    /// 扫描 8 个槽位，找出块设备。
    pub fn probe() -> Option<Self> {
        for slot in 0..MMIO_SLOTS {
            let base = MMIO_BASE + slot * MMIO_STRIDE;
            // SAFETY: virt 机器保证这片 MMIO 区域存在且已映射。
            let magic = unsafe { ((base + MAGIC) as *const u32).read_volatile() };
            if magic != VIRTIO_MAGIC {
                continue;
            }
            let device_id = unsafe { ((base + DEVICE_ID) as *const u32).read_volatile() };
            if device_id != DEVICE_ID_BLOCK {
                continue; // 空槽位的 device_id 是 0
            }
            let version = unsafe { ((base + VERSION) as *const u32).read_volatile() };

            let mut dev = Self {
                base,
                version,
                capacity: 0,
            };
            dev.init()?;
            info!(
                "virtio-blk：槽位 {}（{:#x}），MMIO v{}，容量 {} 扇区（{} KiB）",
                slot,
                base,
                version,
                dev.capacity,
                dev.capacity * 512 / 1024
            );
            return Some(dev);
        }
        None
    }

    /// 按 virtio 规范做初始化握手。
    fn init(&mut self) -> Option<()> {
        // 规范规定的顺序，一步都不能乱：设备是按这个状态机来判断
        // 驱动进行到哪一步的。
        self.write_reg(STATUS, 0); // 复位
        let mut status = STATUS_ACKNOWLEDGE; // "我看到你了"
        self.write_reg(STATUS, status);
        status |= STATUS_DRIVER; // "我知道怎么驱动你"
        self.write_reg(STATUS, status);

        // 特性协商。我们一个可选特性都不要 —— 基础的读写功能不需要任何
        // 特性位，而少一个特性就少一堆要处理的情况。
        self.write_reg(DEVICE_FEATURES_SEL, 0);
        let _features = self.read_reg(DEVICE_FEATURES);
        self.write_reg(DRIVER_FEATURES_SEL, 0);
        self.write_reg(DRIVER_FEATURES, 0);

        if self.version >= 2 {
            // modern 才有的"特性已确定"握手
            status |= STATUS_FEATURES_OK;
            self.write_reg(STATUS, status);
            if self.read_reg(STATUS) & STATUS_FEATURES_OK == 0 {
                error!("virtio-blk：设备拒绝了我们的特性集");
                return None;
            }
        }

        self.setup_queue()?;

        status |= STATUS_DRIVER_OK; // "我准备好了"
        self.write_reg(STATUS, status);

        // 容量在配置空间开头，单位是 512 字节扇区
        let lo = self.read_reg(CONFIG) as u64;
        let hi = self.read_reg(CONFIG + 4) as u64;
        self.capacity = (hi << 32) | lo;

        Some(())
    }

    /// 告诉设备 virtqueue 在哪。legacy 和 modern 的做法完全不同。
    fn setup_queue(&mut self) -> Option<()> {
        self.write_reg(QUEUE_SEL, 0);

        let max = self.read_reg(QUEUE_NUM_MAX);
        if max == 0 {
            error!("virtio-blk：0 号队列不存在");
            return None;
        }
        if (max as usize) < QUEUE_SIZE {
            error!("virtio-blk：设备队列上限 {} 小于我们要的 {}", max, QUEUE_SIZE);
            return None;
        }
        self.write_reg(QUEUE_NUM, QUEUE_SIZE as u32);

        let queue_addr = &raw const QUEUE as usize;

        if self.version == 1 {
            // legacy：整个队列必须是**页对齐且连续**的，告诉设备页框号即可。
            self.write_reg(GUEST_PAGE_SIZE, PAGE_SIZE as u32);
            self.write_reg(QUEUE_ALIGN, PAGE_SIZE as u32);
            self.write_reg(QUEUE_PFN, (queue_addr / PAGE_SIZE) as u32);
        } else {
            // modern：三张表可以分开放，各自给 64 位物理地址。
            // 我们还是放在一起，只是分别把地址告诉设备。
            let desc = queue_addr;
            let avail = queue_addr + core::mem::offset_of!(VirtQueue, avail);
            let used = queue_addr + core::mem::offset_of!(VirtQueue, used);

            self.write_reg(QUEUE_DESC_LOW, desc as u32);
            self.write_reg(QUEUE_DESC_HIGH, (desc >> 32) as u32);
            self.write_reg(QUEUE_DRIVER_LOW, avail as u32);
            self.write_reg(QUEUE_DRIVER_HIGH, (avail >> 32) as u32);
            self.write_reg(QUEUE_DEVICE_LOW, used as u32);
            self.write_reg(QUEUE_DEVICE_HIGH, (used >> 32) as u32);
            self.write_reg(QUEUE_READY, 1);
        }

        Some(())
    }

    /// 提交一次读或写，同步等它完成。数据经由 [`BOUNCE`] 中转。
    ///
    /// # Safety
    ///
    /// 只能有一个调用同时进行 —— 请求头、状态字节和中转缓冲区都是静态的。
    /// 本项目的内核不可抢占，而这个函数中间不会让出 CPU，所以成立。
    unsafe fn request(&self, block_id: usize, write: bool) {
        assert!(
            (block_id as u64) < self.capacity,
            "块号 {} 超出设备容量 {}",
            block_id,
            self.capacity
        );

        let header = &raw mut REQ_HEADER;
        let status = &raw mut REQ_STATUS;
        let queue = &raw mut QUEUE;

        (*header).type_ = if write { BLK_T_OUT } else { BLK_T_IN };
        (*header).reserved = 0;
        (*header).sector = block_id as u64;
        *status = 0xff; // 先写一个"不可能"的值，好确认设备真的改过它

        // 三段式的请求链：请求头 → 数据 → 状态字节。
        // 前两段的方向不同 —— 读磁盘时数据段是设备写、驱动读，
        // 所以要打上 DESC_F_WRITE；写磁盘时反过来。
        (*queue).desc[0] = Descriptor {
            addr: header as u64,
            len: core::mem::size_of::<BlkReqHeader>() as u32,
            flags: DESC_F_NEXT,
            next: 1,
        };
        (*queue).desc[1] = Descriptor {
            addr: &raw const BOUNCE as u64,
            len: BLOCK_SZ as u32,
            flags: if write {
                DESC_F_NEXT
            } else {
                DESC_F_NEXT | DESC_F_WRITE
            },
            next: 2,
        };
        (*queue).desc[2] = Descriptor {
            addr: status as u64,
            len: 1,
            flags: DESC_F_WRITE,
            next: 0,
        };

        let used_before = (*queue).used.idx;

        // 把链头（0 号描述符）放进可用环
        let slot = ((*queue).avail.idx as usize) % QUEUE_SIZE;
        (*queue).avail.ring[slot] = 0;

        // 屏障：必须保证上面所有的写都对设备可见之后，才更新 idx。
        // 少了它，设备可能看到一个已经递增的 idx，却读到还没写完的描述符。
        fence(Ordering::SeqCst);
        (*queue).avail.idx = (*queue).avail.idx.wrapping_add(1);
        fence(Ordering::SeqCst);

        // 踢设备一脚
        self.write_reg(QUEUE_NOTIFY, 0);

        // 轮询等完成。没有超时保护 —— 设备卡住的话这里会死循环，
        // 但那已经是硬件/模拟器出问题了，加超时也救不回来。
        let used_idx = &raw const (*queue).used.idx;
        while used_idx.read_volatile() == used_before {
            core::hint::spin_loop();
        }
        fence(Ordering::SeqCst);

        let st = status.read_volatile();
        assert_eq!(
            st, 0,
            "virtio-blk 请求失败：块 {}，状态字节 {}",
            block_id, st
        );
    }
}

impl BlockDevice for VirtIOBlk {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        assert_eq!(buf.len(), BLOCK_SZ, "块缓冲区大小必须正好是 {BLOCK_SZ}");
        // SAFETY: 内核不可抢占，这段不会让出 CPU，所以静态缓冲区独占。
        unsafe {
            self.request(block_id, false);
            // 用裸指针拷贝，不去构造指向 static mut 的引用
            core::ptr::copy_nonoverlapping(
                (&raw const BOUNCE).cast::<u8>(),
                buf.as_mut_ptr(),
                BLOCK_SZ,
            );
        }
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) {
        assert_eq!(buf.len(), BLOCK_SZ, "块缓冲区大小必须正好是 {BLOCK_SZ}");
        // SAFETY: 同上。
        unsafe {
            core::ptr::copy_nonoverlapping(
                buf.as_ptr(),
                (&raw mut BOUNCE).cast::<u8>(),
                BLOCK_SZ,
            );
            self.request(block_id, true);
        }
    }
}

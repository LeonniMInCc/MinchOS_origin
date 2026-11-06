//! 串口输入：16550 UART 的接收中断 + 环形缓冲区。
//!
//! # 这是在还第 10 章的账
//!
//! 第 10 章的 `Stdin::read` 是这么"等"输入的：
//!
//! ```text
//!   loop {
//!       SBI 问一下有没有字符 → 有就返回，没有就 yield
//!   }
//! ```
//!
//! 两个问题：
//!
//! 1. **浪费**：等待的进程反复被调度、什么也干不成
//! 2. **丢字符**：两次轮询之间到达的字符，超出串口硬件那点 FIFO 就没了。
//!    症状是粘贴一段长命令会缺字母 —— 第 10 章实测到过
//!
//! 正确做法是让**硬件来通知我们**：串口收到字符时产生一个中断，
//! 内核在中断处理程序里把它存进环形缓冲区，并唤醒等着的进程。
//! 于是没有轮询、也没有窗口期。

use super::plic;
use crate::sync::{notify_all, UPSafeCell, WaitQueue};

/// QEMU virt 上 16550 串口的位置。
const UART_BASE: usize = 0x1000_0000;

/// 接收缓冲寄存器（读）
const RBR: usize = 0;
/// 中断使能寄存器
const IER: usize = 1;
/// 线路状态寄存器
const LSR: usize = 5;
/// `LSR` 的第 0 位：有数据可读
const LSR_DATA_READY: u8 = 1 << 0;
/// `IER` 的第 0 位：收到数据时产生中断
const IER_RX_AVAILABLE: u8 = 1 << 0;

fn read_reg(offset: usize) -> u8 {
    // SAFETY: UART 的 MMIO 区域已在内核地址空间里恒等映射。
    unsafe { ((UART_BASE + offset) as *const u8).read_volatile() }
}

fn write_reg(offset: usize, value: u8) {
    // SAFETY: 同上。
    unsafe { ((UART_BASE + offset) as *mut u8).write_volatile(value) }
}

/// 输入缓冲区能存多少字符。
///
/// 开得比串口硬件 FIFO 大得多，这样即使内核一时顾不上，
/// 粘贴进来的一大段命令也不会丢。
const BUFFER_SIZE: usize = 256;

struct RingBuffer {
    data: [u8; BUFFER_SIZE],
    head: usize,
    tail: usize,
    len: usize,
    /// 丢弃的字符数。缓冲区满了还继续来字符时累加 —— 有了这个数字，
    /// "丢字符"就从一个诡异现象变成了一个可观测的指标。
    dropped: usize,
}

impl RingBuffer {
    const fn new() -> Self {
        Self {
            data: [0; BUFFER_SIZE],
            head: 0,
            tail: 0,
            len: 0,
            dropped: 0,
        }
    }

    fn push(&mut self, byte: u8) {
        if self.len == BUFFER_SIZE {
            self.dropped += 1;
            return;
        }
        self.data[self.tail] = byte;
        self.tail = (self.tail + 1) % BUFFER_SIZE;
        self.len += 1;
    }

    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let byte = self.data[self.head];
        self.head = (self.head + 1) % BUFFER_SIZE;
        self.len -= 1;
        Some(byte)
    }
}

static BUFFER: UPSafeCell<RingBuffer> =
    // SAFETY: 单核；中断处理程序和普通内核代码不会同时运行
    // （内核不可抢占，见第 5 章）。
    unsafe { UPSafeCell::new(RingBuffer::new()) };

/// 等着有输入的进程。
static READERS: WaitQueue = WaitQueue::new();

/// 打开串口的接收中断。
pub fn init() {
    write_reg(IER, read_reg(IER) | IER_RX_AVAILABLE);
    plic::enable(plic::IRQ_UART0);

    // 把此刻已经躺在硬件 FIFO 里的字节收走。
    //
    // 内核启动要花一点时间，这期间用户（或者自动化脚本）完全可能已经
    // 敲进来了几个字符。它们到达时中断还没打开，不会有人来通知我们 ——
    // 不主动捞一把的话，这几个字节就永远沉在 FIFO 里了。
    //
    // 实测：不加这一步，喂进来的第一条命令会稳定地丢掉首字母。
    handle_interrupt();

    info!("串口输入已切换为中断驱动（缓冲区 {} 字节）", BUFFER_SIZE);
}

/// 串口中断处理：把硬件 FIFO 里的字符全部搬进缓冲区，然后唤醒等待者。
///
/// **必须一次性排空 FIFO**。只取一个字符的话，剩下的不会再产生新中断
/// （16550 的中断是电平触发的），于是系统就此失聪。
pub fn handle_interrupt() {
    let mut got = 0;
    {
        let mut buffer = BUFFER.exclusive_access();
        while read_reg(LSR) & LSR_DATA_READY != 0 {
            buffer.push(read_reg(RBR));
            got += 1;
        }
        // 借用必须在这里结束 —— 下面要唤醒进程，而唤醒会去动别的
        // UPSafeCell。这条规矩从第 10 章起反复出现。
    }
    if got > 0 {
        notify_all(&READERS);
    }
}

/// 取一个字符。缓冲区空时返回 `None`。
pub fn try_getchar() -> Option<u8> {
    BUFFER.exclusive_access().pop()
}

/// 把当前进程挂进"等输入"的队列。调用方随后要阻塞自己。
pub fn reader_queue() -> &'static WaitQueue {
    &READERS
}

/// 因为缓冲区满而丢掉的字符数。
pub fn dropped() -> usize {
    BUFFER.exclusive_access().dropped
}

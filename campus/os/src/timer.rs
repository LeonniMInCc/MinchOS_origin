//! 时钟。
//!
//! 第 4 周的协作式调度有个致命弱点：任务不主动 `yield`，内核就拿它没办法。
//! 一个 `loop {}` 能把整个系统卡死。
//!
//! 解决办法是给 CPU 装个闹钟。每隔一个时间片，硬件强行把控制权交还内核，
//! 内核就能把 CPU 从任何任务手里夺回来 —— 不管它愿不愿意。
//! 这就是**抢占式调度**。
//!
//! # 时间是怎么来的
//!
//! RISC-V 提供一个只读的 `time` CSR，每个时钟周期加一。QEMU virt 上它的
//! 频率是 10 MHz（开机时 OpenSBI 打印的 `aclint-mtimer @ 10000000Hz`）。
//!
//! 而"什么时候响"由 `mtimecmp` 寄存器决定：当 `time >= mtimecmp` 时触发
//! 时钟中断。但 `mtimecmp` 是 M 态资源，S 态的内核碰不到，只能通过
//! SBI 的 `set_timer` 请固件代劳。

use crate::csr::time;
use crate::sbi::set_timer;

/// QEMU virt 机器的时钟频率：10 MHz。
///
/// 这个数字来自开机时 OpenSBI 打印的
/// `Platform Timer Device : aclint-mtimer @ 10000000Hz`。
/// 换到别的平台（真实开发板）必须改，否则时间片长度会整体偏掉。
const CLOCK_FREQ: usize = 10_000_000;

/// 每秒触发多少次时钟中断。
///
/// 100 次 ⇒ 时间片 10 ms，和 Linux 的默认 `HZ=100` 一致。
///
/// 这个值是个权衡：调大了响应更及时，但每次中断都要保存/恢复现场、
/// 走一遍调度器，开销占比上升；调小了开销低，但一个任务可能霸占
/// CPU 很久，交互会卡顿。
const TICKS_PER_SEC: usize = 100;

const MSEC_PER_SEC: usize = 1_000;

/// 读取当前的时钟计数。
pub fn get_time() -> usize {
    time::read()
}

/// 当前时间，单位毫秒。
///
/// 先除后乘可以避免 `time * 1000` 溢出 —— 虽然 64 位下要跑几万年
/// 才会真的溢出，但养成习惯没坏处。
pub fn get_time_ms() -> usize {
    get_time() / (CLOCK_FREQ / MSEC_PER_SEC)
}

/// 设定下一次时钟中断：从现在起一个时间片之后。
///
/// **每次处理完时钟中断都必须再调一次**。SBI 的定时器是一次性的，
/// 响过就没了 —— 忘了重设，时间片轮转就此停摆，而且症状是
/// "第一次抢占成功，之后再也不抢占了"，很难一眼看出来。
pub fn set_next_trigger() {
    set_timer(get_time() + CLOCK_FREQ / TICKS_PER_SEC);
}

/// 时间片长度，单位毫秒。仅用于打印信息。
pub const fn slice_ms() -> usize {
    MSEC_PER_SEC / TICKS_PER_SEC
}

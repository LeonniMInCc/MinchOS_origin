//! SBI (Supervisor Binary Interface) 调用封装。
//!
//! CCNUOS 内核跑在 S 态。关机、操作串口、设置时钟这些需要 M 态权限的动作，
//! 内核自己干不了，必须通过 `ecall` 指令陷入到下层的 SBI 固件
//! （这里是 QEMU 自带的 OpenSBI）请它代劳。
//!
//! 这层封装刻意不引入 `sbi-rt` 等外部 crate —— 手写 `ecall` 只有几十行，
//! 但能让人清楚看见"内核向固件请求服务"这件事到底是怎么发生的。
//!
//! ## 调用约定
//!
//! | 寄存器 | 含义                     |
//! |--------|--------------------------|
//! | `a7`   | Extension ID (EID)       |
//! | `a6`   | Function ID (FID)        |
//! | `a0`~`a5` | 参数                  |
//! | `a0`   | 返回的错误码 (SbiRet.error) |
//! | `a1`   | 返回值 (SbiRet.value)      |

use core::arch::asm;

/// Legacy 扩展：向串口写一个字符。
const EID_LEGACY_CONSOLE_PUTCHAR: usize = 0x01;
/// Legacy 扩展：关机。作为 SRST 不可用时的兜底。
const EID_LEGACY_SHUTDOWN: usize = 0x08;
/// SRST 扩展 ("SRST" 的 ASCII)：系统复位/关机。
const EID_SRST: usize = 0x5352_5354;
/// TIME 扩展 ("TIME" 的 ASCII)：设置时钟中断的触发时刻。
const EID_TIME: usize = 0x5449_4D45;

/// 执行一次 SBI 调用，返回 `(error, value)`。
///
/// # Safety
///
/// `ecall` 会把控制权交给固件。调用方必须保证 EID/FID 与参数的组合
/// 是 SBI 规范中定义过的，否则固件的行为未定义。
#[inline(always)]
fn sbi_call(eid: usize, fid: usize, arg0: usize, arg1: usize, arg2: usize) -> (usize, usize) {
    let (error, value);
    // SAFETY: 调用方按上面的 # Safety 保证了 EID/FID 组合合法。
    // 寄存器约定严格遵循 SBI 规范：参数 a0-a2，FID 在 a6，EID 在 a7，
    // 返回值 a0 = error、a1 = value。
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") arg0 => error,
            inlateout("a1") arg1 => value,
            in("a2") arg2,
            in("a6") fid,
            in("a7") eid,
        );
    }
    (error, value)
}

/// 向 SBI 串口输出一个字符。
///
/// 这是 CCNUOS 在实现自己的串口驱动之前，唯一的输出手段。
pub fn console_putchar(c: usize) {
    sbi_call(EID_LEGACY_CONSOLE_PUTCHAR, 0, c, 0, 0);
}

// 第 14 周起输入不再走 SBI。
//
// SBI 的 `console_getchar` 是非阻塞的"看一眼"接口，内核只能轮询取字符，
// 两次轮询之间到达的字符会丢（第 10 章实测过：粘贴长命令会缺字母）。
// 现在内核直接驱动 16550 串口并打开接收中断，见 `drivers::uart`。
//
// 输出仍然走 SBI —— 那边是同步的，没有丢数据的问题，
// 而且省掉了一个发送驱动。

/// 设定下一次时钟中断的触发时刻（以 `time` 计数器的绝对值表示）。
///
/// 为什么要求固件代劳？定时器比较寄存器 `mtimecmp` 是 M 态资源，
/// S 态的内核碰不到 —— 只能通过 SBI 请 OpenSBI 去写。
///
/// 注意这是**一次性**的：闹钟响过就没了。所以每次处理完时钟中断，
/// 必须立刻再设下一次，否则时间片轮转就此停摆。
pub fn set_timer(stime_value: usize) {
    sbi_call(EID_TIME, 0, stime_value, 0, 0);
}

/// 关机。`failure` 为真时告诉固件这是一次异常退出。
///
/// 优先用 SRST 扩展；老固件上 SRST 会返回 `ERR_NOT_SUPPORTED`，
/// 这时退回到已废弃但普遍可用的 legacy shutdown。
pub fn shutdown(failure: bool) -> ! {
    const RESET_TYPE_SHUTDOWN: usize = 0;
    const RESET_REASON_NONE: usize = 0;
    const RESET_REASON_FAILURE: usize = 1;

    let reason = if failure {
        RESET_REASON_FAILURE
    } else {
        RESET_REASON_NONE
    };

    sbi_call(EID_SRST, 0, RESET_TYPE_SHUTDOWN, reason, 0);
    // 走到这里说明 SRST 没被支持，用 legacy 再试一次。
    sbi_call(EID_LEGACY_SHUTDOWN, 0, 0, 0, 0);

    unreachable!("SBI shutdown 失败：固件既不支持 SRST 也不支持 legacy shutdown");
}

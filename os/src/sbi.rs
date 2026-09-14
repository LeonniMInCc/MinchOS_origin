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

/// 执行一次 SBI 调用，返回 `(error, value)`。
///
/// # Safety
///
/// `ecall` 会把控制权交给固件。调用方必须保证 EID/FID 与参数的组合
/// 是 SBI 规范中定义过的，否则固件的行为未定义。
#[inline(always)]
fn sbi_call(eid: usize, fid: usize, arg0: usize, arg1: usize, arg2: usize) -> (usize, usize) {
    let (error, value);
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

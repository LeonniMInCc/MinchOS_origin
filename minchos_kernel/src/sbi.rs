//! SBI (Supervisor Binary Interface) 调用封装
//!
//! 本实验选择让 S 态内核通过 ecall 使用固件的输出和关机服务，
//! 向 M 态的 OpenSBI 发出请求。S 态是否能直接访问设备取决于权限与映射。此处将 ecall 的
//! 汇编细节包装成 Rust 函数，使上层代码只需调用普通函数即可。

use core::arch::asm;

/// SBI 调用返回值
#[repr(C)]
struct SbiRet {
    /// 错误码，0 表示成功
    error: usize,
    /// 返回值
    value: usize,
}

/// 发起一次 SBI 调用（最多 3 个参数）
///
/// 参数通过 a0-a2 传入，功能号通过 a6 (fid) 和 a7 (eid) 指定。
/// 返回值从 a0 (error) 和 a1 (value) 取回。
#[inline(always)]
fn sbi_call(eid: usize, fid: usize, arg0: usize, arg1: usize, arg2: usize) -> SbiRet {
    let error: usize;
    let value: usize;
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
    SbiRet { error, value }
}

// ---------- SBI 扩展号 ----------

/// Legacy 扩展：逐字符输出（EID = 0x01）
const SBI_CONSOLE_PUTCHAR: usize = 0x01;
/// 系统复位扩展（EID = 0x53525354，即 "SRST"）
const SBI_SHUTDOWN: usize = 0x53525354;

// ---------- 对外接口 ----------

/// 向串口输出一个字节（通过 SBI Legacy 接口）
pub fn console_putchar(c: usize) {
    sbi_call(SBI_CONSOLE_PUTCHAR, 0, c, 0, 0);
}

/// 请求 SBI 关机
pub fn shutdown() -> ! {
    sbi_call(SBI_SHUTDOWN, 0, 0, 0, 0);
    panic!("It should shutdown!");
}

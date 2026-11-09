//! Trap 处理。
//!
//! "Trap"是 RISC-V 对两类事件的统称：
//!
//! - **异常 (Exception)**：由当前这条指令同步触发。`ecall`（系统调用）、
//!   非法指令、访存越界都属于这一类。
//! - **中断 (Interrupt)**：由外部事件异步触发。时钟、外设。第 5 周才会用到。
//!
//! 两者的处理流程完全一样：CPU 跳到 `stvec` 指定的地址，
//! 我们在那里保存现场、分派处理、恢复现场、返回。

mod context;

use crate::batch::run_next_app;
use crate::csr::{decode_scause, scause, sepc, stval, stvec, Exception, Trap};
use crate::syscall::syscall;
use core::arch::global_asm;

pub use context::TrapContext;

global_asm!(include_str!("trap.S"));

extern "C" {
    fn __alltraps();
}

/// 初始化 trap 处理：告诉 CPU 出事时往哪跳。
pub fn init() {
    // SAFETY: __alltraps 是我们自己写的、满足 4 字节对齐的合法 trap 入口。
    //
    // stvec 的低 2 位是模式位，0 表示 Direct 模式 —— 所有 trap 都跳到
    // 同一个入口，由软件自己分派。另一种是 Vectored 模式，按中断号跳到
    // 不同偏移，对我们没必要。__alltraps 已经 .align 2，低 2 位天然是 0。
    let entry = __alltraps as *const () as usize;
    unsafe {
        stvec::write(entry);
    }
    info!("trap 入口已设置：stvec = {:#x}", entry);
}

/// Trap 的 Rust 侧处理入口，由 `__alltraps` 在保存完现场后调用。
///
/// 返回值仍是那个 `TrapContext` 指针，`__alltraps` 会直接落进 `__restore`
/// 把它恢复回去。
///
/// `#[no_mangle]` 是必须的 —— 汇编里写的是 `call trap_handler`，
/// 符号名不能被 Rust 改写。
#[no_mangle]
pub fn trap_handler(cx: &mut TrapContext) -> &mut TrapContext {
    let scause = scause::read();
    let stval = stval::read();

    match decode_scause(scause) {
        // --- 系统调用 ---
        Trap::Exception(Exception::UserEnvCall) => {
            // sepc 指向的是 `ecall` 这条指令本身。如果直接返回，
            // 会再执行一次 ecall，陷入死循环。所以要手动跳过它。
            // RISC-V 的 ecall 固定 4 字节。
            cx.sepc += 4;

            // 系统调用约定：a7 放调用号，a0~a2 放参数，返回值写回 a0
            let result = syscall(cx.x[17], [cx.x[10], cx.x[11], cx.x[12]]);
            cx.x[10] = result as usize;
        }

        // --- 访存错误：用户程序碰了不该碰的地址 ---
        Trap::Exception(
            e @ (Exception::StoreFault
            | Exception::StorePageFault
            | Exception::LoadFault
            | Exception::LoadPageFault
            | Exception::StoreMisaligned
            | Exception::LoadMisaligned),
        ) => {
            error!(
                "用户程序访存出错：{:?}，地址 {:#x}，指令位于 {:#x} —— 杀掉，换下一个",
                e,
                stval,
                sepc::read()
            );
            run_next_app();
        }

        // --- 非法指令：比如用户程序试图执行只有内核能用的指令 ---
        Trap::Exception(Exception::IllegalInstruction) => {
            error!(
                "用户程序执行了非法指令，指令编码 {:#x}，位于 {:#x} —— 杀掉，换下一个",
                stval,
                sepc::read()
            );
            run_next_app();
        }

        // --- 还没处理的情况 ---
        other => {
            panic!(
                "内核遇到未处理的 trap：{:?}，scause={:#x}，stval={:#x}，sepc={:#x}",
                other,
                scause,
                stval,
                sepc::read()
            );
        }
    }

    cx
}

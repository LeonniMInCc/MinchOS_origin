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

use crate::csr::{
    decode_scause, scause, sepc, sie, sie_bits, stval, stvec, Exception, Interrupt, Trap,
};
use crate::syscall::syscall;
use crate::task::{exit_current_and_run_next, suspend_current_and_run_next};
use crate::timer::set_next_trigger;
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

/// 打开时钟中断。
///
/// # 只开 `sie.STIE`，不碰 `sstatus.SIE`
///
/// 按 RISC-V 特权级规范：CPU 运行在**更低**特权级（U 态）时，
/// S 态中断是**永远开启**的，和 `sstatus.SIE` 无关。所以只要打开
/// `sie.STIE`，用户程序就能被时钟中断抢占。
///
/// 而 `sstatus.SIE` 管的是"CPU 正在 S 态执行时是否响应中断"。
/// 我们刻意保持它为 0，也就是说 —— **内核自身不可抢占**。
///
/// 这是一个明确的设计决定，不是偷懒：
///
/// - 内核执行期间到达的时钟中断会被挂起（pending），
///   等 `sret` 回到用户态的那一刻才被送达，一个都不会丢
/// - 内核里的临界区因此天然是原子的，
///   [`crate::sync::UPSafeCell`] 的"不被抢占"前提得以保住
///
/// 代价是内核里一段长耗时的操作会推迟所有任务的调度。
/// Linux 在 2.6 之前也是这么做的，后来才引入可抢占内核。
pub fn enable_timer_interrupt() {
    // SAFETY: 只置位 STIE，不改动 sie 的其他位。
    unsafe {
        sie::write(sie::read() | sie_bits::STIE);
    }
    info!(
        "时钟中断已开启：时间片 {} ms（内核自身不可抢占）",
        crate::timer::slice_ms()
    );
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

        // --- 时钟中断：时间片用完了，换人 ---
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            // 必须先把下一次闹钟设上。SBI 的定时器是一次性的，
            // 而 suspend_current_and_run_next 会把执行流切走 ——
            // 忘了重设，抢占就只会发生一次。
            set_next_trigger();

            // 注意这里**不动 sepc**。中断和 `ecall` 不同：被打断的那条
            // 指令还没执行完，任务恢复时要重新执行它。
            // 只有 `ecall` 这种"已经完成了语义"的异常才需要 +4。
            suspend_current_and_run_next();
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
            exit_current_and_run_next();
        }

        // --- 非法指令：比如用户程序试图执行只有内核能用的指令 ---
        Trap::Exception(Exception::IllegalInstruction) => {
            error!(
                "用户程序执行了非法指令，指令编码 {:#x}，位于 {:#x} —— 杀掉，换下一个",
                stval,
                sepc::read()
            );
            exit_current_and_run_next();
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

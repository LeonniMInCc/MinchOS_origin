//! Trap 处理。
//!
//! 第 8 周起，trap 处理多了一件事：**换地址空间**。整条路径是：
//!
//! ```text
//!   用户程序（用户地址空间）
//!     │ ecall / 异常 / 时钟中断
//!     ▼
//!   __alltraps  ← 跳板页，两个地址空间里同一个虚拟地址
//!     │ 保存现场到 TRAP_CONTEXT
//!     │ csrw satp ← 内核页表        ★ 换地址空间
//!     ▼
//!   trap_handler（内核地址空间，跑在本任务的内核栈上）
//!     │ 分派：系统调用 / 杀进程 / 重新调度
//!     ▼
//!   trap_return
//!     │ stvec ← TRAMPOLINE
//!     │ jr 跳板页里的 __restore
//!     ▼
//!   __restore   ← 还是那一页
//!     │ csrw satp ← 用户页表        ★ 换回去
//!     │ 恢复现场
//!     │ sret
//!     ▼
//!   用户程序
//! ```
//!
//! 两次 `csrw satp` 都发生在跳板页上，这是整章设计的支点。

mod context;

use crate::config::{TRAMPOLINE, TRAP_CONTEXT};
use crate::csr::{
    decode_scause, scause, sepc, sie, sie_bits, stval, stvec, Exception, Interrupt, Trap,
};
use crate::syscall::syscall;
use crate::task::{
    current_trap_cx, current_user_token, exit_current_and_run_next, suspend_current_and_run_next,
};
use crate::timer::set_next_trigger;
use core::arch::{asm, global_asm};

pub use context::TrapContext;

global_asm!(include_str!("trap.S"));

extern "C" {
    fn __alltraps();
    fn __restore();
    fn __kernel_trap();
}

/// 把 trap 入口指向内核态版本：内核自己出错时直接 panic 在第一现场。
fn set_kernel_trap_entry() {
    // SAFETY: __kernel_trap 是 .align 2 的合法入口。
    unsafe {
        stvec::write(__kernel_trap as *const () as usize);
    }
}

/// 把 trap 入口指向跳板页。
///
/// 注意写的是 [`TRAMPOLINE`] 这个**虚拟地址常量**，而不是 `__alltraps`
/// 的链接地址。因为用户地址空间里根本没有内核的 `.text` —— 唯一可执行的
/// 内核代码就是映射在 `TRAMPOLINE` 上的那一页。
fn set_user_trap_entry() {
    // SAFETY: 每个地址空间都在 TRAMPOLINE 处映射了 __alltraps 所在的页。
    unsafe {
        stvec::write(TRAMPOLINE);
    }
}

/// 初始化 trap 处理。
pub fn init() {
    // 先指向内核态入口。启动阶段还没有任何用户程序在跑，
    // 这时出的任何 trap 都是内核自己的问题，应该立刻在第一现场 panic。
    set_kernel_trap_entry();
    info!(
        "trap 入口已设置：内核态 {:#x}，用户态 {:#x}（跳板页）",
        __kernel_trap as *const () as usize,
        TRAMPOLINE
    );
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
/// 这正是 [`crate::sync::UPSafeCell`] 赖以成立的前提。
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

/// 打开外部中断（PLIC 转发来的设备中断）。
///
/// 第 14 周新增。有了它，串口输入才能从"轮询"变成"硬件通知"。
pub fn enable_external_interrupt() {
    // SAFETY: 只置位 SEIE，不改动 sie 的其他位。
    unsafe {
        sie::write(sie::read() | sie_bits::SEIE);
    }
}

/// 内核自己触发 trap 时的处理：没有恢复的可能，打印现场后停机。
///
/// 内核态的 trap 几乎总是编程错误（解引用了空指针、访问了没映射的地址、
/// 内核栈写穿了 guard page）。它和用户态的 trap 有本质区别：
/// 用户程序出错可以杀掉它继续跑，内核出错则说明整个系统的假设已经破了。
#[no_mangle]
pub fn kernel_trap_handler() -> ! {
    let scause = scause::read();
    panic!(
        "内核自身触发了 trap：{:?}\n         scause={:#x}  stval={:#x}  sepc={:#x}",
        decode_scause(scause),
        scause,
        stval::read(),
        sepc::read()
    );
}

/// Trap 的 Rust 侧处理入口。
///
/// 由跳板页上的 `__alltraps` 在换好页表之后 `jr` 跳进来 ——
/// 注意是 `jr` 而不是 `call`：我们不打算原路返回，
/// 回用户态走的是 [`trap_return`] 那条路。
///
/// 和第 3-7 周相比，它不再接收 `&mut TrapContext` 参数：那时 `sp` 正指着
/// 内核栈上的上下文，直接传地址即可；现在上下文在**用户地址空间**里，
/// 内核只能通过它的物理页号去访问（[`current_trap_cx`]）。
#[no_mangle]
pub fn trap_handler() -> ! {
    // 处理期间 stvec 指向内核态入口：万一内核自己在处理 trap 时又出错，
    // 会走清晰的 panic 路径，而不是被 __alltraps 当成一次用户态 trap
    // 再折腾一遍（那样只会得到一个假现场）。
    set_kernel_trap_entry();

    let scause = scause::read();
    let stval = stval::read();

    match decode_scause(scause) {
        // --- 系统调用 ---
        Trap::Exception(Exception::UserEnvCall) => {
            let cx = current_trap_cx();
            // sepc 指向的是 `ecall` 这条指令本身。如果直接返回，
            // 会再执行一次 ecall，陷入死循环。RISC-V 的 ecall 固定 4 字节。
            cx.sepc += 4;
            let (id, args) = (cx.x[17], [cx.x[10], cx.x[11], cx.x[12]]);

            let result = syscall(id, args);

            // 重新取一次。`syscall` 里可能发生过任务切换（sys_yield、
            // 或者被抢占），中间隔了很久 —— 虽然回来之后还是同一个任务、
            // 同一份上下文，但不重新取的话，上面那个引用就横跨了一次
            // 可能的地址空间切换，读起来很容易让人误判。
            current_trap_cx().x[10] = result as usize;
        }

        // --- 时钟中断：时间片用完了，换人 ---
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            // 必须先把下一次闹钟设上。SBI 的定时器是一次性的，
            // 而 suspend_current_and_run_next 会把执行流切走 ——
            // 忘了重设，抢占就只会发生一次。
            set_next_trigger();

            // 注意这里**不动 sepc**。中断和 `ecall` 不同：被打断的那条
            // 指令还没执行完，任务恢复时要重新执行它。
            suspend_current_and_run_next();
        }

        // --- 外部中断：某个设备有事了 ---
        Trap::Interrupt(Interrupt::SupervisorExternal) => {
            // 认领 → 处理 → 完成。少了最后一步，这个中断源就再也不会响了。
            let irq = crate::drivers::plic::claim();
            if irq == crate::drivers::plic::IRQ_UART0 {
                crate::drivers::uart::handle_interrupt();
            } else if irq != 0 {
                warn!("收到未处理的外部中断 {}", irq);
            }
            if irq != 0 {
                crate::drivers::plic::complete(irq);
            }
        }

        // --- 访存错误：用户程序碰了不该碰的地址 ---
        Trap::Exception(
            e @ (Exception::StoreFault
            | Exception::StorePageFault
            | Exception::LoadFault
            | Exception::LoadPageFault
            | Exception::InstructionFault
            | Exception::InstructionPageFault
            | Exception::StoreMisaligned
            | Exception::LoadMisaligned),
        ) => {
            error!(
                "用户程序访存出错：{:?}，地址 {:#x}，指令位于 {:#x} —— 杀掉，换下一个",
                e,
                stval,
                current_trap_cx().sepc
            );
            exit_current_and_run_next(-1);
        }

        // --- 非法指令：比如用户程序试图执行只有内核能用的指令 ---
        Trap::Exception(Exception::IllegalInstruction) => {
            error!(
                "用户程序执行了非法指令，指令编码 {:#x}，位于 {:#x} —— 杀掉，换下一个",
                stval,
                current_trap_cx().sepc
            );
            exit_current_and_run_next(-1);
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

    trap_return()
}

/// 返回用户态。永不返回。
///
/// 这里要做的三件事，每一件都有讲究：
///
/// 1. **把 `stvec` 切回跳板页** —— 下一次 trap 才能正确进入
/// 2. **算出 `__restore` 在跳板页里的虚拟地址** —— 不能直接 `call __restore`，
///    因为那会跳到它的**链接地址**（内核 `.text` 里），而马上就要换成
///    用户页表了，那个地址在用户空间里不存在
/// 3. **用 `jr` 跳过去，参数放 `a0`/`a1`** —— 这是和汇编约定的接口
#[no_mangle]
pub fn trap_return() -> ! {
    set_user_trap_entry();

    let trap_cx_va = TRAP_CONTEXT;
    let user_satp = current_user_token();

    // 跳板页是整段搬过去的，所以 __restore 相对 __alltraps 的偏移
    // 在两个地址空间里完全一样。
    let restore_va =
        __restore as *const () as usize - __alltraps as *const () as usize + TRAMPOLINE;

    // SAFETY: restore_va 落在跳板页内，而跳板页在当前（内核）地址空间
    // 和即将切入的用户地址空间里都有映射。a0/a1 是与 trap.S 约定的接口。
    unsafe {
        asm!(
            // 刚刚可能往内存里写过用户程序的代码（装载/exec），
            // 让取指单元丢弃缓存重新取
            "fence.i",
            "jr {restore_va}",
            restore_va = in(reg) restore_va,
            in("a0") trap_cx_va,
            in("a1") user_satp,
            options(noreturn)
        );
    }
}

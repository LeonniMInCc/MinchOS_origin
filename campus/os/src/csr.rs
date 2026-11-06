//! RISC-V 控制状态寄存器 (CSR) 访问。
//!
//! CSR 是 CPU 里一组特殊寄存器，控制着特权级、中断、异常、分页这些机制。
//! 它们不能用普通的 `ld`/`sd` 访问，只能用专门的 `csrr` / `csrw` 指令。
//!
//! 这里手写而不是引入 `riscv` crate：整个模块不到一百行，
//! 但能让人看清"设置一个 CSR"到底是怎么一回事。
//!
//! # 本章用到的 CSR
//!
//! | 寄存器     | 作用                                           |
//! |------------|------------------------------------------------|
//! | `stvec`    | Trap 入口地址 —— 出事时 CPU 往哪跳              |
//! | `sepc`     | 触发 trap 的那条指令的地址                      |
//! | `scause`   | trap 的原因（是中断还是异常，具体哪一种）        |
//! | `stval`    | 出错的附加信息（比如非法访问的地址）             |
//! | `sstatus`  | 状态标志，其中 `SPP` 位记录"从哪个特权级来的"    |
//! | `sscratch` | 一个可以自由使用的暂存寄存器，我们用它倒腾栈指针 |

/// 为一个 CSR 生成 `read` / `write` 函数。
macro_rules! define_csr {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        pub mod $name {
            /// 读取该 CSR 的当前值。
            #[inline(always)]
            #[allow(dead_code)] // 有些 CSR 本章只写不读，但保留接口
            pub fn read() -> usize {
                let value: usize;
                // SAFETY: csrr 只是读取，不改变任何状态。
                unsafe {
                    core::arch::asm!(
                        concat!("csrr {}, ", stringify!($name)),
                        out(reg) value,
                        options(nomem, nostack)
                    );
                }
                value
            }

            /// 写入该 CSR。
            ///
            /// # Safety
            ///
            /// 写 CSR 会直接改变 CPU 的行为（trap 往哪跳、返回到哪个特权级）。
            /// 调用方必须清楚自己在做什么。
            #[inline(always)]
            #[allow(dead_code)]
            pub unsafe fn write(value: usize) {
                core::arch::asm!(
                    concat!("csrw ", stringify!($name), ", {}"),
                    in(reg) value,
                    options(nomem, nostack)
                );
            }
        }
    };
}

define_csr!(
    /// Trap 向量基址：发生 trap 时 CPU 跳转到这个地址。
    stvec
);
define_csr!(
    /// Exception Program Counter：触发 trap 的那条指令的地址。
    /// `sret` 会返回到这里，所以处理 `ecall` 时要手动 +4 跳过它。
    sepc
);
define_csr!(
    /// Trap 原因。最高位是"是否为中断"，低位是具体编号。
    scause
);
define_csr!(
    /// Trap 附加信息。缺页/访存错误时存放出错的地址。
    stval
);
define_csr!(
    /// 状态寄存器。见 [`sstatus_bits`]。
    sstatus
);
define_csr!(
    /// 暂存寄存器。`__alltraps` 用它在用户栈和内核栈之间倒腾。
    sscratch
);
define_csr!(
    /// 中断使能位图。见 [`sie_bits`]。
    sie
);
define_csr!(
    /// Supervisor Address Translation and Protection —— 分页的总开关。
    ///
    /// ```text
    ///   63    60 59        44 43            0
    ///   ┌───────┬────────────┬───────────────┐
    ///   │ MODE  │    ASID    │   根页表 PPN   │
    ///   └───────┴────────────┴───────────────┘
    ///     0 = 关闭（物理地址直通）
    ///     8 = SV39
    /// ```
    ///
    /// **写这个寄存器是整个内核里最危险的一条指令**：它执行完之后，
    /// 取下一条指令就要走新页表了。新页表里要是没有当前这段代码，
    /// CPU 立刻取指失败，而且连报错的机会都没有。
    satp
);
define_csr!(
    /// 只读的计数器，每个时钟周期加一。QEMU virt 上频率是 10 MHz。
    ///
    /// 它是我们唯一的时间来源 —— 内核靠它算时间片，用户程序靠
    /// `sys_get_time` 间接读它。
    time
);

/// `sstatus` 里我们关心的几个位。
pub mod sstatus_bits {
    /// Supervisor Previous Privilege：记录进入 S 态之前处于哪个特权级。
    ///
    /// - `0` = 来自 U 态（用户程序）
    /// - `1` = 来自 S 态（内核自己）
    ///
    /// **`sret` 会根据这一位决定返回到哪个特权级**。所以要让一段代码
    /// 跑在 U 态，办法就是把 SPP 清零，然后 `sret`。
    pub const SPP: usize = 1 << 8;

    /// Supervisor Previous Interrupt Enable：`sret` 之后 `SIE` 恢复成什么。
    pub const SPIE: usize = 1 << 5;

    /// permit Supervisor User Memory access：S 态能否访问带 `U` 位的页。
    ///
    /// **默认是 0，也就是不能。** 这是一条很容易忽略但很重要的保护：
    /// 内核不小心解引用了一个用户指针时，硬件会立刻拦下来，
    /// 而不是让它悄悄读写用户内存。
    ///
    /// 第 7 周的设计（内核和用户共用一个地址空间）绕不过它 ——
    /// 内核要往带 `U` 的页里装载用户程序、要读用户传来的缓冲区，
    /// 所以必须打开。第 8 周给每个进程建独立地址空间之后，
    /// 内核改用手动查页表的方式访问用户内存，这一位就能关掉了。
    /// 第 8 周起 CCNUOS **不再打开它** —— 保留定义是为了让这段文档
    /// 和第 7 章的对照留在代码里。
    #[allow(dead_code)]
    pub const SUM: usize = 1 << 18;

    /// Supervisor Interrupt Enable：**S 态自身**是否响应中断。
    ///
    /// 注意这一位管的只是"CPU 正在 S 态执行时"。按 RISC-V 特权级规范，
    /// 当 CPU 运行在**更低**的特权级（U 态）时，S 态中断是**永远开启**的，
    /// 与这一位无关。
    ///
    /// 所以第 5 周要实现抢占用户程序，只需要打开 [`sie_bits::STIE`]，
    /// **不需要**也**不应该**设置这一位 —— 保持它为 0，内核自身就是
    /// 不可抢占的，这正是 [`crate::sync::UPSafeCell`] 赖以成立的前提。
    #[allow(dead_code)]
    pub const SIE: usize = 1 << 1;
}

/// `sie`（中断使能）里我们关心的几个位。
pub mod sie_bits {
    /// Supervisor Software Interrupt Enable
    #[allow(dead_code)]
    pub const SSIE: usize = 1 << 1;
    /// Supervisor **Timer** Interrupt Enable —— 第 5 周的主角
    pub const STIE: usize = 1 << 5;
    /// Supervisor External Interrupt Enable（外设中断，本项目用不到）
    #[allow(dead_code)]
    pub const SEIE: usize = 1 << 9;
}

// ---------------------------------------------------------------------------
// scause 解码
// ---------------------------------------------------------------------------

/// `scause` 最高位：1 表示中断，0 表示异常。
const INTERRUPT_FLAG: usize = 1 << (usize::BITS - 1);

/// 一次 trap 的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trap {
    /// 异常：由当前指令同步触发（ecall、非法指令、访存错误……）
    Exception(Exception),
    /// 中断：由外部事件异步触发（时钟、外设……）
    Interrupt(Interrupt),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exception {
    InstructionMisaligned,
    InstructionFault,
    IllegalInstruction,
    Breakpoint,
    LoadMisaligned,
    LoadFault,
    StoreMisaligned,
    StoreFault,
    /// 来自 U 态的 `ecall` —— 也就是系统调用
    UserEnvCall,
    InstructionPageFault,
    LoadPageFault,
    StorePageFault,
    Unknown(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupt {
    SupervisorSoft,
    SupervisorTimer,
    SupervisorExternal,
    Unknown(usize),
}

/// 把 `scause` 的原始值解码成 [`Trap`]。
pub fn decode_scause(scause: usize) -> Trap {
    let code = scause & !INTERRUPT_FLAG;
    if scause & INTERRUPT_FLAG != 0 {
        Trap::Interrupt(match code {
            1 => Interrupt::SupervisorSoft,
            5 => Interrupt::SupervisorTimer,
            9 => Interrupt::SupervisorExternal,
            other => Interrupt::Unknown(other),
        })
    } else {
        Trap::Exception(match code {
            0 => Exception::InstructionMisaligned,
            1 => Exception::InstructionFault,
            2 => Exception::IllegalInstruction,
            3 => Exception::Breakpoint,
            4 => Exception::LoadMisaligned,
            5 => Exception::LoadFault,
            6 => Exception::StoreMisaligned,
            7 => Exception::StoreFault,
            8 => Exception::UserEnvCall,
            12 => Exception::InstructionPageFault,
            13 => Exception::LoadPageFault,
            15 => Exception::StorePageFault,
            other => Exception::Unknown(other),
        })
    }
}

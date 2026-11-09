//! Trap 上下文。

use crate::csr::{sstatus, sstatus_bits};

/// 一次 trap 发生时，需要保存下来的全部 CPU 状态。
///
/// 用户程序执行到一半被打断（主动 `ecall` 或被动出错），内核要跑起来
/// 处理这件事。但内核一运行就会覆盖掉寄存器里的内容 —— 所以必须先把
/// 用户程序的"现场"完整拍下来存好，处理完再一模一样地放回去，
/// 用户程序才会觉得什么都没发生过。
///
/// # 为什么是 `#[repr(C)]`
///
/// 这个结构体由**汇编**（`trap.S`）填写，由 **Rust**（`trap_handler`）读取。
/// 汇编里写的是 `sd x1, 1*8(sp)` 这种硬编码偏移，所以字段的内存布局
/// 必须是确定的、和声明顺序一致的。Rust 默认的 `repr(Rust)` 允许编译器
/// 随意重排字段 —— 那样汇编就会读到错误的位置，而且**不会有任何报错**，
/// 只会得到一堆诡异的行为。
///
/// # 内存布局
///
/// ```text
///   偏移      内容
///   0         x0  (恒为 0，存了也没用，但保留位置让偏移对得上)
///   8         x1  (ra,  返回地址)
///   16        x2  (sp,  栈指针 —— 存的是用户栈)
///   24        x3  (gp)
///   32        x4  (tp)
///   40..      x5..x31
///   256       sstatus
///   264       sepc
///   ─────────────────  共 34 * 8 = 272 字节
/// ```
#[repr(C)]
#[derive(Debug)]
pub struct TrapContext {
    /// 32 个通用寄存器 `x0`~`x31`。
    ///
    /// `x0` 在 RISC-V 里硬连线为 0，保存它毫无意义，但保留这个位置
    /// 能让"寄存器编号 × 8 = 偏移"这条规则成立，汇编写起来简单得多。
    pub x: [usize; 32],

    /// 进入 trap 前的 `sstatus`。关键是其中的 `SPP` 位 ——
    /// `sret` 靠它决定返回到 U 态还是 S 态。
    pub sstatus: usize,

    /// 触发 trap 的那条指令的地址。`sret` 会返回到这里。
    pub sepc: usize,
}

impl TrapContext {
    /// 设置栈指针（`x2` 就是 `sp`）。
    pub fn set_sp(&mut self, sp: usize) {
        self.x[2] = sp;
    }

    /// 构造一个"即将开始执行的用户程序"的初始上下文。
    ///
    /// 这是本章最妙的一处设计：**启动一个用户程序，和从一次 trap 返回，
    /// 是同一件事。**
    ///
    /// 我们不需要另写一套"进入用户态"的代码，只要伪造一个 trap 上下文 ——
    /// 假装用户程序刚刚在 `entry` 处陷入了内核 —— 然后走正常的 `__restore`
    /// 返回路径，CPU 就会以 U 态从 `entry` 开始执行。
    pub fn app_init_context(entry: usize, sp: usize) -> Self {
        let mut sstatus = sstatus::read();
        // 把 SPP 清零，这样 sret 之后 CPU 会落到 U 态。
        // 这一位就是"用户程序凭什么跑在用户态"的全部答案。
        sstatus &= !sstatus_bits::SPP;
        // 第 3 周还没开中断，用户态也先不开。第 5 周会把 SPIE 置 1。
        sstatus &= !sstatus_bits::SPIE;

        let mut cx = Self {
            x: [0; 32],
            sstatus,
            sepc: entry, // sret 会跳到这里 —— 用户程序的入口
        };
        cx.set_sp(sp); // 让用户程序用它自己的栈
        cx
    }
}

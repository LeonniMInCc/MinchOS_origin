//! Trap 上下文。

use crate::csr::{sstatus, sstatus_bits};

/// 一次 trap 发生时，需要保存下来的全部 CPU 状态。
///
/// 用户程序执行到一半被打断（主动 `ecall` 或被动出错），内核要跑起来
/// 处理这件事。但内核一运行就会覆盖掉寄存器里的内容 —— 所以必须先把
/// 用户程序的"现场"完整拍下来存好，处理完再一模一样地放回去。
///
/// # `#[repr(C)]` 是正确性的前提
///
/// 这个结构体由**汇编**（`trap.S`）填写，由 **Rust** 读取。
/// 汇编里写的是 `sd x1, 1*8(sp)` 这种硬编码偏移，而 Rust 默认的
/// `repr(Rust)` 允许编译器任意重排字段 —— 一旦重排，汇编就会读写错误的
/// 位置，而且**不会有任何编译错误或运行时报错**。
///
/// # 第 8 周新增的三个字段
///
/// 前七章 TrapContext 只有寄存器和两个 CSR。现在多了三样：
/// `kernel_satp`、`kernel_sp`、`trap_handler`。
///
/// 为什么？因为 `__alltraps` 现在要**换地址空间**，它需要知道内核的
/// 页表在哪、内核栈在哪、处理函数在哪。而这三样信息：
///
/// - 不能放在寄存器里 —— 寄存器正要被保存，动不得
/// - 不能从内核的数据结构读 —— 还没切到内核页表，读不到
///
/// 唯一的出路就是**预先写进 TrapContext**：`trap_return` 在返回用户态
/// 之前填好，`__alltraps` 陷入时直接读出来。
///
/// 这是一个很典型的内核设计手法：**当某个时刻拿不到需要的信息时，
/// 就在更早的、还拿得到的时刻把它存下来。**
///
/// # 内存布局
///
/// ```text
///   偏移      内容
///   0         x0  (恒为 0，存了也没用，但保留位置让偏移对得上)
///   8         x1  (ra)
///   16        x2  (sp —— 存的是用户栈)
///   24..      x3..x31
///   256       sstatus
///   264       sepc
///   272       kernel_satp    ← 第 8 周新增
///   280       kernel_sp      ← 第 8 周新增
///   288       trap_handler   ← 第 8 周新增
///   ─────────────────  共 37 * 8 = 296 字节
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

    /// 内核地址空间的 `satp`。`__alltraps` 用它换页表。
    pub kernel_satp: usize,

    /// 本任务在内核地址空间里的内核栈顶。
    pub kernel_sp: usize,

    /// `trap_handler` 的地址。
    pub trap_handler: usize,
}

impl TrapContext {
    /// 设置栈指针（`x2` 就是 `sp`）。
    pub fn set_sp(&mut self, sp: usize) {
        self.x[2] = sp;
    }

    /// 构造一个"即将开始执行的用户程序"的初始上下文。
    ///
    /// **启动一个用户程序，和从一次 trap 返回，是同一件事。**
    /// 我们不需要另写一套"进入用户态"的代码，只要伪造一个 trap 上下文 ——
    /// 假装用户程序刚刚在 `entry` 处陷入了内核 —— 然后走正常的返回路径。
    pub fn app_init_context(
        entry: usize,
        user_sp: usize,
        kernel_satp: usize,
        kernel_sp: usize,
        trap_handler: usize,
    ) -> Self {
        let mut sstatus = sstatus::read();
        // 把 SPP 清零，这样 sret 之后 CPU 会落到 U 态。
        // 这一位就是"用户程序凭什么跑在用户态"的全部答案。
        sstatus &= !sstatus_bits::SPP;
        // 用户态不需要 SPIE：CPU 在 U 态执行时，S 态中断本来就是开着的
        // （见第 5 章）。这里清掉只是让状态更干净。
        sstatus &= !sstatus_bits::SPIE;

        let mut cx = Self {
            x: [0; 32],
            sstatus,
            sepc: entry, // sret 会跳到这里 —— 用户程序的入口
            kernel_satp,
            kernel_sp,
            trap_handler,
        };
        cx.set_sp(user_sp); // 让用户程序用它自己的栈
        cx
    }
}

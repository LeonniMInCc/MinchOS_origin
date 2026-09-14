//! 任务控制块。

use super::context::TaskContext;
use crate::config::TRAP_CONTEXT;
use crate::mm::{alloc_kernel_stack, kernel_token, MemorySet, PhysPageNum, VirtAddr};
use crate::trap::TrapContext;

/// 任务的生命周期状态。
///
/// 第 9 周引入真正的进程后还会加上 `Zombie`
/// （已退出但父进程还没 wait 走它的退出码）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskStatus {
    /// 就绪，等待被调度
    Ready,
    /// 正在 CPU 上运行
    Running,
    /// 已退出
    Exited,
}

/// 任务控制块 (TCB)：内核眼里的"一个任务"。
///
/// 这个结构体的膨胀史就是操作系统抽象能力的成长史：
///
/// - 第 4 周：只有状态和任务上下文
/// - **第 8 周（现在）**：加上独立的地址空间
/// - 第 9 周：加 PID 和父子关系
/// - 第 10 周：加文件描述符表
///
/// 注意它从第 8 周起**不再是 `Copy`** —— `MemorySet` 拥有一堆物理页帧，
/// 复制它没有意义。这个变化连带把任务数组从定长数组变成了 `Vec`。
pub struct TaskControlBlock {
    pub task_status: TaskStatus,
    pub task_cx: TaskContext,

    /// 这个任务的地址空间。它被丢弃时，任务占用的全部物理页帧
    /// （代码、数据、栈、页表本身）会沿着 `FrameTracker` 一路自动归还。
    pub memory_set: MemorySet,

    /// 这个任务的 [`TrapContext`] 所在的**物理**页号。
    ///
    /// 为什么要单独存？因为 `TrapContext` 位于任务自己的地址空间里
    /// （虚拟地址 [`TRAP_CONTEXT`]），而内核跑在自己的地址空间里 ——
    /// 内核想读写它，只能通过物理页号 + 恒等映射绕过去。
    pub trap_cx_ppn: PhysPageNum,

    /// 用户栈顶。第 9 周做 `fork` 时要用。
    pub base_size: usize,
}

impl TaskControlBlock {
    /// 从一个 ELF 文件创建任务。
    pub fn new(elf_data: &[u8], app_id: usize) -> Self {
        // 1. 建立地址空间：ELF 各段 + 用户栈 + TrapContext 页 + 跳板页
        let (memory_set, user_sp, entry) = MemorySet::from_elf(elf_data);

        // 2. 记下 TrapContext 页的物理页号
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT).floor())
            .expect("from_elf 应该已经映射了 TrapContext 页")
            .ppn();

        // 3. 在**内核地址空间**里给它分配内核栈（自带 guard page）
        let kernel_sp = alloc_kernel_stack(app_id);

        let tcb = Self {
            task_status: TaskStatus::Ready,
            // 第一次被调度时直接进 trap_return，走"从一次不存在的 trap 返回"
            task_cx: TaskContext::goto_trap_return(kernel_sp),
            memory_set,
            trap_cx_ppn,
            base_size: user_sp,
        };

        // 4. 填好初始 TrapContext。
        //
        // 后三个参数是第 8 周新增的：__alltraps 换页表时需要它们，
        // 而那一刻它既读不到内核的数据结构，也没有空闲寄存器 ——
        // 只能事先存在这里。
        extern "C" {
            fn trap_handler();
        }
        *tcb.trap_cx() = TrapContext::app_init_context(
            entry,
            user_sp,
            kernel_token(),
            kernel_sp,
            trap_handler as *const () as usize,
        );

        debug!(
            "  任务 {} 地址空间就绪：入口 {:#x}，用户栈顶 {:#x}，内核栈顶 {:#x}",
            app_id, entry, tcb.base_size, kernel_sp
        );
        tcb
    }

    /// 通过物理页号访问这个任务的 [`TrapContext`]。
    ///
    /// 这是内核跨地址空间读写用户数据的最简形式：目标页的物理页号是已知的，
    /// 而内核恒等映射了整个物理内存，所以直接当地址用就行。
    pub fn trap_cx(&self) -> &'static mut TrapContext {
        self.trap_cx_ppn.get_mut()
    }

    /// 这个任务地址空间的 `satp`。
    pub fn user_token(&self) -> usize {
        self.memory_set.token()
    }
}

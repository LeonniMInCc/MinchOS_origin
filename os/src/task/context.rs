//! 任务上下文。

/// 一次**任务切换**需要保存的状态。
///
/// # 为什么只有 14 个寄存器，而 [`TrapContext`] 要存 34 个？
///
/// [`TrapContext`]: crate::trap::TrapContext
///
/// 这是本章最值得想明白的一件事。两者保存的东西差了一倍多，
/// 因为它们发生的**时机**根本不同：
///
/// | | `TrapContext` | `TaskContext` |
/// |---|---|---|
/// | 触发方式 | 硬件异常，**随时可能发生** | `__switch` 是个普通函数调用 |
/// | 被打断的代码 | 用户程序，毫不知情 | 内核自己，主动调用 |
/// | 要保存什么 | 全部 32 个通用寄存器 | 只有 callee-saved |
///
/// `__alltraps` 面对的是"用户程序执行到任意一条指令时被硬件打断"。
/// 那一刻每个寄存器里都可能存着有用的值，少存一个就毁掉了现场。
///
/// 而 `__switch` 是**内核代码主动调用的一个函数**。按照 RISC-V 调用约定，
/// 调用方在调用任何函数前，就已经把自己还需要的 caller-saved 寄存器
/// （`t0`~`t6`、`a0`~`a7`）妥善安置了 —— 这是编译器保证的。
/// `__switch` 作为被调用者，只需要遵守约定：保存好 callee-saved 寄存器
/// （`s0`~`s11`）、返回地址 `ra` 和栈指针 `sp`。
///
/// **换句话说：编译器已经替我们干了一半的活。**
///
/// # `ra` 为什么是这里最关键的字段
///
/// `ra` 决定了"这个任务下次被调度时从哪里继续"。
///
/// - 任务**第一次**被调度：`ra` 指向 `__restore`，于是它会从内核态返回用户态，
///   开始执行程序的第一条指令
/// - 任务**之后**被调度：`ra` 指向它上次调用 `__switch` 的下一条指令，
///   于是它从"让出 CPU 的那一刻"继续往下跑
///
/// 同一个字段，两种语义，一套代码 —— 和第 3 章"启动程序 = 从 trap 返回"
/// 是同一种思路。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TaskContext {
    /// 返回地址：下次被调度时从这里继续执行
    ra: usize,
    /// 该任务的内核栈指针
    sp: usize,
    /// `s0` ~ `s11`，RISC-V 调用约定里的 callee-saved 寄存器
    s: [usize; 12],
}

impl TaskContext {
    pub const fn zero_init() -> Self {
        Self {
            ra: 0,
            sp: 0,
            s: [0; 12],
        }
    }

    /// 构造一个"还没跑过"的任务上下文。
    ///
    /// `kstack_ptr` 是该任务内核栈上那份 [`TrapContext`] 的地址。
    /// 当 `__switch` 把它装进 `sp`、把 `ra` 装成 `__restore` 然后 `ret` 时，
    /// `__restore` 恰好看到 `sp` 指向一份完整的 trap 现场，
    /// 于是把它恢复出去 —— 用户程序就跑起来了。
    pub fn goto_restore(kstack_ptr: usize) -> Self {
        extern "C" {
            fn __restore();
        }
        Self {
            ra: __restore as *const () as usize,
            sp: kstack_ptr,
            s: [0; 12],
        }
    }
}

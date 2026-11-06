//! 进程控制块。

use super::context::TaskContext;
use super::pid::{pid_alloc, KernelStack, PidHandle};
use crate::config::TRAP_CONTEXT;
use crate::fs::{File, Stdin, Stdout};
use crate::mm::{kernel_token, MemorySet, PhysPageNum, VirtAddr};
use crate::sync::{UPSafeCell, WaitQueue};
use crate::trap::TrapContext;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::cell::RefMut;

/// 进程的生命周期状态。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskStatus {
    /// 就绪，在调度队列里等着
    Ready,
    /// 正在 CPU 上运行
    Running,
    /// **阻塞**：在等某件事发生，不在就绪队列里。
    ///
    /// 和 `Ready` 的区别是关键的：`Ready` 的任务调度器会轮到它，
    /// `Blocked` 的任务**调度器根本看不见** —— 只有别人显式唤醒它，
    /// 它才会重新回到就绪队列。
    ///
    /// 第 14 周之前没有这个状态，于是"等待"只能靠不停 yield 轮询
    /// （见 `Stdin::read` 和 `waitpid`），白白烧掉本该属于自己的时间片。
    Blocked,
    /// **僵尸**：已经执行完毕，但父进程还没把它的退出码取走。
    ///
    /// 为什么要有这个状态？因为退出码是给父进程看的。如果进程一 `exit`
    /// 就彻底消失，父进程的 `wait` 就永远拿不到结果了。
    /// 所以进程退出时先变成僵尸，把用户内存还掉（那部分没人再需要），
    /// 只留下一个空壳和退出码，等父进程 `wait` 走它。
    Zombie,
}

/// 进程控制块 (PCB)。
///
/// # 为什么拆成"不可变部分 + `UPSafeCell` 包着的可变部分"
///
/// 进程之间要互相引用：父进程持有子进程、子进程指回父进程。这必然要用
/// `Arc`，而 `Arc` 里的东西是不可变的。所以把需要改的字段收进一个
/// [`UPSafeCell`]。
///
/// `pid` 和 `kernel_stack` 留在外面，因为它们一旦确定就不再变 ——
/// 放进去只会让每次读取都要借一次。
///
/// # 结构体的膨胀史
///
/// - 第 4 周：状态 + 任务上下文
/// - 第 8 周：加独立地址空间
/// - **第 9 周（现在）**：加 PID、父子关系、退出码
/// - 第 10 周：加文件描述符表
pub struct TaskControlBlock {
    // ---- 创建后不再改变 ----
    pub pid: PidHandle,
    pub kernel_stack: KernelStack,

    /// 等着**这个进程的子进程退出**的那些进程（实际上只可能是它自己）。
    ///
    /// `waitpid` 发现子进程还活着时，就把自己挂进来然后阻塞；
    /// 子进程退出时唤醒。第 9 周那个"返回 -2 让用户库循环重试"的
    /// 权宜之计到这里退休了。
    ///
    /// # 为什么它在"不可变部分"而不是 `inner` 里
    ///
    /// [`WaitQueue`] 自己就带内部可变性，不需要外层那道 `RefCell`。
    /// 更重要的是：唤醒一个等待者要访问**它的** `inner`，
    /// 而等待者很可能就是这个进程自己。如果队列藏在 `inner` 里，
    /// "取出队列"和"唤醒等待者"就会同时借用同一个 `RefCell` —— 当场 panic。
    ///
    /// 这个坑真实踩过一次。教训是：**能不放进锁里的东西就别放进去**，
    /// 尤其是那些本身就是同步设施的东西。
    pub child_waiters: WaitQueue,

    // ---- 会变的部分 ----
    inner: UPSafeCell<TaskControlBlockInner>,
}

pub struct TaskControlBlockInner {
    pub task_status: TaskStatus,
    pub task_cx: TaskContext,
    pub memory_set: MemorySet,
    /// 这个进程的 [`TrapContext`] 所在的**物理**页号。
    pub trap_cx_ppn: PhysPageNum,
    /// 用户栈顶
    pub base_size: usize,

    /// 父进程。用 [`Weak`] 而不是 [`Arc`] —— 否则父子互相强引用，
    /// 引用计数永远降不到 0，两个进程的内存谁都释放不掉。
    ///
    /// **父子关系里，"拥有"的方向是单向的：父进程拥有子进程。**
    pub parent: Option<Weak<TaskControlBlock>>,

    /// 子进程。父进程持有强引用，所以子进程即使退出成了僵尸，
    /// 它的 PCB 也会一直活到被 `wait` 走。
    pub children: Vec<Arc<TaskControlBlock>>,

    /// 退出码。只在 [`TaskStatus::Zombie`] 状态下有意义。
    pub exit_code: i32,

    /// 文件描述符表。下标就是 fd，`None` 表示这个槽位空着。
    ///
    /// 约定俗成的前三个：0 = 标准输入，1 = 标准输出，2 = 标准错误。
    ///
    /// 存 `Arc` 而不是值：`fork` 时父子**共享同一个文件对象**（只复制
    /// 这张表），这正是 Unix 的语义 —— 父子往同一个 stdout 里写，
    /// 内容会交错而不是各写各的。第 11 周的管道靠的就是这个。
    pub fd_table: Vec<Option<Arc<dyn File>>>,
}

impl TaskControlBlockInner {
    pub fn trap_cx(&self) -> &'static mut TrapContext {
        self.trap_cx_ppn.get_mut()
    }

    pub fn user_token(&self) -> usize {
        self.memory_set.token()
    }

    pub fn is_zombie(&self) -> bool {
        self.task_status == TaskStatus::Zombie
    }

    /// 找一个空闲的文件描述符槽位，没有就在末尾加一个。
    ///
    /// **总是返回最小的可用 fd。** 这不是实现细节，是 POSIX 的硬性要求 ——
    /// shell 的重定向完全建立在它之上：
    ///
    /// ```text
    ///   close(1);        // 释放 stdout，1 成为最小的空闲号
    ///   dup(pipe_write); // 于是新的副本必然落在 1 上
    /// ```
    ///
    /// 换成"随便找一个空位"的话，这套几十年来的惯用法就全垮了。
    pub fn alloc_fd(&mut self) -> usize {
        match self.fd_table.iter().position(|slot| slot.is_none()) {
            Some(fd) => fd,
            None => {
                self.fd_table.push(None);
                self.fd_table.len() - 1
            }
        }
    }
}

impl TaskControlBlock {
    pub fn inner_exclusive_access(&self) -> RefMut<'_, TaskControlBlockInner> {
        self.inner.exclusive_access()
    }

    pub fn pid(&self) -> usize {
        self.pid.0
    }

    /// 从一个 ELF 文件创建进程。只有 `initproc` 走这条路，
    /// 其余进程都是 `fork` 出来的。
    pub fn new(elf_data: &[u8]) -> Self {
        let (memory_set, user_sp, entry) = MemorySet::from_elf(elf_data);
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT).floor())
            .expect("from_elf 应该已经映射了 TrapContext 页")
            .ppn();

        let pid = pid_alloc();
        let kernel_stack = KernelStack::new(&pid);
        let kernel_sp = kernel_stack.top();

        let tcb = Self {
            pid,
            kernel_stack,
            child_waiters: WaitQueue::new(),
            // SAFETY: 单核，且所有访问都限制在 inner_exclusive_access 的短作用域内。
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    task_status: TaskStatus::Ready,
                    task_cx: TaskContext::goto_trap_return(kernel_sp),
                    memory_set,
                    trap_cx_ppn,
                    base_size: user_sp,
                    parent: None,
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: default_fd_table(),
                })
            },
        };

        *tcb.inner_exclusive_access().trap_cx() =
            TrapContext::app_init_context(entry, user_sp, kernel_token(), kernel_sp, trap_entry());

        tcb.inner_exclusive_access()
            .memory_set
            .trace_layout(tcb.pid());

        tcb
    }

    /// `fork`：复制出一个几乎一模一样的子进程。
    ///
    /// "几乎"的差别只有三处：
    ///
    /// 1. 新的 PID 和新的内核栈
    /// 2. `TrapContext.kernel_sp` 要指向**子进程自己的**内核栈
    /// 3. 系统调用返回值不同（由 `sys_fork` 设成 0）
    ///
    /// 其余全部照搬 —— 包括 `sepc`，所以父子从**同一条指令**继续执行。
    /// 这正是 `fork` 那个著名的"一次调用、两次返回"的由来。
    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        let mut parent_inner = self.inner_exclusive_access();

        let memory_set = MemorySet::from_existed_user(&parent_inner.memory_set);
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT).floor())
            .expect("复制出的地址空间缺少 TrapContext 页")
            .ppn();

        let pid = pid_alloc();
        let kernel_stack = KernelStack::new(&pid);
        let kernel_sp = kernel_stack.top();

        let child = Arc::new(Self {
            pid,
            kernel_stack,
            child_waiters: WaitQueue::new(),
            // SAFETY: 同 new()
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    task_status: TaskStatus::Ready,
                    task_cx: TaskContext::goto_trap_return(kernel_sp),
                    memory_set,
                    trap_cx_ppn,
                    base_size: parent_inner.base_size,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    exit_code: 0,
                    // 复制这张表，但表里的 Arc 指向同一批文件对象 ——
                    // 父子共享打开的文件，这是 Unix 的语义
                    fd_table: parent_inner.fd_table.clone(),
                })
            },
        });

        parent_inner.children.push(Arc::clone(&child));
        drop(parent_inner);

        // TrapContext 是连同地址空间一起拷过来的，里面的 kernel_sp 还是
        // 父进程的内核栈。不改的话，子进程第一次陷入内核就会跑到父进程的
        // 栈上去 —— 两个进程同时用一个内核栈，后果不堪设想。
        child.inner_exclusive_access().trap_cx().kernel_sp = kernel_sp;

        // 把子进程的地址空间布局报出去。和父进程对比着看，能直接看到
        // **相同的虚拟页号落在了不同的物理页上** —— fork 到底复制了什么，
        // 这一眼就清楚了。
        child
            .inner_exclusive_access()
            .memory_set
            .trace_layout(child.pid());

        child
    }

    /// `exec`：用一个新程序**替换**当前进程的地址空间。
    ///
    /// 注意 PID、父子关系、内核栈都不变 —— 换的只是"这个进程在执行什么"。
    /// 旧的 `MemorySet` 在赋值时被丢弃，它占的物理页帧全部自动归还。
    pub fn exec(&self, elf_data: &[u8]) {
        let (memory_set, user_sp, entry) = MemorySet::from_elf(elf_data);
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT).floor())
            .expect("from_elf 应该已经映射了 TrapContext 页")
            .ppn();

        let mut inner = self.inner_exclusive_access();
        inner.memory_set = memory_set; // 旧地址空间在这一行被 Drop
        inner.trap_cx_ppn = trap_cx_ppn;
        inner.base_size = user_sp;

        // 内核栈不变，所以 kernel_sp 用的还是原来那个
        *inner.trap_cx() = TrapContext::app_init_context(
            entry,
            user_sp,
            kernel_token(),
            self.kernel_stack.top(),
            trap_entry(),
        );

        // 先放掉借用再输出。原则见第 14 章那个坑：**能在锁外面做的事，
        // 就别在锁里面做。**
        drop(inner);
        self.inner_exclusive_access()
            .memory_set
            .trace_layout(self.pid());
    }
}

/// 新进程的初始文件描述符表：stdin / stdout / stderr。
fn default_fd_table() -> Vec<Option<Arc<dyn File>>> {
    alloc::vec![
        Some(Arc::new(Stdin) as Arc<dyn File>),
        Some(Arc::new(Stdout) as Arc<dyn File>),
        Some(Arc::new(Stdout) as Arc<dyn File>),
    ]
}

fn trap_entry() -> usize {
    extern "C" {
        fn trap_handler();
    }
    trap_handler as *const () as usize
}

//! 进程标识符与内核栈。
//!
//! 第 8 周之前，"任务"是编译期就定好的一张表，编号就是它在表里的下标，
//! 内核栈也按这个下标静态排布。
//!
//! 第 9 周有了 `fork` 之后这一套就不成立了：进程**运行时**才被创建，
//! 也会退出并让出编号。所以需要一个真正的分配器。

use crate::config::{kernel_stack_position, MAX_PROCESS_NUM};
use crate::mm::{map_kernel_stack, unmap_kernel_stack};
use crate::sync::UPSafeCell;
use alloc::vec::Vec;

/// 一个进程号的所有权凭证。
///
/// 和 [`crate::mm::FrameTracker`] 是同一个套路：持有它就等于占着这个号，
/// 它被丢弃时号自动归还。**"忘记释放 PID"在语法上就不可能发生。**
///
/// 这一点在进程模型里比在页帧里更重要：进程退出的路径有好几条
/// （正常 exit、被信号杀、父进程 wait 走），每条都记得释放 PID 是不现实的。
pub struct PidHandle(pub usize);

impl Drop for PidHandle {
    fn drop(&mut self) {
        PID_ALLOCATOR.exclusive_access().dealloc(self.0);
    }
}

/// 栈式 PID 分配器。和页帧分配器结构一样：
/// 优先复用回收的号，用完了才往后取新的。
struct PidAllocator {
    current: usize,
    recycled: Vec<usize>,
}

impl PidAllocator {
    const fn new() -> Self {
        Self {
            current: 0,
            recycled: Vec::new(),
        }
    }

    fn alloc(&mut self) -> PidHandle {
        if let Some(pid) = self.recycled.pop() {
            return PidHandle(pid);
        }
        assert!(
            self.current < MAX_PROCESS_NUM,
            "进程数超过上限 {MAX_PROCESS_NUM} —— 内核栈会一路排进用户地址空间"
        );
        let pid = self.current;
        self.current += 1;
        PidHandle(pid)
    }

    fn dealloc(&mut self, pid: usize) {
        assert!(pid < self.current, "回收了一个从未分配过的 PID {pid}");
        assert!(
            !self.recycled.contains(&pid),
            "重复回收 PID {pid}（double free）"
        );
        self.recycled.push(pid);
    }
}

static PID_ALLOCATOR: UPSafeCell<PidAllocator> =
    // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
    unsafe { UPSafeCell::new(PidAllocator::new()) };

pub fn pid_alloc() -> PidHandle {
    PID_ALLOCATOR.exclusive_access().alloc()
}

/// 一个进程在**内核地址空间**里的内核栈。
///
/// 它的生命周期和进程绑定：创建时在内核地址空间里映射出来，
/// 进程被彻底回收时自动解除映射、归还物理页帧。
///
/// 位置由 PID 决定（见 [`kernel_stack_position`]），相邻两个栈之间
/// 隔着一页不映射的 guard page。
pub struct KernelStack {
    pid: usize,
}

impl KernelStack {
    pub fn new(pid: &PidHandle) -> Self {
        map_kernel_stack(pid.0);
        Self { pid: pid.0 }
    }

    /// 栈顶（虚拟地址）。进程的 `TrapContext.kernel_sp` 就是它。
    pub fn top(&self) -> usize {
        kernel_stack_position(self.pid).1
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        unmap_kernel_stack(self.pid);
    }
}

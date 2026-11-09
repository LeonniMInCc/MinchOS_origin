//! 批处理系统：一个接一个地运行用户程序。
//!
//! 这是操作系统最原始的形态 —— 程序排好队，跑完一个换下一个，
//! 中间不需要人工干预。第 4 周会升级成"多道程序"（多个程序同时在内存里
//! 轮流执行），第 9 周才有真正的进程。
//!
//! # 本章的内存布局
//!
//! ```text
//!   0x80200000  ┌──────────────────┐
//!               │  内核代码与数据   │
//!               │  内核栈 / 用户栈  │  ← 都是内核 .bss 里的静态数组
//!   0x80518000  ├──────────────────┤
//!               │       ...        │
//!   0x80400000  ├──────────────────┤
//!               │  用户程序加载区   │  ← 每次只装一个，装完覆盖
//!   0x80420000  └──────────────────┘
//! ```
//!
//! **注意：这里没有任何隔离。** 用户程序拿到的是真实物理地址，
//! 它完全有能力改写内核的内存 —— 硬件不会阻止。我们只能在系统调用
//! 的入口处做软件检查（见 [`is_user_accessible`]）。
//!
//! 这正是第 7–8 周要引入分页的原因：把"用户程序看到的地址"和
//! "真实物理地址"分开，让硬件来强制隔离。

use crate::config::{
    APP_BASE_ADDRESS, APP_SIZE_LIMIT, KERNEL_STACK_SIZE, MAX_APP_NUM, USER_STACK_SIZE,
};
use crate::sbi::shutdown;
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use core::arch::asm;
use core::mem::size_of;

// ---------------------------------------------------------------------------
// 两个栈
// ---------------------------------------------------------------------------

/// 内核栈。用户程序陷入内核后，内核在这上面运行。
///
/// `#[repr(align(4096))]` 让它页对齐 —— 现在只是整洁，
/// 第 8 周给内核栈加 guard page 时会变成硬性要求。
#[repr(align(4096))]
struct Stack<const N: usize> {
    // 只取地址、从不按字段读写 —— 栈的实际使用是由 CPU 通过 sp 完成的
    #[allow(dead_code)]
    data: [u8; N],
}

static mut KERNEL_STACK: Stack<KERNEL_STACK_SIZE> = Stack {
    data: [0; KERNEL_STACK_SIZE],
};

static mut USER_STACK: Stack<USER_STACK_SIZE> = Stack {
    data: [0; USER_STACK_SIZE],
};

/// 栈顶。RISC-V 的栈向低地址生长，所以"顶"是高地址那一端。
fn kernel_stack_top() -> usize {
    &raw const KERNEL_STACK as usize + KERNEL_STACK_SIZE
}

fn user_stack_top() -> usize {
    &raw const USER_STACK as usize + USER_STACK_SIZE
}

fn user_stack_bottom() -> usize {
    &raw const USER_STACK as usize
}

/// 把一个 [`TrapContext`] 压到内核栈顶，返回它的地址。
///
/// `__restore` 需要的正是这个地址（通过 `a0` 传入）。
fn push_trap_context(cx: TrapContext) -> usize {
    let ptr = (kernel_stack_top() - size_of::<TrapContext>()) as *mut TrapContext;
    // SAFETY: 内核栈是静态分配的，减去一个 TrapContext 的大小后仍在栈内
    // （TrapContext 只有 272 字节，而内核栈有 8 KiB）。
    unsafe {
        ptr.write(cx);
    }
    ptr as usize
}

// ---------------------------------------------------------------------------
// 用户程序表
// ---------------------------------------------------------------------------

extern "C" {
    /// 由 `build.rs` 生成的 `link_app.S` 提供。
    ///
    /// 内存布局是：程序个数，然后是 `num_app + 1` 个地址，
    /// 第 `i` 个程序的字节范围是 `[app_start[i], app_start[i+1])`。
    static _num_app: usize;
}

struct AppManager {
    num_app: usize,
    /// 下一个要运行的程序编号
    current_app: usize,
    app_start: [usize; MAX_APP_NUM + 1],
}

impl AppManager {
    const fn empty() -> Self {
        Self {
            num_app: 0,
            current_app: 0,
            app_start: [0; MAX_APP_NUM + 1],
        }
    }

    /// 从 `link_app.S` 里读出程序表。
    fn load_table(&mut self) {
        let table = &raw const _num_app;
        // SAFETY: link_app.S 保证了这块数据的布局，表长由 num_app 决定。
        unsafe {
            let num_app = table.read_volatile();
            assert!(
                num_app <= MAX_APP_NUM,
                "用户程序数量 {} 超过上限 {}",
                num_app,
                MAX_APP_NUM
            );
            let raw = core::slice::from_raw_parts(table.add(1), num_app + 1);
            self.app_start[..=num_app].copy_from_slice(raw);
            self.num_app = num_app;
        }
    }

    /// 把第 `app_id` 个程序拷贝到加载区。
    ///
    /// # Safety
    ///
    /// 会覆写 `[APP_BASE_ADDRESS, APP_BASE_ADDRESS + APP_SIZE_LIMIT)`。
    /// 调用方必须确保当前没有程序正在这块区域里运行。
    unsafe fn load_app(&self, app_id: usize) {
        assert!(app_id < self.num_app);
        let (start, end) = (self.app_start[app_id], self.app_start[app_id + 1]);
        let size = end - start;
        assert!(
            size <= APP_SIZE_LIMIT,
            "程序 {} 有 {} 字节，超过加载区上限 {} 字节",
            app_id,
            size,
            APP_SIZE_LIMIT
        );

        // 先清空整个加载区。不清的话，上一个程序的残留字节会留在
        // 新程序的末尾之后 —— 万一新程序跑飞过去，就会执行到别人的代码。
        core::slice::from_raw_parts_mut(APP_BASE_ADDRESS as *mut u8, APP_SIZE_LIMIT).fill(0);

        let src = core::slice::from_raw_parts(start as *const u8, size);
        let dst = core::slice::from_raw_parts_mut(APP_BASE_ADDRESS as *mut u8, size);
        dst.copy_from_slice(src);

        // 关键：我们刚刚把一堆数据当作**代码**写进了内存。
        //
        // CPU 的取指单元有自己的缓存，它可能还存着这块地址的旧内容 ——
        // 也就是上一个程序的指令。`fence.i` 让 CPU 丢弃指令缓存，
        // 重新从内存取指。
        //
        // 忘了这条指令，症状是"新程序莫名其妙执行了上一个程序的代码"，
        // 而且在 QEMU 上未必复现（QEMU 的 TCG 可能不模拟这个细节），
        // 一上真机就崩。属于典型的"能跑但是错的"。
        asm!("fence.i");

        debug!(
            "已装载程序 {}：{} 字节，从 {:#x} 拷贝到 {:#x}",
            app_id, size, start, APP_BASE_ADDRESS
        );
    }

    fn print_info(&self) {
        info!("共 {} 个用户程序：", self.num_app);
        for i in 0..self.num_app {
            info!(
                "  [{}] [{:#012x}, {:#012x})  {} 字节",
                i,
                self.app_start[i],
                self.app_start[i + 1],
                self.app_start[i + 1] - self.app_start[i]
            );
        }
    }
}

static APP_MANAGER: UPSafeCell<AppManager> =
    // SAFETY: 单核，且所有访问都在 exclusive_access 的短作用域内。
    unsafe { UPSafeCell::new(AppManager::empty()) };

/// 读取用户程序表并打印。
pub fn init() {
    let mut manager = APP_MANAGER.exclusive_access();
    manager.load_table();
    manager.print_info();
}

/// 用户程序可访问的地址范围检查。
///
/// 内核在系统调用入口用它验证用户传来的指针。本章没有分页，
/// 这是唯一的防线。
pub fn is_user_accessible(start: usize, len: usize) -> bool {
    // 先防整数溢出：用户传 (start=0xffff_ffff_ffff_ffff, len=2) 的话，
    // start + len 会绕回一个很小的数，让后面的比较全部失效。
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let in_app_region = start >= APP_BASE_ADDRESS && end <= APP_BASE_ADDRESS + APP_SIZE_LIMIT;
    let in_user_stack = start >= user_stack_bottom() && end <= user_stack_top();
    in_app_region || in_user_stack
}

/// 装载并运行下一个用户程序。永不返回。
///
/// 程序全部跑完就关机。
pub fn run_next_app() -> ! {
    // 这个作用域至关重要。
    //
    // 下面的 __restore 是一去不回的 —— 它直接 sret 到用户态，
    // 这个函数的栈帧再也不会被执行。如果 RefMut 还活着，它的 Drop
    // 就永远不会运行，借用标记会一直是"已借出"。
    // 下一次有程序 exit 时，exclusive_access() 会直接 panic。
    //
    // 这正是用 RefCell 而不是裸 UnsafeCell 的价值：它把这个错误
    // 变成一个明确的崩溃，而不是悄无声息的数据竞争。
    let app_id = {
        let mut manager = APP_MANAGER.exclusive_access();
        if manager.current_app >= manager.num_app {
            info!("全部 {} 个用户程序执行完毕，关机", manager.num_app);
            drop(manager); // shutdown 也不返回，同样要手动释放
            shutdown(false);
        }
        let id = manager.current_app;
        // SAFETY: 上一个程序已经 exit 或被杀掉，加载区没人在用。
        unsafe {
            manager.load_app(id);
        }
        manager.current_app += 1;
        id
    };

    info!("—— 开始运行程序 {} ——", app_id);

    // 伪造一个"仿佛刚从用户态陷入内核"的现场，然后走正常的返回路径。
    // 启动一个程序 = 从一次不存在的 trap 返回。
    let cx_addr = push_trap_context(TrapContext::app_init_context(
        APP_BASE_ADDRESS,
        user_stack_top(),
    ));

    extern "C" {
        fn __restore(cx_addr: usize);
    }

    // SAFETY: cx_addr 指向刚压入内核栈的合法 TrapContext。
    unsafe {
        __restore(cx_addr);
    }

    unreachable!("__restore 不应该返回")
}

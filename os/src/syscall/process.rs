//! 进程（本章还只是"任务"）相关的系统调用。
//!
//! 第 9 周引入真正的进程模型后，`exit` 会变成"通知父进程、回收资源、
//! 变成僵尸等待被 wait"。现在它只是把任务标记成已退出然后换人。

use crate::task::{current_task_id, exit_current_and_run_next, suspend_current_and_run_next};
use crate::timer::get_time_ms;

/// `exit(code)` —— 当前任务主动退出。永不返回。
pub fn sys_exit(exit_code: i32) -> ! {
    if exit_code == 0 {
        info!("任务 {} 正常退出，退出码 {}", current_task_id(), exit_code);
    } else {
        warn!("任务 {} 退出，退出码 {}", current_task_id(), exit_code);
    }
    exit_current_and_run_next()
}

/// `yield()` —— 主动让出 CPU。
///
/// 这是第 4 周整章的意义所在：**协作式调度**。
/// 任务自愿放弃 CPU，调度器挑下一个就绪任务跑。
///
/// 注意这个函数是**会返回**的 —— 但返回的时刻是"当前任务再次被调度上来"。
/// 中间可能已经跑过了好几个别的任务。这种"时间上不连续的返回"是
/// 内核代码的常态，初看很别扭，习惯了就好。
///
/// 协作式调度的致命弱点也在这里：任务不 yield，谁也拿它没办法。
/// 一个 `loop {}` 就能把整个系统卡死。第 5 周的时钟中断才能解决这个问题。
pub fn sys_yield() -> isize {
    suspend_current_and_run_next();
    0
}

/// `get_time()` —— 返回开机以来的毫秒数。
///
/// 用户程序自己读不到 `time` CSR：那是 S 态才能访问的寄存器，
/// U 态执行 `csrr` 会触发非法指令异常。所以时间也得向内核要。
///
/// 有了它，用户程序就能测量自己"实际占用了多久 CPU" ——
/// 在抢占式调度下，这个数字会明显大于它自己的计算量，
/// 差额就是被别的任务占走的时间。
pub fn sys_get_time() -> isize {
    get_time_ms() as isize
}

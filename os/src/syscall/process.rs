//! 进程相关的系统调用。
//!
//! 第 3 周是批处理系统：程序一个接一个跑，`exit` 就意味着"换下一个"。
//! 第 9 周引入真正的进程模型后，`exit` 会变成"通知父进程、回收资源、
//! 变成僵尸等待被 wait"。

use crate::batch::run_next_app;

/// `exit(code)` —— 用户程序主动退出。
///
/// 返回类型是 `!`：这个调用永远不会返回到用户程序。
/// [`run_next_app`] 会直接 `__restore` 到下一个程序，
/// 当前程序的执行流就此终止。
pub fn sys_exit(exit_code: i32) -> ! {
    if exit_code == 0 {
        info!("用户程序正常退出，退出码 {}", exit_code);
    } else {
        warn!("用户程序退出，退出码 {}", exit_code);
    }
    run_next_app()
}

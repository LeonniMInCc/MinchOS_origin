//! CCNUOS 的 shell。
//!
//! 一个 shell 本质上只做三件事，而这三件事我们前几周刚好都做完了：
//!
//! | 做什么 | 靠什么 |
//! |---|---|
//! | 读一行命令 | `sys_read`（第 10 周） |
//! | 开一个新进程去执行 | `fork` + `exec`（第 9 周） |
//! | 等它结束并拿到退出码 | `waitpid`（第 9 周） |
//!
//! **shell 不是内核的一部分**，它就是一个普通的用户程序。
//! 这一点是 Unix 设计里最重要的观念之一：命令解释器没有任何特权，
//! 你完全可以写一个自己的换掉它。
//!
//! # 为什么不用 `String`
//!
//! 用户库还没有堆分配器（`#![no_std]` 且没接 `alloc`），所以命令行
//! 用一个固定大小的数组。对一个教学 shell 来说 256 字节绰绰有余，
//! 而且省掉了在用户态再实现一个分配器的麻烦。

#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{exec, fork, getchar, waitpid};

const LINE_MAX: usize = 255; // 留一个字节给 exec 需要的 \0
const CR: u8 = 0x0d; // 回车
const LF: u8 = 0x0a; // 换行
const BS: u8 = 0x08; // Backspace
const DEL: u8 = 0x7f; // 大多数终端的退格键发的是这个

/// 一行命令的缓冲区。
struct Line {
    buf: [u8; LINE_MAX + 1],
    len: usize,
}

impl Line {
    fn new() -> Self {
        Self {
            buf: [0; LINE_MAX + 1],
            len: 0,
        }
    }

    fn push(&mut self, ch: u8) -> bool {
        if self.len >= LINE_MAX {
            return false;
        }
        self.buf[self.len] = ch;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> bool {
        if self.len == 0 {
            return false;
        }
        self.len -= 1;
        self.buf[self.len] = 0;
        true
    }

    fn clear(&mut self) {
        self.buf = [0; LINE_MAX + 1];
        self.len = 0;
    }

    fn as_str(&self) -> &str {
        // 只接受合法 UTF-8。用户敲进来的字节不保证是，所以不能 unwrap。
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

fn prompt() {
    print!("ccnuos> ");
}

/// 执行一条命令：fork 出子进程去 exec，父进程等它结束。
fn run_command(cmd: &str) {
    let pid = fork();

    if pid == 0 {
        // ---- 子进程 ----
        // exec 成功的话地址空间就被换掉了，下面的代码属于旧程序，不会执行
        exec(cmd);
        println!("ccnuos: 找不到命令 \"{}\"", cmd);
        // 用 127 退出，和大多数 shell 对"command not found"的约定一致
        user_lib::sys_exit(127);
    }

    // ---- 父进程（shell 自己）----
    let mut exit_code: i32 = 0;
    let reaped = waitpid(pid, &mut exit_code);

    if exit_code != 0 {
        println!("[{}] 退出码 {}", reaped, exit_code);
    }
}

#[no_mangle]
pub fn main() -> i32 {
    println!();
    println!("CCNUOS shell —— 输入程序名回车执行，输入 help 看有哪些程序，exit 退出");

    let mut line = Line::new();
    prompt();

    loop {
        let ch = getchar();
        match ch {
            CR | LF => {
                println!();
                let cmd = line.as_str();
                match cmd.trim() {
                    "" => {}
                    "exit" => {
                        println!("再见。");
                        return 0;
                    }
                    "help" => print_help(),
                    other => run_command(other),
                }
                line.clear();
                prompt();
            }

            BS | DEL => {
                if line.pop() {
                    // 退格：光标左移、打空格盖掉、再左移。
                    // 串口没有"删除一个字符"这种操作，只能这么模拟。
                    print!("{}", BS as char);
                    print!(" ");
                    print!("{}", BS as char);
                }
            }

            // 可打印字符
            0x20..=0x7e => {
                if line.push(ch) {
                    // 回显。串口默认不回显，不打出来的话敲什么都看不见。
                    print!("{}", ch as char);
                }
            }

            // 其余（方向键之类的转义序列）一律忽略
            _ => {}
        }
    }
}

fn print_help() {
    println!("可以运行的程序：");
    for name in [
        "00hello_world",
        "01yield_a / 02yield_b / 03yield_c   地址空间隔离演示",
        "04spin_x / 05spin_y                 抢占演示（从不 yield）",
        "06timer                             计时",
        "07bad_syscall_arg                   越界系统调用参数（会被拒绝）",
        "08bad_instruction                   非法指令（会被杀）",
        "09bad_address                       访问空地址（会被杀）",
        "10bad_kernel_write                  越权写内核（会被杀）",
        "11forktest                          fork 语义演示",
    ] {
        println!("  {}", name);
    }
    println!("内置命令：help / exit");
}

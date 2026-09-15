//! CCNUOS 的 shell。
//!
//! 第 11 周起它还支持**管道**：`12words | 13upper | 14wc`。
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

use user_lib::{close, dup, exec, fork, getchar, pipe, waitpid, STDIN, STDOUT};

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

/// 管道最多支持几级。用固定数组是因为用户库还没有堆分配器。
const MAX_STAGES: usize = 4;

/// 按 `|` 切开命令行。返回段数，语法有问题返回 `None`。
fn split_pipeline<'a>(cmd: &'a str, out: &mut [&'a str; MAX_STAGES]) -> Option<usize> {
    let mut n = 0;
    for part in cmd.split('|') {
        let part = part.trim();
        if part.is_empty() {
            return None; // "a | | b" 或者 "a |"
        }
        if n >= MAX_STAGES {
            return None;
        }
        out[n] = part;
        n += 1;
    }
    if n == 0 {
        None
    } else {
        Some(n)
    }
}

/// 执行一条命令（可能是一条管道）。
fn run_command(cmd: &str) {
    let mut stages = [""; MAX_STAGES];
    let Some(n) = split_pipeline(cmd, &mut stages) else {
        println!("ccnuos: 管道语法错误（最多 {} 级，且每段不能为空）", MAX_STAGES);
        return;
    };

    // n 段需要 n-1 个管道
    let mut pipes = [[0i32; 2]; MAX_STAGES - 1];
    for p in pipes.iter_mut().take(n.saturating_sub(1)) {
        if pipe(p) < 0 {
            println!("ccnuos: 创建管道失败");
            return;
        }
    }

    let mut pids = [0isize; MAX_STAGES];

    for i in 0..n {
        let pid = fork();

        if pid == 0 {
            // ---- 子进程：先把自己的 stdin/stdout 接到管道上 ----
            //
            // 手法是"关掉再复制"：close(0) 让 0 成为最小的空闲号，
            // 紧接着 dup 出来的副本必然落在 0 上。这套几十年来的惯用法
            // 完全建立在"alloc_fd 返回最小空闲号"这条 POSIX 规定上。
            if i > 0 {
                close(STDIN);
                dup(pipes[i - 1][0] as usize);
            }
            if i + 1 < n {
                close(STDOUT);
                dup(pipes[i][1] as usize);
            }

            // **必须关掉所有管道 fd 的原始副本。**
            // 它们已经被 dup 到 0/1 了，留着的话这个进程自己就持有着
            // 管道的写端，于是下游永远等不到 EOF —— 整条管道死锁。
            for p in pipes.iter().take(n - 1) {
                close(p[0] as usize);
                close(p[1] as usize);
            }

            exec(stages[i]);
            println!("ccnuos: 找不到命令 \"{}\"", stages[i]);
            user_lib::sys_exit(127);
        }

        pids[i] = pid;
    }

    // ---- 父进程（shell 自己）----
    //
    // 同样必须关掉自己那份。shell 不参与数据传输，但只要它还握着写端，
    // 管道就不算"所有写端都关了"，末端的 wc 就会一直等下去。
    // **这是管道编程里最经典的一个 bug。**
    for p in pipes.iter().take(n - 1) {
        close(p[0] as usize);
        close(p[1] as usize);
    }

    for pid in pids.iter().take(n) {
        let mut exit_code: i32 = 0;
        waitpid(*pid, &mut exit_code);
        if exit_code != 0 {
            println!("[{}] 退出码 {}", pid, exit_code);
        }
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
        "12words / 13upper / 14wc            管道演示（过滤器）",
        "15pipetest                          直接用 pipe() 的例子",
    ] {
        println!("  {}", name);
    }
    println!("内置命令：help / exit");
    println!("支持管道，最多 {} 级，例如：", MAX_STAGES);
    println!("  12words | 13upper");
    println!("  12words | 13upper | 14wc");
}

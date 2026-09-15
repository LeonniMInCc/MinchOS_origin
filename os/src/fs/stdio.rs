//! 标准输入与标准输出。都接在 SBI 串口上。

use super::File;
use crate::mm::UserBuffer;
use crate::drivers::uart;
use crate::sync::wait_on_io;

/// 标准输入（fd 0）。
pub struct Stdin;

/// 标准输出（fd 1 和 fd 2）。
pub struct Stdout;

impl File for Stdin {
    fn readable(&self) -> bool {
        true
    }

    fn writable(&self) -> bool {
        false
    }

    /// 读一个字符。**会阻塞到真的有输入为止。**
    ///
    /// # 第 14 周：从轮询改成了真正的阻塞
    ///
    /// 第 10–13 周的实现是忙等：反复问 SBI "有字符吗"，没有就 `yield`。
    /// 那有两个真实代价 —— 等待的进程白烧时间片，而且**两次轮询之间到达
    /// 的字符会丢**（第 10 章实测到过：粘贴长命令会缺字母）。
    ///
    /// 现在串口收到字符会产生中断，内核在中断处理程序里把它存进环形
    /// 缓冲区并唤醒等待者。没有输入时，进程从调度器视野里彻底消失。
    ///
    /// # 为什么一次只读一个字符
    ///
    /// 串口是"裸"设备，没有行缓冲。shell 要逐字符处理（回显、退格），
    /// 所以这里也按一次一个来，语义最简单。相当于终端的 raw 模式。
    fn read(&self, mut buf: UserBuffer) -> Option<usize> {
        if buf.is_empty() {
            return Some(0);
        }

        let ch = loop {
            if let Some(c) = uart::try_getchar() {
                break c;
            }
            // 缓冲区空。挂进等待队列然后睡下 —— 串口中断会叫醒我们。
            //
            // 用 `loop` 而不是 `if`：被唤醒只说明"来字符了"，
            // 未必轮得到我们（可能有别的进程先取走了）。
            wait_on_io(uart::reader_queue());
        };

        *buf.bytes_mut().next()? = ch;
        Some(1)
    }

    fn write(&self, _buf: UserBuffer) -> Option<usize> {
        error!("不能往标准输入里写");
        None
    }
}

impl File for Stdout {
    fn readable(&self) -> bool {
        false
    }

    fn writable(&self) -> bool {
        true
    }

    fn read(&self, _buf: UserBuffer) -> Option<usize> {
        error!("不能从标准输出里读");
        None
    }

    fn write(&self, buf: UserBuffer) -> Option<usize> {
        let len = buf.len();

        // 用户程序送来的是任意字节，不保证是合法 UTF-8。先收集再解码，
        // 这样跨页的多字节字符也不会被拆开。
        let bytes: alloc::vec::Vec<u8> = buf.bytes().collect();
        for chunk in bytes.utf8_chunks() {
            print!("{}", chunk.valid());
            if !chunk.invalid().is_empty() {
                // 非法字节用 U+FFFD 替换，而不是让内核 panic ——
                // 用户程序永远不该有能力搞崩内核。
                print!("\u{FFFD}");
            }
        }

        Some(len)
    }
}

//! 标准输入与标准输出。都接在 SBI 串口上。

use super::File;
use crate::mm::UserBuffer;
use crate::sbi::console_getchar;
use crate::task::suspend_current_and_run_next;

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
    /// # "阻塞"是怎么实现的
    ///
    /// SBI 的 `console_getchar` 不阻塞：有就给，没有就返回 -1。
    /// 所以"等"这件事只能自己做 —— 反复问，问不到就 `yield` 让别人先跑：
    ///
    /// ```text
    ///   loop {
    ///       有字符？ → 返回
    ///       没有   → suspend_current_and_run_next()
    ///   }
    /// ```
    ///
    /// 这是**忙等**：没有输入时，这个进程每被调度到一次就白跑一次。
    /// 真正该做的是把进程挂进一个等待队列，等串口中断来了再唤醒 ——
    /// 那需要外部中断和等待队列，是第 14 章的事。
    ///
    /// 在那之前，忙等是可接受的：反正调度器总会把 CPU 分给别的进程，
    /// 只是浪费掉本该属于这个进程的那些时间片。
    ///
    /// # 为什么一次只读一个字符
    ///
    /// 串口是"裸"设备，没有行缓冲的概念。shell 需要逐字符处理
    /// （回显、退格），所以这里也按一次一个来，语义最简单。
    /// 这相当于终端的 raw 模式。
    fn read(&self, mut buf: UserBuffer) -> Option<usize> {
        if buf.is_empty() {
            return Some(0);
        }

        let ch = loop {
            match console_getchar() {
                Some(c) => break c,
                // 没有输入。注意这里能安全地让出 CPU，是因为调用方
                // （sys_read）已经把进程控制块的借用释放掉了 ——
                // 否则切换任务时会撞上 RefCell 的重复借用。
                None => suspend_current_and_run_next(),
            }
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

//! 匿名管道。
//!
//! 管道是 Unix 里最优雅的设计之一：一个只有两端的字节缓冲区，
//! 写端写进去，读端读出来。就这么简单的东西，撑起了整个
//! "小工具串起来干大事" 的哲学。
//!
//! # 它为什么能"连接"两个进程
//!
//! 关键在第 10 周埋下的一件事：**`fork` 复制文件描述符表，但共享底下的
//! 文件对象**。
//!
//! ```text
//!   shell: pipe() → fd 3 (读端), fd 4 (写端)
//!     │
//!     ├─ fork → 子进程 A：close(1), dup(4) ⇒ 它的 stdout 就是管道写端
//!     └─ fork → 子进程 B：close(0), dup(3) ⇒ 它的 stdin  就是管道读端
//! ```
//!
//! 两个子进程各自的 fd 号完全不同，但 `Arc` 指向**同一个环形缓冲区**。
//! A 往 stdout 写，B 从 stdin 就读到了 —— 而 A 和 B 都不知道自己在管道里。
//!
//! # EOF 是怎么判断的
//!
//! 读端读到空缓冲区时，要区分两种情况：
//!
//! - **写端还在**：数据只是还没来，应该等
//! - **写端全关了**：再也不会有数据了，返回 0 表示 EOF
//!
//! 所以缓冲区里存了指向两端的 [`Weak`] 引用。用 `Weak` 而不是 `Arc`：
//! 缓冲区被两端持有，要是它反过来强引用两端，引用计数就永远降不到 0 ——
//! 和第 9 章父子进程那个循环引用是同一个问题。

use super::File;
use crate::mm::UserBuffer;
use crate::sync::UPSafeCell;
use crate::task::suspend_current_and_run_next;
use alloc::sync::{Arc, Weak};

/// 管道缓冲区大小。
///
/// 故意开得小：32 字节很容易被写满，这样"写端阻塞等读端"这条路径
/// 在日常使用中就会被真实地走到，而不是一个从没被测试过的分支。
const RING_SIZE: usize = 32;

#[derive(Copy, Clone, PartialEq, Eq)]
enum Status {
    Empty,
    Normal,
    Full,
}

struct RingBuffer {
    data: [u8; RING_SIZE],
    head: usize,
    tail: usize,
    status: Status,
    /// 指向读端和写端。用来判断"另一头还在不在"。
    read_end: Option<Weak<Pipe>>,
    write_end: Option<Weak<Pipe>>,
}

impl RingBuffer {
    const fn new() -> Self {
        Self {
            data: [0; RING_SIZE],
            head: 0,
            tail: 0,
            status: Status::Empty,
            read_end: None,
            write_end: None,
        }
    }

    fn available_read(&self) -> usize {
        match self.status {
            Status::Empty => 0,
            Status::Full => RING_SIZE,
            Status::Normal if self.tail > self.head => self.tail - self.head,
            Status::Normal => RING_SIZE - self.head + self.tail,
        }
    }

    fn available_write(&self) -> usize {
        RING_SIZE - self.available_read()
    }

    fn read_byte(&mut self) -> u8 {
        debug_assert!(self.available_read() > 0);
        let byte = self.data[self.head];
        self.head = (self.head + 1) % RING_SIZE;
        self.status = if self.head == self.tail {
            Status::Empty
        } else {
            Status::Normal
        };
        byte
    }

    fn write_byte(&mut self, byte: u8) {
        debug_assert!(self.available_write() > 0);
        self.data[self.tail] = byte;
        self.tail = (self.tail + 1) % RING_SIZE;
        self.status = if self.tail == self.head {
            Status::Full
        } else {
            Status::Normal
        };
    }

    /// 所有写端都关了吗？关了就再也不会有新数据，读端该看到 EOF 了。
    fn all_write_ends_closed(&self) -> bool {
        self.write_end
            .as_ref()
            .map(|w| w.upgrade().is_none())
            .unwrap_or(true)
    }

    /// 所有读端都关了吗？关了就再也没人会来取，写端不该继续傻等。
    ///
    /// 真实的 Unix 在这种情况下会给写进程发 `SIGPIPE`（默认杀掉它）。
    /// 我们还没有信号，所以只能让 `write` 返回一个错误。
    fn all_read_ends_closed(&self) -> bool {
        self.read_end
            .as_ref()
            .map(|w| w.upgrade().is_none())
            .unwrap_or(true)
    }
}

/// 管道的一端。
pub struct Pipe {
    readable: bool,
    writable: bool,
    buffer: Arc<UPSafeCell<RingBuffer>>,
}

/// 造一个管道，返回 `(读端, 写端)`。
pub fn make_pipe() -> (Arc<Pipe>, Arc<Pipe>) {
    // SAFETY: 单核，且所有访问都限制在 exclusive_access 的短作用域内。
    let buffer = Arc::new(unsafe { UPSafeCell::new(RingBuffer::new()) });

    let read_end = Arc::new(Pipe {
        readable: true,
        writable: false,
        buffer: Arc::clone(&buffer),
    });
    let write_end = Arc::new(Pipe {
        readable: false,
        writable: true,
        buffer: Arc::clone(&buffer),
    });

    {
        let mut ring = buffer.exclusive_access();
        ring.read_end = Some(Arc::downgrade(&read_end));
        ring.write_end = Some(Arc::downgrade(&write_end));
    }

    (read_end, write_end)
}

impl File for Pipe {
    fn readable(&self) -> bool {
        self.readable
    }

    fn writable(&self) -> bool {
        self.writable
    }

    /// 读。缓冲区空就等，直到有数据或者写端全关了（EOF，返回 0）。
    ///
    /// 语义和 `read(2)` 一致：**只要读到一个字节就返回**，不必填满缓冲区。
    fn read(&self, mut buf: UserBuffer) -> Option<usize> {
        if !self.readable {
            error!("试图从管道的写端读");
            return None;
        }
        let want = buf.len();
        if want == 0 {
            return Some(0);
        }

        let mut dst = buf.bytes_mut();
        let mut got = 0;

        loop {
            let mut ring = self.buffer.exclusive_access();
            let available = ring.available_read();

            if available == 0 {
                let eof = ring.all_write_ends_closed();
                // 必须先放掉借用再让出 CPU —— 否则切回来时
                // RefCell 会发现重复借用。见第 10 章 10.5 节。
                drop(ring);
                if eof {
                    return Some(0); // 写端全关了，再也不会有数据
                }
                suspend_current_and_run_next();
                continue;
            }

            for _ in 0..available.min(want) {
                match dst.next() {
                    Some(slot) => {
                        *slot = ring.read_byte();
                        got += 1;
                    }
                    None => break,
                }
            }
            return Some(got);
        }
    }

    /// 写。缓冲区满就等，直到腾出空位或者读端全关了。
    fn write(&self, buf: UserBuffer) -> Option<usize> {
        if !self.writable {
            error!("试图往管道的读端写");
            return None;
        }
        let want = buf.len();
        if want == 0 {
            return Some(0);
        }

        let mut src = buf.bytes();
        let mut sent = 0;

        loop {
            let mut ring = self.buffer.exclusive_access();
            let space = ring.available_write();

            if space == 0 {
                let orphaned = ring.all_read_ends_closed();
                drop(ring);
                if orphaned {
                    // 没人会来取了。真实系统这里会发 SIGPIPE，
                    // 我们还没有信号，只能报告已经写出去多少。
                    error!("往一个没有读端的管道里写");
                    return Some(sent);
                }
                suspend_current_and_run_next();
                continue;
            }

            for _ in 0..space {
                match src.next() {
                    Some(byte) => {
                        ring.write_byte(byte);
                        sent += 1;
                    }
                    None => break,
                }
            }
            drop(ring);

            if sent == want {
                return Some(sent);
            }
            // 还没写完（缓冲区被填满了），下一轮继续
        }
    }
}

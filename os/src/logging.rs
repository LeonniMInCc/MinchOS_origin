//! 分级日志。
//!
//! 内核调试没有断点以外的奢侈手段，日志就是主要的观察窗口。
//! 这里不用 `log` crate，自己实现一个够用的：五个级别、带颜色、
//! 通过编译期环境变量 `LOG` 过滤。
//!
//! ```sh
//! make run            # 默认 INFO
//! LOG=DEBUG make run  # 打开 DEBUG
//! LOG=TRACE make run  # 全开
//! ```
//!
//! 因为 [`MAX_LEVEL`] 是编译期常量，被过滤掉的日志会被 LLVM 整段消除，
//! 运行时零开销 —— 这正是我们敢在热路径（比如任务切换）上撒日志的原因。

use core::fmt;

/// 日志级别。数值越小越严重。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

/// 编译期确定的日志上限，由环境变量 `LOG` 指定，缺省为 `Info`。
pub const MAX_LEVEL: Level = level_from_env();

/// 解析 `LOG` 环境变量。只看首字母，`DEBUG` / `debug` / `d` 都认。
const fn level_from_env() -> Level {
    match option_env!("LOG") {
        Some(s) => {
            let bytes = s.as_bytes();
            if bytes.is_empty() {
                return Level::Info;
            }
            match bytes[0] {
                b'E' | b'e' => Level::Error,
                b'W' | b'w' => Level::Warn,
                b'I' | b'i' => Level::Info,
                b'D' | b'd' => Level::Debug,
                b'T' | b't' => Level::Trace,
                _ => Level::Info,
            }
        }
        None => Level::Info,
    }
}

impl Level {
    /// 该级别的显示名与 ANSI 颜色码。
    const fn style(self) -> (&'static str, u8) {
        match self {
            Level::Error => ("ERROR", 31), // 红
            Level::Warn => ("WARN ", 93),  // 亮黄
            Level::Info => ("INFO ", 34),  // 蓝
            Level::Debug => ("DEBUG", 32), // 绿
            Level::Trace => ("TRACE", 90), // 灰
        }
    }
}

/// 日志落地点。级别不够就直接返回，调用会被优化掉。
pub fn log(level: Level, args: fmt::Arguments) {
    if level > MAX_LEVEL {
        return;
    }
    let (name, color) = level.style();
    crate::console::print(format_args!("\u{1b}[{}m[{}] {}\u{1b}[0m\n", color, name, args));
}

#[macro_export]
macro_rules! error {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::logging::log($crate::logging::Level::Error, format_args!($fmt $(, $($arg)+)?))
    };
}

#[macro_export]
macro_rules! warn {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::logging::log($crate::logging::Level::Warn, format_args!($fmt $(, $($arg)+)?))
    };
}

#[macro_export]
macro_rules! info {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::logging::log($crate::logging::Level::Info, format_args!($fmt $(, $($arg)+)?))
    };
}

#[macro_export]
macro_rules! debug {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::logging::log($crate::logging::Level::Debug, format_args!($fmt $(, $($arg)+)?))
    };
}

#[macro_export]
macro_rules! trace {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::logging::log($crate::logging::Level::Trace, format_args!($fmt $(, $($arg)+)?))
    };
}

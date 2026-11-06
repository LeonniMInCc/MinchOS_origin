//! 结构化事件输出：给可视化工具用的原始素材。
//!
//! # 和日志的区别
//!
//! `info!` 那套是给**人**看的：自然语言、随手写、格式随意。
//! 这里输出的是给**程序**看的：字段固定、一行一条、可以被脚本解析。
//!
//! ```text
//!   @EV sched t=123456 pid=2
//!   @EV sys   t=123460 pid=2 id=64
//!   @EV exit  t=123900 pid=2 code=0
//! ```
//!
//! 宿主机上的 `tools/trace2html.py` 收集这些行，渲染成调度甘特图、
//! 进程树、系统调用时序。内核不需要知道图长什么样，
//! **它只负责说清楚发生了什么** —— 这个分工让可视化可以随便改，
//! 而内核一行都不用动。
//!
//! # 默认是关闭的
//!
//! 事件输出走串口，而串口很慢（每个字符一次 SBI 调用）。开着跑，
//! 调度行为本身就会被观测行为改变 —— 典型的观测者效应。
//!
//! 所以它由编译期环境变量 `TRACE` 控制，和第 2 章的 `LOG` 同一个套路：
//!
//! ```sh
//! make trace          # 打开事件输出并生成可视化
//! make run            # 默认关闭，零开销
//! ```
//!
//! 关闭时 [`ENABLED`] 是编译期常量 `false`，所有事件调用会被 LLVM
//! 整段消除，连一条比较指令都不剩。

use crate::timer::get_time;

/// 事件输出是否开启。编译期常量。
pub const ENABLED: bool = option_env!("TRACE").is_some();

/// 每条事件行的前缀。挑一个正常输出里不会出现的记号，
/// 这样解析脚本可以直接按行过滤，不必担心和程序输出混淆。
const TAG: &str = "@EV";

/// 时间戳，单位微秒。
///
/// 毫秒对调度来说太粗了 —— 一个时间片才 10 ms，而一次系统调用只要几微秒。
/// `time` CSR 在 QEMU virt 上是 10 MHz，除以 10 正好得到微秒。
fn now_us() -> usize {
    get_time() / 10
}

/// 内部使用：打印一条事件。
#[doc(hidden)]
pub fn emit(kind: &str, args: core::fmt::Arguments) {
    crate::console::print(format_args!("{} {} t={} {}\n", TAG, kind, now_us(), args));
}

/// 输出一条结构化事件。`TRACE` 没开时整条语句被消除。
#[macro_export]
macro_rules! event {
    ($kind:literal $(, $fmt:literal $(, $($arg:tt)+)?)?) => {
        if $crate::trace::ENABLED {
            $crate::trace::emit($kind, format_args!(concat!("" $(, $fmt)?) $($(, $($arg)+)?)?))
        }
    };
}

/// 开机时打一条头，让解析脚本知道从哪开始、以及时间基准。
pub fn banner() {
    if ENABLED {
        crate::console::print(format_args!(
            "{} begin t={} unit=us version=0.15\n",
            TAG,
            now_us()
        ));
        info!("事件输出已开启（TRACE=1）—— 可视化素材会混在串口输出里");
    }
}

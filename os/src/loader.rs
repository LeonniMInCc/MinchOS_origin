//! 嵌在内核镜像里的用户程序。
//!
//! # 第 8 周，这个模块缩水了一大半
//!
//! 第 3–7 周它要干很多脏活：给每个程序划一块固定的物理槽位、维护每任务的
//! 内核栈和用户栈数组、把 ELF 的段拷到写死的地址、再手写一个
//! `is_accessible_by` 去检查用户传来的指针。
//!
//! 现在这些全没了。有了地址空间之后：
//!
//! - 装载变成 [`crate::mm::MemorySet::from_elf`]：按需分配页帧并建立映射
//! - 内核栈由内核地址空间统一分配（还自带 guard page）
//! - 用户栈是地址空间的一部分
//! - 指针检查变成"查一下用户页表里这页带不带 `U`"
//!
//! 剩下的只有一件事：**把 ELF 的字节找出来**。
//!
//! 本章还没有文件系统，用户程序在编译期由 `build.rs` 生成的 `link_app.S`
//! 用 `.incbin` 嵌进内核的 `.data` 段。第 13 周做完文件系统后，
//! 这个模块会被"从磁盘读文件"取代。

use alloc::vec::Vec;

extern "C" {
    /// 由 `build.rs` 生成的 `link_app.S` 提供。
    ///
    /// 内存布局：`[程序个数 N, start_0, start_1, ..., start_N]`，
    /// 第 `i` 个程序的字节范围是 `[start_i, start_{i+1})`。
    static _num_app: usize;
}

fn app_table() -> *const usize {
    &raw const _num_app
}

/// 链接进内核的用户程序数量。
pub fn num_app() -> usize {
    // SAFETY: link_app.S 保证表头就是一个 usize 的计数。
    unsafe { app_table().read_volatile() }
}

/// 第 `app_id` 个程序的 ELF 字节。
pub fn get_app_data(app_id: usize) -> &'static [u8] {
    assert!(app_id < num_app(), "程序编号 {app_id} 越界");
    // SAFETY: link_app.S 保证了表的布局；这些字节嵌在内核 .data 段里，
    // 生命周期和内核一样长。
    unsafe {
        let starts = app_table().add(1);
        let start = starts.add(app_id).read_volatile();
        let end = starts.add(app_id + 1).read_volatile();
        core::slice::from_raw_parts(start as *const u8, end - start)
    }
}

extern "C" {
    /// 由 `build.rs` 生成：所有程序的名字，一个接一个的 `\0` 结尾字符串。
    static _app_names: u8;
}

/// 所有用户程序的名字，顺序和编号一致。
///
/// 每次调用都重新解析一遍。程序数量是个位数，而调用点只有 `exec`，
/// 没必要为它维护一份缓存。
pub fn app_names() -> Vec<&'static str> {
    let mut names = Vec::new();
    let mut p = &raw const _app_names;

    for _ in 0..num_app() {
        // SAFETY: link_app.S 保证这里有 num_app() 个连续的 \0 结尾字符串。
        unsafe {
            let start = p;
            let mut len = 0;
            while p.read_volatile() != 0 {
                p = p.add(1);
                len += 1;
            }
            p = p.add(1); // 跳过 \0

            let bytes = core::slice::from_raw_parts(start, len);
            names.push(core::str::from_utf8(bytes).expect("程序名不是合法 UTF-8"));
        }
    }
    names
}

/// 按名字找一个用户程序的 ELF 字节。`sys_exec` 用它。
pub fn get_app_data_by_name(name: &str) -> Option<&'static [u8]> {
    app_names()
        .iter()
        .position(|&n| n == name)
        .map(get_app_data)
}

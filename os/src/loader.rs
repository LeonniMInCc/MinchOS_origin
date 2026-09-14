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

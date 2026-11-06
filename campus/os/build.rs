//! 构建脚本。
//!
//! 第 13 周起它只剩一件事：处理 `LOG` 环境变量和几个需要触发重新编译的文件。
//!
//! 之前它还负责生成 `link_app.S`，把所有用户程序的二进制用 `.incbin`
//! 嵌进内核的 `.data` 段 —— 因为那时没有文件系统，内核没有任何办法
//! "读取"一个程序。现在程序都在磁盘上，那段逻辑随着文件系统一起退休了。
//!
//! 内核镜像因此瘦了两百多 KiB，而且**换用户程序不必重新编译内核**。

fn main() {
    // logging.rs 用 option_env!("LOG") 在编译期决定日志级别。
    // 没有这行的话，改了 LOG 也不会触发重新编译，日志级别会诡异地不生效。
    println!("cargo:rerun-if-env-changed=LOG");
    println!("cargo:rerun-if-env-changed=TRACE");
    println!("cargo:rerun-if-changed=src/linker.ld");
    println!("cargo:rerun-if-changed=src/entry.asm");
    println!("cargo:rerun-if-changed=src/trap/trap.S");
    println!("cargo:rerun-if-changed=src/task/switch.S");
}

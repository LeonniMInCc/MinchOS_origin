//! 构建脚本。
//!
//! 除了处理 `LOG` 环境变量，主要工作是生成 `src/link_app.S` ——
//! 一段把所有用户程序的二进制**直接嵌进内核镜像**的汇编。
//!
//! 为什么要这么做？本章还没有文件系统，内核没有任何办法"读取"一个程序。
//! 唯一的出路是在编译期就把它们塞进自己的 `.data` 段里带着走。
//! 第 13 周做完文件系统后，用户程序才能改从磁盘加载。

use std::fs::{read_dir, File};
use std::io::{Result, Write};

/// 用户程序源码目录
const USER_BIN_SRC: &str = "../user/src/bin";
/// 用户程序编译产物目录
const USER_TARGET: &str = "../user/target/riscv64gc-unknown-none-elf/release";

fn main() {
    println!("cargo:rerun-if-env-changed=LOG");
    println!("cargo:rerun-if-changed=src/linker.ld");
    println!("cargo:rerun-if-changed=src/entry.asm");
    println!("cargo:rerun-if-changed=src/trap/trap.S");
    println!("cargo:rerun-if-changed={USER_BIN_SRC}");
    println!("cargo:rerun-if-changed={USER_TARGET}");

    generate_link_app().expect("生成 link_app.S 失败");
}

/// 收集用户程序名，按文件名排序 —— 这也决定了它们的执行顺序，
/// 所以源文件用 `00`、`01` 这样的前缀来控制。
fn app_names() -> Vec<String> {
    let Ok(entries) = read_dir(USER_BIN_SRC) else {
        // 用户程序目录还不存在（比如首次 clone 后单独编译内核），
        // 生成一个空表，内核会报告"0 个程序"然后关机。
        return Vec::new();
    };

    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            name.strip_suffix(".rs").map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

fn generate_link_app() -> Result<()> {
    let apps = app_names();
    let mut f = File::create("src/link_app.S")?;

    writeln!(
        f,
        "# 本文件由 os/build.rs 自动生成，请勿手动修改。\n\
         #\n\
         # _num_app 的布局：\n\
         #   [0]              程序个数 N\n\
         #   [1 ..= N+1]      N+1 个地址，第 i 个程序占 [app_i_start, app_i_end)\n"
    )?;

    writeln!(f, "    .align 3")?;
    writeln!(f, "    .section .data")?;
    writeln!(f, "    .global _num_app")?;
    writeln!(f, "_num_app:")?;
    writeln!(f, "    .quad {}", apps.len())?;
    for i in 0..apps.len() {
        writeln!(f, "    .quad app_{i}_start")?;
    }
    if let Some(last) = apps.len().checked_sub(1) {
        writeln!(f, "    .quad app_{last}_end")?;
    }

    for (i, app) in apps.iter().enumerate() {
        writeln!(f, "\n    # {app}")?;
        writeln!(f, "    .section .data")?;
        writeln!(f, "    .global app_{i}_start")?;
        writeln!(f, "    .global app_{i}_end")?;
        writeln!(f, "    .align 3")?;
        writeln!(f, "app_{i}_start:")?;
        // .incbin 把一个文件的原始字节嵌入当前位置
        writeln!(f, "    .incbin \"{USER_TARGET}/{app}.bin\"")?;
        writeln!(f, "app_{i}_end:")?;
    }

    Ok(())
}

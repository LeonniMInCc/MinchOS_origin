# CCNUOS

一个用 Rust 从零编写的 RISC-V 64 (SV39) 教学操作系统内核，运行在 QEMU `virt` 机器上。

目标不只是"能跑"，而是**每一步都讲得清楚**：仓库按周打 tag，每个 tag 配一篇
`docs/chXX.md`，`git diff v0.1..v0.2` 就是那一章的全部实验内容。

## 当前状态

| 进度 | 内容 |
|---|---|
| ✅ 第 1 周 (`v0.1`) | 裸机启动、SBI 串口输出、分级日志、内存布局自省 |
| ✅ 第 2 周 (`v0.2`) | 伙伴系统堆分配器，`Box`/`Vec`/`String`/`BTreeMap` 可用 |
| ⬜ 第 3 周 | 特权级切换、Trap 处理、第一个用户程序 |

完整计划见 [ROADMAP.md](ROADMAP.md)。

## 快速开始

依赖：Rust stable（含 `riscv64gc-unknown-none-elf` target 与 `llvm-tools` 组件）、QEMU 7.0+。

macOS 一次性准备：

```bash
brew install rustup qemu && rustup default stable && rustup target add riscv64gc-unknown-none-elf && rustup component add rust-src llvm-tools
```

运行：

```bash
make run
```

你应该看到 OpenSBI 的 banner，然后是 CCNUOS 的启动信息、内核内存布局表，最后内核主动关机退出 QEMU。

其他命令：

```bash
make check          # 快速类型检查
make asm            # 反汇编内核，看生成了什么代码
make debug          # 挂起等待 GDB（另开终端跑 make gdb）
LOG=DEBUG make run  # 调高日志级别（ERROR/WARN/INFO/DEBUG/TRACE）
make clean
```

## 目录结构

```
ccnuos/
├── Makefile              构建与运行入口
├── rust-toolchain.toml   钉死工具链版本与 target，保证可复现
├── ROADMAP.md            16 周计划
├── docs/                 每周一篇的讲解文档
└── os/                   内核 crate
    ├── .cargo/config.toml  交叉编译目标 + 链接脚本
    ├── build.rs            让 LOG 环境变量能触发重新编译
    └── src/
        ├── linker.ld       内存布局，把内核钉在 0x80200000
        ├── entry.asm       入口：支起启动栈，跳进 Rust
        ├── main.rs         rust_main：清 .bss、初始化、自检、关机
        ├── config.rs       内核可调参数
        ├── sbi.rs          SBI 调用（ecall）封装
        ├── console.rs      print! / println!
        ├── logging.rs      分级彩色日志
        ├── lang_items.rs   panic handler
        ├── sync.rs         UPSafeCell：单核下的内部可变性
        └── mm/
            └── heap_allocator.rs   伙伴系统分配器 + #[global_allocator]
```

## 设计取舍

**不依赖任何外部 crate。** SBI 封装、日志、控制台都是手写的。对教学项目而言，
`sbi-rt` 和 `log` 省下的那点代码量，远不如"亲眼看见 `ecall` 怎么发生"值钱。

**用 stable 而不是 nightly。** Rust 1.82 之后 `&raw const`、默认 alloc error handler、
`PanicInfo::message` 等都已稳定，教学 OS 需要的特性在 stable 上够用了，
换来的是工具链不会某天早上自己坏掉。

**每个 `unsafe` 都要有理由。** 代码里的 `unsafe` 块都附了说明：为什么这里必须
绕过类型系统，以及凭什么它是安全的。

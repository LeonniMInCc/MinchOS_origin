# MinchOS - A Microkernel-style OS Project

![Language](https://img.shields.io/badge/Language-Rust-ea7336?style=flat&logo=rust)
![Status](https://img.shields.io/badge/Status-In%20Progress-yellow)
![Topic](https://img.shields.io/badge/Topic-Operating%20System-blue)

**MinchOS** 是为了深耕操作系统底层原理而发起的项目。
本项目最初使用 Go 语言完成了进程管理模拟（Level A 实验2），现在正式全面转向 **Level B** 标准的底层内核开发！
我们采用现代系统级语言 **Rust**，目标是基于 **RISC-V (64-bit)** 架构，从零手搓一个能够在 QEMU 虚拟机上裸机运行的微内核操作系统，并满足 CCNU 操作系统实验自动判题平台的要求。

---

## 🎯 当前开发阶段

### [重制版] Level B - 实验一：启动和系统初始化 (CH01)
**目标**：脱离原有操作系统环境，实现裸机环境搭建、内存地址链接重定位，并实现底层串口打印，通过线上平台自动判题。

- [x] **Day 1: 环境破冰与内核骨架组装**
  - 彻底梳理 Level A 与 Level B 的考核区别，制定出 Rust + RISC-V 的开发路线。
  - 完成了 Rust 交叉编译目标的配置 (`riscv64gc-unknown-none-elf`)。
  - 初始化了 `os` 项目结构。
  - 编写了自定义的内存地图链接脚本 (`linker.ld`)，将内核入口强制定向至 QEMU 虚拟机的物理启动地址 `0x80200000`。
  - 编写了硬件上电的第一段启动汇编代码 (`entry.asm`)，初始化了函数调用栈 (Stack)。
  - 在 `#![no_std]` 和 `#![no_main]` 环境下接管了入口，并编写了内核恐慌 (panic) 处理逻辑。

- [x] **Day 2: 点亮屏幕与内存清零 (当前进度)**
  - 实现基于 OpenSBI `ecall` 的底层串口字符打印函数 (`console.rs`, `sbi.rs`)。
  - 修复了中文 UTF-8 打印时使用 `chars()` 导致截断乱码的问题，改用 `bytes()` 进行字节级原样输出。
  - 在入口函数中清零了 `.bss` 段，并动态计算输出了内核各段 (`.text`, `.rodata`, `.data`, `.bss`, `boot_stack`) 的内存布局。
  - 修复了 Rust 编译器中 `direct cast of function item into an integer` 的安全警告，将函数指针显式转换为 `*const ()` 再转为 `usize`。
  - 完美符合自动判题程序所需的特定中文输出格式要求，并主动关机 (`sbi::shutdown()`)。

---

## 📂 历史存档
- 之前的 Go 语言版 PCB 调度模拟器代码（Level A）依旧保存在根目录下的 `main.go` 中，可作为进程管理逻辑的参考。

## 🚀 自动判题与代码上传原理 (CI/CD)
本次实验采用了现代软件工程中的 **持续集成 (Continuous Integration)** 方式进行自动化判题。以下是“方法一”背后发生的底层流程：

1. **本地开发与版本控制**：我们在本地编写 Rust 源码，并使用 Git 进行版本管理 (`git commit`)。
2. **触发远程 Pipeline**：当我们通过 `git push` 将代码推送到学校的评测 Git 仓库时，GitLab/Gitea 等代码托管平台会通过 Webhook 触发后端的 **CI/CD Runner**。
3. **独立沙盒构建**：评测服务器会在 `/tmp` 目录下生成一个随机的隔离沙盒（如 `/tmp/ccnuos-grade-_e1urzg5/repo`），拉取我们在远端的代码。
4. **编译与静默检查**：服务器执行 `make build` / `cargo build`，检查是否存在语法错误或强制警告（例如之前由于直接把函数转成整数 `as usize` 触发的警告，必须修正为 `as *const () as usize` 才能过审）。
5. **QEMU 自动化运行测试**：判题脚本使用 QEMU 模拟 RISC-V 裸机环境，并传入我们的内核二进制文件 (`os.bin`)，截获内核输出的串口字符流。
6. **正则匹配与打分**：评测机将 QEMU 串口输出的内容与预期的字符串（如 `[INFO ] 内核已进入 S 态，启动栈可用，.bss 已清零`）进行严格匹配，最终得出 `score.json` 并反馈给前端。

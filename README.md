# MinchOS

使用 Rust 在 RISC-V 64 位 QEMU 环境中逐步实现操作系统课程 Level B 实验，长期目标是探索微内核设计。早期 Go 进程管理模拟保留作为学习记录。

## 当前进度

- 2026-11-06：完整归档校园项目。`campus/` 为校园 main 源码快照；所有章节历史见 `campus/ch01`—`campus/ch16` 分支。同步结构及后续约定见[校园项目同步](docs/校园项目同步.md)。

- 完成 CH01 裸机启动、启动栈、BSS 清零、SBI 输出与关机代码。
- 修正链接符号转换警告及中文输出方式。
- 2026-10-11：用户反馈已成功通过提交；本仓库未附最新成绩单，不声明具体分数。
- 完整复盘与逐文件解释：[CH01：首次判题复盘与启动代码说明](docs/CH01-复盘与代码说明.md)。

## 目录

| 路径 | 内容 |
| --- | --- |
| `minchos_kernel/` | Rust 内核；对应学校仓库的 `os/` |
| `main.go`、`go.mod` | 早期 Go PCB 与队列模拟练习 |
| `report.tex` | 早期实验报告模板 |
| `docs/` | 实验日志和代码说明 |

## 编译

安装 Rust 和 `riscv64gc-unknown-none-elf` 目标后，在内核目录执行：

```bash
cd minchos_kernel
cargo build --release
cargo rustc --release -- -D warnings
```

QEMU 运行：安装 GNU Make、QEMU（含 qemu-system-riscv64）、rust-objcopy 与 LLVM 工具后执行 `make run`。

## 学校判题与 GitHub 归档

学校仓库按章节使用 `ch01` 等分支。在对应章节仓库根目录执行其提供的判题命令，例如 `make grade CH=1 FORMAT=json`。学校构建与评分工具现随 `campus/` 和章节分支一同归档，使用方式以对应分支说明为准。

GitLab Web IDE 的提交会更新远端分支；学校在线工作区需要同步该提交后再推送，否则可能出现 non-fast-forward。是否自动启动判题取决于学校平台，应核对判题记录中的提交编号。

GitHub `main` 用于代码与实验过程归档。当前内核尚未实现分页、调度或完整微内核服务。

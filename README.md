# CCNUOS

一个用 Rust 从零编写的 RISC-V 64 (SV39) 教学操作系统内核，运行在 QEMU `virt` 机器上。

目标不只是"能跑"，而是**每一步都讲得清楚**：仓库按周打 tag，每个 tag 配一篇
`docs/chXX.md`，`git diff v0.1..v0.2` 就是那一章的全部实验内容。

> **`v1.0` 已完成**　16 周 · 24 个系统调用 · 25 个用户态程序 · 零外部依赖 ·
> 16 个端到端回归用例全部通过
>
> [实验报告](docs/REPORT.md) · [答辩材料](docs/slides.html) ·
> [运行录像](docs/demo.html) · [运行可视化](docs/trace.html)

## 十六周

| 进度 | 内容 |
|---|---|
| ✅ 第 1 周 (`v0.1`) | 裸机启动、SBI 串口输出、分级日志、内存布局自省 |
| ✅ 第 2 周 (`v0.2`) | 伙伴系统堆分配器，`Box`/`Vec`/`String`/`BTreeMap` 可用 |
| ✅ 第 3 周 (`v0.3`) | 特权级切换、Trap 处理、系统调用、批处理运行 5 个用户程序 |
| ✅ 第 4 周 (`v0.4`) | 多道程序、`__switch` 任务切换、`sys_yield` 协作式轮转 |
| ✅ 第 5 周 (`v0.5`) | 时钟中断、时间片轮转、抢占式调度 —— **阶段 I 完成** |
| ✅ 第 6 周 (`v0.6`) | 物理页帧分配器（RAII 回收）、SV39 三级页表 |
| ✅ 第 7 周 (`v0.7`) | 地址空间抽象、开启分页、从 ELF 装载、内核态 trap 入口 |
| ✅ 第 8 周 (`v0.8`) | 跳板页、每进程独立地址空间、内核栈 guard page —— **阶段 II 核心完成** |
| ✅ 第 9 周 (`v0.9`) | 进程树、fork/exec/waitpid、僵尸回收 —— **阶段 II 完成** |
| ✅ 第 10 周 (`v0.10`) | `File` trait、文件描述符表、`sys_read`、交互式 shell |
| ✅ 第 11 周 (`v0.11`) | 匿名管道（环形缓冲 + EOF 检测）、`sys_pipe`/`sys_dup`、shell 支持 `\|` |
| ✅ 第 12 周 (`v0.12`) | `easy-fs` 独立 crate（宿主机可测）、virtio-blk 驱动、块缓存 |
| ✅ 第 13 周 (`v0.13`) | inode/目录/`open`/`>` 重定向，程序改从磁盘加载 —— **阶段 III 完成** |
| ✅ 第 14 周 (`v0.14`) | 阻塞/唤醒、互斥锁·信号量·条件变量、死锁检测、中断驱动的串口输入 |
| ✅ 第 15 周 (`v0.15`) | 结构化事件输出（默认零开销）、调度甘特图 / 系统调用时序 / 进程树 / 地址空间可视化 |
| ✅ 第 16 周 (`v1.0`) | 端到端回归测试（16 个用例）、实验报告、答辩材料 —— **项目完成** |

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

你会看到 OpenSBI 的 banner、CCNUOS 的启动信息和自检，然后进入 shell 提示符：

```
ccnuos> 
```

输入 `help` 看有哪些程序，输入程序名回车执行，`exit` 退出（系统随即关机）。
支持管道和输出重定向：

```
ccnuos> 12words | 13upper | 14wc
ccnuos> 12words > out.txt
ccnuos> 18ls
```

**完整的运行、调试与排错说明见 [docs/RUNNING.md](docs/RUNNING.md)**，包括每一段输出的含义、卡住了怎么退出、以及 OS 开发都有哪些工具。

其他命令：

```bash
make test           # 文件系统单元测试（宿主机，不需要 QEMU，0.01 秒）
make regress        # 端到端回归测试：23 个演示程序全跑一遍（约 2 分钟）
make check          # 快速类型检查
make asm            # 反汇编内核，看生成了什么代码
make demo           # 录一次启动，生成可在浏览器回放的单文件 HTML
make trace          # 录一次带事件输出的运行，生成四张可视化图
make debug          # 挂起等待 GDB（另开终端跑 make gdb）
LOG=DEBUG make run  # 调高日志级别（ERROR/WARN/INFO/DEBUG/TRACE）
make clean
```

`make demo` 生成的 [docs/demo.html](docs/demo.html) 把启动过程连同颜色逐字回放出来，
带播放/暂停/调速，不依赖任何外部资源 —— 发给没装 QEMU 的人也能看。

`make trace` 生成的 [docs/trace.html](docs/trace.html) 是另一回事：它让内核输出结构化事件，
在宿主机上渲染成**调度甘特图**、**系统调用时序**、**进程树**和**地址空间对照**四张图。
事件输出默认关闭且零开销（关闭时二进制里连事件的记号都不存在），
因为打开之后光是打印事件就要吃掉六成 CPU 时间 —— 详见 [docs/ch15.md](docs/ch15.md)。

## 目录结构

```
ccnuos/
├── Makefile              构建与运行入口
├── rust-toolchain.toml   钉死工具链版本与 target，保证可复现
├── ROADMAP.md            16 周计划
├── docs/                 16 章讲解文档 + 实验报告 + 答辩材料
└── os/                   内核 crate
    ├── .cargo/config.toml  交叉编译目标 + 链接脚本
    ├── build.rs            让 LOG / TRACE 环境变量能触发重新编译
    └── src/
        ├── linker.ld       内存布局，把内核钉在 0x80200000
        ├── entry.asm       入口：支起启动栈，跳进 Rust
        ├── main.rs         rust_main：清 .bss、初始化、自检、关机
        ├── config.rs       内核可调参数
        ├── sbi.rs          SBI 调用（ecall）封装
        ├── console.rs      print! / println!
        ├── logging.rs      分级彩色日志
        ├── lang_items.rs   panic handler
        ├── csr.rs          RISC-V 控制状态寄存器访问
        ├── timer.rs        时钟：time CSR + SBI set_timer，10 ms 时间片
        ├── trace.rs        结构化事件输出（编译期开关，关闭时零开销）
        ├── drivers/
        │   ├── virtio_blk.rs  第一个真正的设备驱动（virtqueue + 轮询）
        │   ├── plic.rs        平台级中断控制器
        │   └── uart.rs        中断驱动的串口输入 + 环形缓冲
        ├── sync/
        │   ├── up.rs          UPSafeCell
        │   ├── wait_queue.rs  等待队列（三种同步原语共同的底座）
        │   ├── mutex.rs       阻塞式互斥锁
        │   ├── semaphore.rs   计数信号量
        │   └── condvar.rs     条件变量
        ├── fs/
        │   ├── mod.rs      File trait（三个实现共用同一组动词）
        │   ├── stdio.rs    标准输入/输出
        │   ├── pipe.rs     匿名管道（环形缓冲区）
        │   └── inode.rs    磁盘文件
        ├── mm/
        │   ├── heap_allocator.rs   伙伴系统分配器 + #[global_allocator]
        │   ├── address.rs          四种地址/页号的 newtype 封装
        │   ├── frame_allocator.rs  物理页帧分配器 + FrameTracker (RAII)
        │   ├── page_table.rs       SV39 三级页表
        │   ├── memory_set.rs       地址空间：逻辑段 + 页表 + 跳板
        │   └── elf.rs              最小 ELF64 解析器
        ├── trap/
        │   ├── trap.S      跳板页：__alltraps / __restore（+ __kernel_trap）
        │   └── context.rs  TrapContext（34 个寄存器，硬件随时打断）
        ├── task/
        │   ├── switch.S    手写的 __switch
        │   ├── context.rs  TaskContext（14 个寄存器，借力调用约定）
        │   ├── pid.rs      PID 分配器 + 内核栈（都是 RAII）
        │   ├── manager.rs  就绪队列
        │   ├── processor.rs 当前进程 + idle 调度循环
        │   └── tcb.rs      进程控制块（含进程树）
        └── syscall/        系统调用分派

user/                     用户程序（独立 crate）
├── src/lib.rs            最小运行库：_start、clear_bss、println!
└── src/bin/              initproc + user_shell + 23 个用户程序
                          （全部链接到同一个虚拟地址 0x10000）


easy-fs/                  文件系统（独立 crate，不依赖内核）
├── src/block_dev.rs      BlockDevice trait —— 与外界的唯一接口
├── src/block_cache.rs    块缓存（LRU + 写回）
├── src/bitmap.rs         位图分配器
├── src/layout.rs         超级块、inode、目录项
├── src/efs.rs            文件系统本体（分配 inode 与数据块）
├── src/vfs.rs            inode 句柄：find / create / ls / 读写
└── src/tests.rs          18 个单元测试，宿主机上 `make test` 秒级跑完

easy-fs-fuse/             在宿主机上制作镜像，与内核共用 easy-fs

tools/                    宿主机侧工具，不进内核镜像
├── capture.sh            录一次 QEMU 运行
├── ansi2html.py          串口输出 → 可回放的 HTML（make demo）
├── trace2html.py         结构化事件 → 四张可视化图（make trace）
└── regress.py            端到端回归测试（make regress）
```

## 设计取舍

**不依赖任何外部 crate。** SBI 封装、日志、控制台都是手写的。对教学项目而言，
`sbi-rt` 和 `log` 省下的那点代码量，远不如"亲眼看见 `ecall` 怎么发生"值钱。

**用 stable 而不是 nightly。** Rust 1.82 之后 `&raw const`、默认 alloc error handler、
`PanicInfo::message` 等都已稳定，教学 OS 需要的特性在 stable 上够用了，
换来的是工具链不会某天早上自己坏掉。

**每个 `unsafe` 都要有理由。** 全部 51 个 `unsafe` 块都附了 `SAFETY` 说明：
为什么这里必须绕过类型系统，以及凭什么它是安全的。这一条是**核对过的**，
不是声称的 —— 第 16 周发现有 11 处没写，补齐了。

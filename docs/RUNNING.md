# 运行指南

怎么把 CCNUOS 跑起来、怎么看效果、卡住了怎么办。

---

## 0. 先说安全

CCNUOS **不会碰你的 Mac**。它跑在 QEMU 里，而 QEMU 就是一个普通的用户态程序，
和 Chrome 没有本质区别 —— 它在自己的进程内存里模拟出一台虚拟的 RISC-V 电脑，
CCNUOS 以为自己在操作"硬件"，实际操作的全是 QEMU 分配的一块普通内存。

这个项目**没有也不会**：

- 碰你的启动盘、EFI 分区或引导程序（macOS 的启动流程完全不知道 CCNUOS 存在）
- 装任何内核扩展或系统级驱动
- 写 QEMU 进程之外的任何文件
- 要求 `sudo`

而且 Mac 是 ARM64，CCNUOS 编译出来的是 RISC-V 指令 —— 你的 CPU 根本不认识这些字节。

内核崩了会怎样？**QEMU 进程退出，仅此而已。**

---

## 1. 一次性准备

只需要做一次。

```bash
brew install rustup qemu
rustup default stable
rustup target add riscv64gc-unknown-none-elf
rustup component add rust-src llvm-tools
```

验证：

```bash
qemu-system-riscv64 --version
```

> **注意**：Homebrew 装的 rustup 是 keg-only，不在默认 PATH 里。
> `make` 已经自动处理了，但如果你想直接敲 `cargo`，需要加一行：
>
> ```bash
> echo 'export PATH="/opt/homebrew/opt/rustup/bin:$PATH"' >> ~/.zshrc
> ```

---

## 2. 跑起来

```bash
cd ~/workspace/vibe/ccnuos
make run
```

就这一条。它会编译内核 → 剥成裸二进制 → 拉起 QEMU。

---

## 3. 你会看到什么

输出分三段，每一段都有意义。

### 第一段：OpenSBI 在自我介绍

```
OpenSBI v1.8.1
   ____                    _____ ____ _____
  / __ \                  / ____|  _ \_   _|
 ...
Firmware Base               : 0x80000000
Runtime SBI Version         : 3.0
Boot HART Base ISA          : rv64imafdch
```

**这还不是你的内核。** OpenSBI 是跑在 M 态的固件，地位相当于 x86 世界的 BIOS。
它初始化完硬件后，会跳转到 `0x80200000` —— 那里才是 CCNUOS。

值得留意 `Domain0 Next Address` 这一行：它是固件即将跳转的地址。
如果这里是 `0x0`，说明固件不知道往哪跳，内核一个字都不会输出（见 [ch01.md](ch01.md) 坑 2）。

### 第二段：CCNUOS 接管

```
   ____   ____  _   _  _   _   ___   ____
  / ___| / ___|| \ | || | | | / _ \ / ___|
 | |    | |    |  \| || | | || | | |\___ \
 | |___ | |___ | |\  || |_| || |_| | ___) |
  \____| \____||_| \_| \___/  \___/ |____/

       CCNUOS v0.2.0 — RISC-V 64 (SV39) 教学操作系统
       第二周：内核堆 / 伙伴系统分配器
```

能看到这个 banner，就说明 `entry.asm` 支起了栈、`rust_main` 跑起来了、
SBI 串口输出通了。**第 1 周的全部工作都浓缩在这几行字能否出现上。**

### 第三段：自检

```
[INFO ] 内核已进入 S 态，启动栈可用，.bss 已清零
[INFO ] 内核内存布局：
[INFO ]   .text            [0x0080200000, 0x0080205000)    20480 B  (  20 KiB)
[INFO ]   .rodata          [0x0080205000, 0x0080207000)     8192 B  (   8 KiB)
[INFO ]   .data            [0x0080207000, 0x0080207000)        0 B  (   0 KiB)
[INFO ]   .bss             [0x0080217000, 0x0080518000)  3149824 B  (3076 KiB)
[INFO ]   boot_stack       [0x0080207000, 0x0080217000)    65536 B  (  64 KiB)
[INFO ]   kernel total     [0x0080200000, 0x0080518000)  共 3168 KiB
[INFO ] 内核堆就绪：[0x0080217000, 0x0080517000)  3072 KiB
[INFO ] 堆自检通过：容量 3072 KiB，峰值占用 6680 B，释放后回落至 0 B
[INFO ] 第二周目标达成，主动关机
```

这张内存布局表会跟着项目一路变化，是观察内核长大的最直接窗口：

- `.bss` 有 3 MiB，因为内核堆就是一个静态数组
- `kernel total` 的终点（`0x80518000`）就是 `ekernel` ——
  **第 6 周做页帧分配器时，可分配的物理内存从这里开始**
- `释放后回落至 0 B` 是堆自检的核心断言，它能抓住伙伴合并逻辑的错误

跑完内核自己调 SBI 关机，QEMU 正常退出，你回到 shell。

---

## 4. 怎么退出 / 卡住了怎么办

正常情况下内核会自己关机，不需要你做任何事。

**如果卡住了**（现在不会，但第 3 周手写 trap 汇编时很可能）：

| 方法 | 操作 |
|---|---|
| QEMU 快捷键 | 按 `Ctrl-A`，**松开**，再按 `X` |
| 另开终端杀进程 | `pkill qemu-system-riscv64` |
| 最粗暴 | 直接关掉终端窗口 |

三种都没有任何副作用。QEMU 只是个普通进程。

> `Ctrl-A` 是 QEMU 的转义前缀，不是组合技。按下 `Ctrl-A` 后松手，
> 再单独按 `X` 才生效。按 `Ctrl-A` 再按 `C` 可以进 QEMU monitor
> （能查看寄存器、内存，调试时有用，`q` 退出）。

---

## 5. 看更多细节

日志分五级，默认只显示 `INFO` 及以上：

```bash
LOG=DEBUG make run     # 多出分配器的内部细节
LOG=TRACE make run     # 全开
LOG=WARN  make run     # 只看警告和错误
```

级别是**编译期常量**，被过滤掉的日志会被 LLVM 整段消除 —— 所以运行时零开销，
可以放心在热路径（比如第 5 周的任务切换）上撒日志。改了 `LOG` 会自动触发重新编译。

---

## 6. 命令速查

| 命令 | 作用 |
|---|---|
| `make run` | 编译并运行 |
| `make check` | 只做类型检查，比完整编译快 |
| `make clippy` | 静态检查，抓常见错误 |
| `make asm` | 反汇编内核，看实际生成了什么指令 |
| `make demo` | 录一次启动，生成可在浏览器里回放的 HTML |
| `make debug` | 挂起等待 GDB 连接 |
| `make gdb` | 连上去（需另开终端） |
| `make clean` | 清理构建产物 |

---

## 7. 在浏览器里看：`make demo`

```bash
make demo
open docs/demo.html
```

会生成一个**单文件 HTML**，把这次启动的串口输出（连同颜色）逐字回放出来，
带播放/暂停/重播/调速。不依赖任何外部资源，可以直接发给别人、
或者放进答辩材料里 —— 对方不需要装 QEMU。

想录带 DEBUG 细节的版本：

```bash
LOG=DEBUG make demo
```

实现在 [`tools/capture.sh`](../tools/capture.sh)（抓日志）和
[`tools/ansi2html.py`](../tools/ansi2html.py)（ANSI 转义 → 带 class 的 span + JS 回放）。

> 这是第 15 周"教学可视化系统"的雏形。到那一周会升级成：内核输出结构化事件，
> 宿主机渲染成进程调度甘特图、页表树、系统调用时序图。

---

## 8. 有没有"专门的工具"

有，而且你已经在用最主流的那个了。

### QEMU —— 我们用的，也是 OS 开发的事实标准

全系统模拟器。模拟整台机器：CPU、内存、串口、磁盘、中断控制器。
Linux 内核开发、rCore/xv6 教学、几乎所有 OS 课程都用它。

优点是外设齐全（第 12 周要用的 virtio-blk 块设备就靠它）、
内置 GDB 服务器、跨架构。**这个项目从头到尾不需要换工具。**

### GDB —— 调试内核的唯一严肃手段

QEMU 内置了 GDB 服务器。`make debug` 会让 QEMU 挂在第一条指令上等你连接：

```bash
# 终端 1
make debug

# 终端 2
brew install riscv64-elf-gdb     # 一次性
make gdb
```

连上后可以单步、下断点、看寄存器（`info registers`）、看内存（`x/16x $sp`）。

**第 3 周开始这个是必需品**，不是可选项。手写 trap 汇编时寄存器保存顺序错了
不会报错，只会跑飞 —— 靠 `println!` 是猜不出来的。

### 其他你可能听说的

| 工具 | 是什么 | 我们用不上的原因 |
|---|---|---|
| **Spike** | RISC-V 官方 ISA 模拟器 | 更"标准"，但外设支持少，做不了第 12 周的块设备 |
| **Renode** | 嵌入式系统仿真器 | 强在多节点/网络仿真，对单机 OS 是杀鸡用牛刀 |
| **真实硬件**（VisionFive 2 等） | 真的 RISC-V 开发板 | 调试要接串口线和 JTAG，迭代一次几分钟。等内核稳定了再考虑 |
| **asciinema** | 终端录屏工具 | 想录成可分享的终端动画可以用，但 `make demo` 已经覆盖了这个需求 |

---

## 9. 出问题了

| 现象 | 原因 | 怎么办 |
|---|---|---|
| `command not found: cargo` | Homebrew 的 rustup 是 keg-only | 见第 1 节的 PATH 配置。`make` 不受影响 |
| `invalid register 'a0'` | 在按宿主机架构编译，没读到 `os/.cargo/config.toml` | 用 `make build`，别直接 `cargo build --manifest-path` |
| OpenSBI banner 打完就没反应 | 固件不知道往哪跳 | 检查 `Domain0 Next Address` 是不是 `0x0`；Makefile 里必须用 `-kernel` 而不是 `-device loader` |
| 中文变成乱码 | 串口按字节输出被截断 | 见 [ch01.md](ch01.md) 坑 3 |
| `找不到 llvm-objcopy` | 缺 llvm-tools 组件 | `rustup component add llvm-tools` |
| QEMU 起来但一片空白 | 内核在早期就跑飞了 | `make debug` + `make gdb`，在 `0x80200000` 下断点单步 |

---

## 10. 一句话总结

```bash
cd ~/workspace/vibe/ccnuos && make run
```

看到 CCNUOS 的 banner 和最后那行 `第二周目标达成，主动关机`，就说明一切正常。

# MinchOS - A Microkernel-style OS Project

![Language](https://img.shields.io/badge/Language-Go-00ADD8?style=flat&logo=go)
![Status](https://img.shields.io/badge/Status-Completed-brightgreen)
![Topic](https://img.shields.io/badge/Topic-Operating%20System-blue)

**MinchOS** 是为了深耕操作系统底层原理而发起的项目。本项目旨在从零构建一个基于微内核（Microkernel）理念的操作系统模拟模型。开发语言选用 **Golang**，利用其原生的高并发优势来模拟内核与进程间的复杂调度。

## 🎯 最终目标
作为一个操作系统课程的最终结课项目，MinchOS 成功实现了：
- 高效的内存管理分配器（SLUB 算法等）
- 细粒度的进程调度与状态机管理
- 基于进程原语的上下文切换与并发控制
- 类似微内核架构的模块化交互设计

---

## 🛠️ 实验日志与开发进度

### [阶段一] 进程调度与内存管理 (已完结)
**目标**：构建 OS 的进程控制块（PCB）管理与底层内存池。

- [x] **Day 1: OS 数据结构基石**
  - 使用 Go 语言完成了项目初始化。
  - 弃用传统的伙伴系统，改用更贴合固定大小对象分配的 **SLUB 分配器** 策略。
  - 定义了操作系统的核心数据结构：进程状态机 (ProcessState)、进程控制块 (PCB)、以及内存分配池 (SlubPool)。

- [x] **Day 2: SLUB 分配器核心算法实现**
  - 完成了内核态初始化函数 Init()，使用单向链表高效组织物理连续内存池。
  - 实现了 O(1) 时间复杂度的 AllocPCB()（分配）与经典的头插法 FreePCB()（回收）。
  - 实现内存池快照打印功能，满足实验报告要求的三个状态：初始快照、进程运行前快照、进程运行后快照。

- [x] **Day 3: 进程调度器与多队列管理**
  - 实现了底层的通用先进先出 (FIFO) 队列管理器。
  - 实现了完整的调度器核心原语：CreateProcess（创建）、Schedule（调度，引入时间片轮转机制）、BlockProcess（阻塞）、WakeupProcess（唤醒）和 DestroyProcess（撤销）。
  - 实现了全局状态快照 PrintSystemSnapshot()，完美展示进程在 CPU、就绪队列和阻塞队列之间的动态流转过程。

- [x] **Day 4 & 5: 终端交互菜单与项目交付**
  - 实现了基于终端的命令行交互菜单 (ufio.Scanner)，允许用户手动键入指令模拟进程流转，完美满足了“菜单式管理”的实验要求。
  - 提供并整理了专业的 LaTeX 实验报告模板 (
eport.tex)。
  - 项目收尾，提交最终代码，完成操作系统 Level A 难度实验验证。

---

## 💻 快速运行

`ash
go run main.go
`

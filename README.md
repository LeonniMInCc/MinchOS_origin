# MinchOS - A Microkernel-style OS Project

![Language](https://img.shields.io/badge/Language-Go-00ADD8?style=flat&logo=go)
![Status](https://img.shields.io/badge/Status-In%20Progress-brightgreen)
![Topic](https://img.shields.io/badge/Topic-Operating%20System-blue)

**MinchOS** 是为了深耕操作系统底层原理而发起的项目。本项目旨在从零构建一个基于微内核（Microkernel）理念的操作系统模拟模型。开发语言选用 **Golang**，利用其原生的高并发优势来模拟内核与进程间的复杂调度。

## 🎯 最终目标
作为一个操作系统课程的最终结课项目，MinchOS 的目标是实现：
- 高效的内存管理分配器（SLUB 算法等）
- 细粒度的进程调度与状态机管理
- 基于进程原语的上下文切换与并发控制
- 类似微内核架构的模块化设计

---

## 🛠️ 实验日志与开发进度

### [阶段一] 进程调度与内存管理 (当前进度)
**目标**：构建 OS 的进程控制块（PCB）管理与底层内存池。

- [x] **Day 1: OS 数据结构基石**
  - 使用 Go 语言完成了项目初始化。
  - 弃用传统的伙伴系统，改用更贴合固定大小对象分配的 **SLUB 分配器** 策略。
  - 定义了操作系统的核心数据结构：进程状态机 (\ProcessState\)、进程控制块 (\PCB\)、以及内存分配池 (\SlubPool\)。

- [x] **Day 2: SLUB 分配器核心算法实现**
  - 完成了内核态初始化函数 \Init()\，使用单向链表高效组织物理连续内存池。
  - 实现了 \O(1)\ 时间复杂度的 \AllocPCB()\（分配）与经典的头插法 \FreePCB()\（回收）。
  - 实现内存池快照打印功能，满足实验报告要求的三个状态：初始快照、进程运行前快照、进程运行后快照。

- [ ] **Day 3: 进程调度器与多队列管理**
  - 待实现进程原语（创建 Create、撤销 Destroy、阻塞 Block、唤醒 Wakeup）。
  - 待实现就绪队列 (ReadyQueue) 和等待队列 (WaitQueue) 的管理机制。

---

## 💻 快速运行

\\ash
go run main.go
\
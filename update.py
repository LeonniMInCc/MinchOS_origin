import os

main_code = '''package main

import "fmt"

// ==========================================
// 1. 基础数据结构 (Day 1 & 2)
// ==========================================

type ProcessState int

const (
	StateReady ProcessState = iota
	StateRunning
	StateWaiting
	StateFinished
)

type PCB struct {
	PID          int
	State        ProcessState
	Next         *PCB
}

type SlubPool struct {
	PoolSize int
	FreeList *PCB
	Memory   []PCB
}

func (pool *SlubPool) Init(size int) {
	pool.PoolSize = size
	pool.Memory = make([]PCB, size)
	for i := 0; i < size-1; i++ {
		pool.Memory[i].Next = &pool.Memory[i+1]
	}
	pool.Memory[size-1].Next = nil
	pool.FreeList = &pool.Memory[0]
}

func (pool *SlubPool) AllocPCB() *PCB {
	if pool.FreeList == nil {
		return nil
	}
	allocated := pool.FreeList
	pool.FreeList = pool.FreeList.Next
	allocated.Next = nil
	return allocated
}

func (pool *SlubPool) FreePCB(pcb *PCB) {
	pcb.PID = 0
	pcb.State = StateFinished
	pcb.Next = pool.FreeList
	pool.FreeList = pcb
}

// ==========================================
// 2. 队列管理器 (Day 3 新增)
// ==========================================

// Queue: 通用的 FIFO (先进先出) 队列
type Queue struct {
	Head *PCB
	Tail *PCB
}

// 入队 (加到队尾)
func (q *Queue) Enqueue(pcb *PCB) {
	pcb.Next = nil
	if q.Head == nil { // 队列为空
		q.Head = pcb
		q.Tail = pcb
	} else { // 队列不为空
		q.Tail.Next = pcb
		q.Tail = pcb
	}
}

// 出队 (从队头取出一个)
func (q *Queue) Dequeue() *PCB {
	if q.Head == nil {
		return nil
	}
	pcb := q.Head
	q.Head = q.Head.Next
	if q.Head == nil { // 如果取走后队列空了，要把 Tail 也置空
		q.Tail = nil
	}
	pcb.Next = nil
	return pcb
}

// ==========================================
// 3. 调度器与进程原语 (Day 3 核心)
// ==========================================

type Scheduler struct {
	ReadyQueue Queue     // 就绪队列
	WaitQueue  Queue     // 等待(阻塞)队列
	RunningPCB *PCB      // 当前正在 CPU 运行的进程
	MemPool    *SlubPool // 掌握内存分配大权
}

// 创建原语 (Create)
func (s *Scheduler) CreateProcess(pid int) {
	fmt.Printf("[原语] 正在创建进程 PID: %d...\n", pid)
	pcb := s.MemPool.AllocPCB()
	if pcb == nil {
		fmt.Println("  -> [失败] 内存不足，无法创建进程！")
		return
	}
	pcb.PID = pid
	pcb.State = StateReady
	s.ReadyQueue.Enqueue(pcb) // 进入就绪队列
	fmt.Println("  -> [成功] 进程已创建，当前处于就绪队列中。")
}

// 调度原语 (Schedule)
func (s *Scheduler) Schedule() {
	if s.RunningPCB != nil {
		fmt.Println("[调度] CPU 正在忙碌，暂时无需调度。")
		return
	}
	pcb := s.ReadyQueue.Dequeue()
	if pcb == nil {
		fmt.Println("[调度] 就绪队列为空，CPU 进入空闲(Idle)状态。")
		return
	}
	pcb.State = StateRunning
	s.RunningPCB = pcb
	fmt.Printf("[调度] 进程 PID: %d 被调度上 CPU 运行。\n", pcb.PID)
}

// 阻塞原语 (Block)
func (s *Scheduler) BlockProcess() {
	if s.RunningPCB == nil {
		fmt.Println("[错误] 当前没有进程在运行，无法阻塞！")
		return
	}
	pcb := s.RunningPCB
	fmt.Printf("[原语] 进程 PID: %d 遇到 I/O 等待，正在阻塞...\n", pcb.PID)
	
	pcb.State = StateWaiting
	s.WaitQueue.Enqueue(pcb) // 进入阻塞队列
	s.RunningPCB = nil       // CPU 腾出来了
}

// 唤醒原语 (Wakeup)
func (s *Scheduler) WakeupProcess() {
	pcb := s.WaitQueue.Dequeue()
	if pcb == nil {
		fmt.Println("[错误] 等待队列为空，没有进程可以唤醒！")
		return
	}
	fmt.Printf("[原语] 资源已就绪，正在唤醒进程 PID: %d...\n", pcb.PID)
	
	pcb.State = StateReady
	s.ReadyQueue.Enqueue(pcb) // 丢回就绪队列去重新排队
}

// 撤销原语 (Destroy)
func (s *Scheduler) DestroyProcess() {
	if s.RunningPCB == nil {
		fmt.Println("[错误] 当前没有进程在运行，无法撤销！")
		return
	}
	pcb := s.RunningPCB
	fmt.Printf("[原语] 进程 PID: %d 运行结束，正在撤销...\n", pcb.PID)
	
	s.MemPool.FreePCB(pcb) // 归还内存
	s.RunningPCB = nil     // CPU 腾出
}

// 打印全局快照
func (s *Scheduler) PrintSystemSnapshot() {
	fmt.Println("\n==========【操作系统全局快照】==========")
	// 打印 CPU
	if s.RunningPCB != nil {
		fmt.Printf(" [CPU 当前运行] : PID = %d\n", s.RunningPCB.PID)
	} else {
		fmt.Println(" [CPU 当前运行] : IDLE (空闲)")
	}

	// 打印就绪队列
	fmt.Print(" [就绪队列]     : ")
	for curr := s.ReadyQueue.Head; curr != nil; curr = curr.Next {
		fmt.Printf("[PID:%d] -> ", curr.PID)
	}
	fmt.Println("nil")

	// 打印阻塞队列
	fmt.Print(" [阻塞队列]     : ")
	for curr := s.WaitQueue.Head; curr != nil; curr = curr.Next {
		fmt.Printf("[PID:%d] -> ", curr.PID)
	}
	fmt.Println("nil")
	fmt.Println("========================================")
}

// ==========================================
// 4. 主函数测试
// ==========================================
func main() {
	// 1. 初始化系统
	myPool := SlubPool{}
	myPool.Init(5) // 造 5 个 PCB 的内存池

	sysScheduler := Scheduler{
		MemPool: &myPool,
	}

	// 2. 模拟进程生命周期操作
	fmt.Println("\n--- 第一步：创建两个进程 ---")
	sysScheduler.CreateProcess(1001)
	sysScheduler.CreateProcess(1002)
	sysScheduler.PrintSystemSnapshot()

	fmt.Println("\n--- 第二步：OS 进行调度 ---")
	sysScheduler.Schedule() // 1001 会上 CPU
	sysScheduler.PrintSystemSnapshot()

	fmt.Println("\n--- 第三步：1001 申请磁盘读取，被阻塞 ---")
	sysScheduler.BlockProcess()
	sysScheduler.PrintSystemSnapshot()

	fmt.Println("\n--- 第四步：OS 再次调度 ---")
	sysScheduler.Schedule() // 1002 会上 CPU
	sysScheduler.PrintSystemSnapshot()

	fmt.Println("\n--- 第五步：磁盘读取完毕，唤醒 1001 ---")
	sysScheduler.WakeupProcess()
	sysScheduler.PrintSystemSnapshot()

	fmt.Println("\n--- 第六步：1002 运行结束，撤销进程 ---")
	sysScheduler.DestroyProcess()
	sysScheduler.PrintSystemSnapshot()
}
'''

readme_code = '''# MinchOS - A Microkernel-style OS Project

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
  - 定义了操作系统的核心数据结构：进程状态机 (ProcessState)、进程控制块 (PCB)、以及内存分配池 (SlubPool)。

- [x] **Day 2: SLUB 分配器核心算法实现**
  - 完成了内核态初始化函数 Init()，使用单向链表高效组织物理连续内存池。
  - 实现了 O(1) 时间复杂度的 AllocPCB()（分配）与经典的头插法 FreePCB()（回收）。
  - 实现内存池快照打印功能，满足实验报告要求的三个状态：初始快照、进程运行前快照、进程运行后快照。

- [x] **Day 3: 进程调度器与多队列管理**
  - 实现了底层的通用先进先出 (FIFO) 队列管理器。
  - 实现了完整的调度器核心原语：CreateProcess（创建）、Schedule（调度）、BlockProcess（阻塞）、WakeupProcess（唤醒）和 DestroyProcess（撤销）。
  - 实现了全局状态快照 PrintSystemSnapshot()，完美展示进程在 CPU、就绪队列和阻塞队列之间的动态流转过程。

- [ ] **Day 4: 终端交互菜单与项目交付**
  - 待实现一个基于终端的菜单循环，允许用户手动键入指令模拟进程流转，以满足“菜单式管理”的实验要求。
  - 整理 LaTeX 报告模板，输出最终交付的 PDF 文档。

---

## 💻 快速运行

`ash
go run main.go
`
'''

with open('main.go', 'w', encoding='utf-8') as f:
    f.write(main_code)
with open('README.md', 'w', encoding='utf-8') as f:
    f.write(readme_code)

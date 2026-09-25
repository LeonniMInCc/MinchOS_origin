package main

import (
	"bufio"
	"fmt"
	"os"
	"strings"
)

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

func (pool *SlubPool) PrintSnapshot() {
	freeCount := 0
	curr := pool.FreeList
	for curr != nil {
		freeCount++
		curr = curr.Next
	}
	usedCount := pool.PoolSize - freeCount
	fmt.Println("========================================")
	fmt.Printf("【SLUB 内存池快照】 总容量: %d | 已分配: %d | 剩余空闲: %d
", pool.PoolSize, usedCount, freeCount)
	fmt.Println("========================================")
}

// ==========================================
// 2. 队列管理器 (Day 3)
// ==========================================

type Queue struct {
	Head *PCB
	Tail *PCB
}

func (q *Queue) Enqueue(pcb *PCB) {
	pcb.Next = nil
	if q.Head == nil {
		q.Head = pcb
		q.Tail = pcb
	} else {
		q.Tail.Next = pcb
		q.Tail = pcb
	}
}

func (q *Queue) Dequeue() *PCB {
	if q.Head == nil {
		return nil
	}
	pcb := q.Head
	q.Head = q.Head.Next
	if q.Head == nil {
		q.Tail = nil
	}
	pcb.Next = nil
	return pcb
}

// ==========================================
// 3. 调度器与进程原语 (Day 3)
// ==========================================

type Scheduler struct {
	ReadyQueue Queue
	WaitQueue  Queue
	RunningPCB *PCB
	MemPool    *SlubPool
}

func (s *Scheduler) CreateProcess(pid int) {
	fmt.Printf("[原语] 正在创建进程 PID: %d...
", pid)
	pcb := s.MemPool.AllocPCB()
	if pcb == nil {
		fmt.Println("  -> [失败] 内存不足，无法创建进程！请先撤销某些进程以释放内存。")
		return
	}
	pcb.PID = pid
	pcb.State = StateReady
	s.ReadyQueue.Enqueue(pcb)
	fmt.Println("  -> [成功] 进程已创建，当前处于就绪队列中。")
}

func (s *Scheduler) Schedule() {
	if s.RunningPCB != nil {
		fmt.Printf("[调度] 当前 PID: %d 正在运行。触发时间片轮转...
", s.RunningPCB.PID)
		// 将当前运行的进程放回就绪队列尾部
		s.RunningPCB.State = StateReady
		s.ReadyQueue.Enqueue(s.RunningPCB)
		s.RunningPCB = nil
	}
	
	pcb := s.ReadyQueue.Dequeue()
	if pcb == nil {
		fmt.Println("[调度] 就绪队列为空，CPU 进入空闲(Idle)状态。")
		return
	}
	pcb.State = StateRunning
	s.RunningPCB = pcb
	fmt.Printf("[调度] 进程 PID: %d 被调度上 CPU 运行。
", pcb.PID)
}

func (s *Scheduler) BlockProcess() {
	if s.RunningPCB == nil {
		fmt.Println("[错误] 当前没有进程在运行，无法阻塞！")
		return
	}
	pcb := s.RunningPCB
	fmt.Printf("[原语] 进程 PID: %d 遇到 I/O 等待，正在阻塞...
", pcb.PID)
	
	pcb.State = StateWaiting
	s.WaitQueue.Enqueue(pcb)
	s.RunningPCB = nil
}

func (s *Scheduler) WakeupProcess() {
	pcb := s.WaitQueue.Dequeue()
	if pcb == nil {
		fmt.Println("[错误] 等待队列为空，没有进程可以唤醒！")
		return
	}
	fmt.Printf("[原语] 资源已就绪，正在唤醒进程 PID: %d...
", pcb.PID)
	
	pcb.State = StateReady
	s.ReadyQueue.Enqueue(pcb)
}

func (s *Scheduler) DestroyProcess() {
	if s.RunningPCB == nil {
		fmt.Println("[错误] 当前没有进程在运行，无法撤销！")
		return
	}
	pcb := s.RunningPCB
	fmt.Printf("[原语] 进程 PID: %d 运行结束，正在撤销...
", pcb.PID)
	
	s.MemPool.FreePCB(pcb)
	s.RunningPCB = nil
}

func (s *Scheduler) PrintSystemSnapshot() {
	fmt.Println("
==========【操作系统全局快照】==========")
	if s.RunningPCB != nil {
		fmt.Printf(" [CPU 当前运行] : PID = %d
", s.RunningPCB.PID)
	} else {
		fmt.Println(" [CPU 当前运行] : IDLE (空闲)")
	}

	fmt.Print(" [就绪队列]     : ")
	for curr := s.ReadyQueue.Head; curr != nil; curr = curr.Next {
		fmt.Printf("[PID:%d] -> ", curr.PID)
	}
	fmt.Println("nil")

	fmt.Print(" [阻塞队列]     : ")
	for curr := s.WaitQueue.Head; curr != nil; curr = curr.Next {
		fmt.Printf("[PID:%d] -> ", curr.PID)
	}
	fmt.Println("nil")
	fmt.Println("========================================")
}

// ==========================================
// 4. 终端交互菜单 (Day 4 核心)
// ==========================================
func main() {
	// 初始化系统，内存池大小设为 10
	myPool := SlubPool{}
	myPool.Init(10)

	sysScheduler := Scheduler{
		MemPool: &myPool,
	}

	scanner := bufio.NewScanner(os.Stdin)
	pidCounter := 1000

	fmt.Println("==================================================")
	fmt.Println("    🚀 欢迎使用 MinchOS 微内核进程调度模拟器 🚀    ")
	fmt.Println("==================================================")

	for {
		fmt.Println("
------------------- 菜单 -------------------")
		fmt.Println("  1. 创建新进程 (Create)")
		fmt.Println("  2. 调度执行 / 时间片轮转 (Schedule)")
		fmt.Println("  3. 阻塞当前运行的进程 (Block)")
		fmt.Println("  4. 唤醒等待队列队头进程 (Wakeup)")
		fmt.Println("  5. 撤销当前运行的进程 (Destroy)")
		fmt.Println("  6. 查看系统全局快照 (OS Snapshot)")
		fmt.Println("  7. 查看底层 SLUB 内存池快照 (Mem Snapshot)")
		fmt.Println("  0. 退出系统 (Exit)")
		fmt.Print("请输入指令编号 [0-7]: ")

		if !scanner.Scan() {
			break
		}
		input := strings.TrimSpace(scanner.Text())
		fmt.Println()

		switch input {
		case "1":
			pidCounter++
			sysScheduler.CreateProcess(pidCounter)
		case "2":
			sysScheduler.Schedule()
		case "3":
			sysScheduler.BlockProcess()
		case "4":
			sysScheduler.WakeupProcess()
		case "5":
			sysScheduler.DestroyProcess()
		case "6":
			sysScheduler.PrintSystemSnapshot()
		case "7":
			sysScheduler.MemPool.PrintSnapshot()
		case "0":
			fmt.Println("系统正在关闭... 感谢使用 MinchOS！")
			return
		default:
			fmt.Println("无效的指令，请输入 0 到 7 之间的数字。")
		}
	}
}

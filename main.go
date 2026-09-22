package main

import "fmt"

// ==========================================
// 1. 数据结构定义 (延续上一步)
// ==========================================

type ProcessState int

const (
	StateReady ProcessState = iota // 0: 就绪
	StateRunning                   // 1: 运行
	StateWaiting                   // 2: 等待/阻塞
	StateSuspend                   // 3: 挂起
	StateFinished                  // 4: 完成/撤销
)

// PCB 结构
type PCB struct {
	PID          int
	State        ProcessState
	Priority     int
	CpuTimeSlice int
	RunTime      int
	Next         *PCB // 极其重要的指针：不仅用于队列排队，也用于在空闲时串联起来
}

// SLUB 内存池结构
type SlubPool struct {
	PoolSize int
	FreeList *PCB  // 指向第一个空闲的 PCB
	Memory   []PCB // 模拟操作系统的整块物理连续内存
}

// 调度器结构 (在下一阶段深入使用，目前只做声明)
type Scheduler struct {
	ReadyQueue   *PCB
	WaitQueue    *PCB
	SuspendQueue *PCB
	RunningPCB   *PCB
	PcbSlubPool  *SlubPool
}

// ==========================================
// 2. SLUB 分配器的核心操作 (Step 2 核心内容)
// ==========================================

// Init: 系统开机时的初始化操作
func (pool *SlubPool) Init(size int) {
	pool.PoolSize = size
	// 操作系统一次性申请一大块连续内存（Go 中的切片）
	pool.Memory = make([]PCB, size)

	// 用 Next 指针，把所有空闲的 PCB 串成单向链表
	for i := 0; i < size-1; i++ {
		pool.Memory[i].Next = &pool.Memory[i+1]
	}
	// 最后一个档案盒后面没有了，指向 nil
	pool.Memory[size-1].Next = nil

	// 头指针指向第 0 个，即链表头部
	pool.FreeList = &pool.Memory[0]
}

// AllocPCB: 申请一个 PCB (进程创建时调用)
func (pool *SlubPool) AllocPCB() *PCB {
	// 如果池子空了（头指针为 nil），说明系统内存耗尽，拒绝分配
	if pool.FreeList == nil {
		fmt.Println("[内核错误] SLUB 分配器：内存池已耗尽！")
		return nil
	}

	// 把当前头上那个摘下来
	allocated := pool.FreeList
	
	// 把头指针往后移一位（让第二个变成新的头）
	pool.FreeList = pool.FreeList.Next
	
	// 为安全起见，把摘下来的这个 PCB 的 Next 掐断
	allocated.Next = nil 

	return allocated
}

// FreePCB: 回收一个 PCB (进程撤销时调用)
func (pool *SlubPool) FreePCB(pcb *PCB) {
	// 数据清空
	pcb.PID = 0
	pcb.State = StateFinished

	// 经典的“头插法”：把刚回收的盒子，插回链表的最开头
	pcb.Next = pool.FreeList
	pool.FreeList = pcb
}

// PrintSnapshot: 打印内存池状态快照（满足实验要求）
func (pool *SlubPool) PrintSnapshot() {
	freeCount := 0
	curr := pool.FreeList
	for curr != nil {
		freeCount++
		curr = curr.Next
	}
	usedCount := pool.PoolSize - freeCount

	fmt.Println("========================================")
	fmt.Printf("【SLUB 内存池快照】 总容量: %d | 已分配: %d | 剩余空闲: %d\n", pool.PoolSize, usedCount, freeCount)
	fmt.Println("========================================")
}

// ==========================================
// 3. 主函数验证
// ==========================================
func main() {
	fmt.Println("=== 操作系统进程与 SLUB 内存管理模拟 ===")
	
	var myPool SlubPool
	
	// 1. 初始快照
	fmt.Println("\n[内核动作] 系统开机，初始化大小为 10 的 PCB 内存池...")
	myPool.Init(10)
	myPool.PrintSnapshot()

	// 2. 运行前快照
	fmt.Println("\n[内核动作] 开始创建 3 个进程，正在申请内存...")
	p1 := myPool.AllocPCB()
	if p1 != nil {
		p1.PID = 1001
		p1.State = StateReady
	}

	p2 := myPool.AllocPCB()
	if p2 != nil {
		p2.PID = 1002
		p2.State = StateReady
	}

	p3 := myPool.AllocPCB()
	if p3 != nil {
		p3.PID = 1003
		p3.State = StateReady
	}
	myPool.PrintSnapshot()

	// 3. 运行后快照
	fmt.Println("\n[内核动作] 进程 PID: 1002 运行完毕，正在撤销并归还 PCB...")
	myPool.FreePCB(p2)
	myPool.PrintSnapshot()
}
package main

import "fmt"

type ProcessState int

const(
	StateReady ProcessState = iota
	StateRunning
	StateWaiting
	StateSuspend
	StateFinished
)

type PCB struct {
	PID int
	State ProcessState
	Priority int
	CpuTimeSlice int
	Runtime int

	Next *PCB
}

type SlubPool struct {
	PoolSize int
	FreeList *PCB
	Memory []PCB
}

type Scheduler struct {
	ReadyQueue *PCB
	WaitQueue *PCB
	SuspendQueue *PCB

	RunningPCB *PCB

	PcbSlubPool *SlubPool
}


func main() {
	fmt.Println("---操作系统进程与Slub内存管理模拟---")
	fmt.Println("first结构数据定义完毕")

	var pcb1 PCB
	pcb1.PID = 1001
	pcb1.State = StateReady
	fmt.Printf("创建了一个进程，PID: %d, 当前状态代码: %d\n", pcb1.PID, pcb1.State)
}
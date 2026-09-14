# Trap 的进入与退出。
#
# 这是整个内核里唯一不能用 Rust 写的地方。原因很实在：函数调用一定会
# 用到寄存器（至少 ra 和 sp），而我们要做的恰恰是"在动任何寄存器之前，
# 先把所有寄存器原封不动地存起来"。这是个鸡生蛋问题，只能用汇编破。
#
# ## sscratch 的用法
#
# 用户程序在 U 态运行时，sp 指向用户栈。trap 一进来我们必须换到内核栈，
# 但又得先把用户栈的值保存下来 —— 而此刻没有任何空闲寄存器可用。
#
# RISC-V 给了一个专门的解法：sscratch。约定是
#
#   U 态运行期间： sp = 用户栈,  sscratch = 内核栈
#   S 态运行期间： sp = 内核栈,  sscratch = 用户栈
#
# 于是一条 `csrrw sp, sscratch, sp`（原子交换）就完成了栈的切换，
# 同时把用户栈保存进了 sscratch。一条指令，零额外寄存器。
#
# ## 两个入口
#
#   __alltraps  从用户态进内核（由 stvec 指向，CPU 自动跳来）
#   __restore   从内核回用户态（也用来"启动"一个新程序）

.altmacro

# 保存/恢复第 n 号通用寄存器。偏移正好是 n*8，
# 这就是 TrapContext 里保留 x0 位置的价值。
.macro SAVE_GP n
    sd x\n, \n*8(sp)
.endm
.macro LOAD_GP n
    ld x\n, \n*8(sp)
.endm

    .section .text
    .globl __alltraps
    .globl __restore
    # stvec 要求 trap 入口 4 字节对齐（低 2 位用来编码模式）
    .align 2

# ---------------------------------------------------------------------------
# __alltraps：用户态 → 内核态
# ---------------------------------------------------------------------------
__alltraps:
    # 交换 sp 和 sscratch。此后：
    #   sp       → 内核栈
    #   sscratch → 用户栈（暂存，稍后存进 TrapContext）
    csrrw sp, sscratch, sp

    # 在内核栈上腾出 34*8 = 272 字节，放 TrapContext
    addi sp, sp, -34*8

    # --- 保存通用寄存器 ---
    # x0 恒为 0，不用存
    sd x1, 1*8(sp)
    # x2 是 sp，此刻已经指向内核栈了，它原来的值在 sscratch 里，稍后处理
    sd x3, 3*8(sp)
    # x4 是 tp（线程指针），本章的用户程序不用它，跳过
    # x5 ~ x31
    .set n, 5
    .rept 27
        SAVE_GP %n
        .set n, n+1
    .endr

    # --- 保存 CSR ---
    # 到这里 t0/t1/t2（也就是 x5/x6/x7）已经存好了，可以随便用
    csrr t0, sstatus
    csrr t1, sepc
    sd t0, 32*8(sp)
    sd t1, 33*8(sp)

    # 把暂存在 sscratch 里的用户栈指针，存进 TrapContext 的 x2 位置
    csrr t2, sscratch
    sd t2, 2*8(sp)

    # trap_handler(cx: &mut TrapContext) —— 第一个参数放 a0。
    # sp 此刻正好指向刚填好的 TrapContext，直接传过去。
    mv a0, sp
    call trap_handler

    # trap_handler 返回的还是那个 TrapContext 指针（在 a0 里），
    # 直接落进下面的 __restore。

# ---------------------------------------------------------------------------
# __restore：内核态 → 用户态
# ---------------------------------------------------------------------------
#
# 有两种进入方式，它们走的是同一段代码 —— 这正是设计的精妙之处：
#
#   1) 从 __alltraps 自然落下来（处理完一次 trap）
#   2) 被 __switch 的 `ret` 跳进来（一个任务第一次被调度）
#
# 两种情况下 sp 都**已经**指向要恢复的 TrapContext，所以这里不需要
# 任何调整：
#
#   情况 1：`call trap_handler` 按调用约定会把 sp 原样还回来，
#           它仍指向 __alltraps 腾出的那块 TrapContext。
#   情况 2：__switch 从 TaskContext 里加载的 sp，正是 loader::init_app_cx
#           压入 TrapContext 后的栈顶。
#
# 第 3 周这里还有一条 `mv sp, a0`，因为那时 __restore 是被 Rust 当普通
# 函数调用的（参数在 a0）。改成任务切换模型后 a0 不再承载这个含义，
# 那条指令必须删掉 —— 留着的话，第一次调度时会把 sp 设成 __switch
# 遗留在 a0 里的垃圾值。
__restore:
    # 把 trap 入口切回用户态版本。
    #
    # 内核执行期间 stvec 指向 __kernel_trap（内核自己出错时走清晰的 panic
    # 路径），回到用户态之前必须换回 __alltraps。放在这里而不是放在
    # trap_handler 末尾，是因为**所有**进入用户态的路径都会经过 __restore：
    # 既包括处理完 trap 返回，也包括任务第一次被调度。少覆盖一条路径，
    # 就会出现"某个任务的第一次系统调用把内核打飞"这种极难复现的 bug。
    #
    # t0 紧接着就会被下面从 TrapContext 里重新加载，所以借用它是安全的。
    la      t0, __alltraps
    csrw    stvec, t0

    # --- 先恢复 CSR ---
    # 必须赶在恢复通用寄存器之前，因为这里要用 t0/t1/t2 当中转
    ld t0, 32*8(sp)     # sstatus：决定 sret 回到哪个特权级
    ld t1, 33*8(sp)     # sepc：   决定 sret 回到哪条指令
    ld t2, 2*8(sp)      # 用户栈指针
    csrw sstatus, t0
    csrw sepc, t1
    csrw sscratch, t2   # 放回 sscratch，供下一次 trap 使用

    # --- 恢复通用寄存器 ---
    ld x1, 1*8(sp)
    ld x3, 3*8(sp)
    .set n, 5
    .rept 27
        LOAD_GP %n
        .set n, n+1
    .endr

    # 释放内核栈上的 TrapContext
    addi sp, sp, 34*8

    # 再交换回去。此后：
    #   sp       → 用户栈
    #   sscratch → 内核栈（等待下一次 trap）
    csrrw sp, sscratch, sp

    # 回到 U 态，从 sepc 处继续执行
    sret

# ---------------------------------------------------------------------------
# __kernel_trap：内核**自己**触发 trap 时的入口
# ---------------------------------------------------------------------------
#
# __alltraps 假定 trap 一定来自用户态 —— 它上来就 `csrrw sp, sscratch, sp`。
# 可如果 trap 其实来自内核，这一下会把 sp 换成一个毫无意义的值
# （内核运行期间 sscratch 存的是用户栈，启动阶段更是 0），
# 接下来的"保存现场"会把寄存器写进非法地址，于是**真正的故障原因
# 被彻底掩盖**，只留下一个指向 __alltraps 内部的诡异缺页地址。
#
# 这不是假想：第 7 周开启分页时就是这么被坑了一次 —— 内核往带 U 位的页
# 写数据触发 StorePageFault，而报出来的却是 sp=0 之后的连锁反应。
#
# 所以内核态另设一个入口：什么都不换，带着当前完好的内核栈直接去 panic。
# 这样 scause / stval / sepc 都还是第一现场。

    .align 2
    .globl __kernel_trap
__kernel_trap:
    call    kernel_trap_handler

# Trap 的进入与退出（跳板版本）。
#
# 第 8 周起，每个进程有自己的地址空间，内核也有。于是 trap 处理多了一件
# 第 3-7 周不需要做的事：**换页表**。
#
# ## 为什么这段代码必须单独占一页
#
# `csrw satp, t0` 执行完之后，取下一条指令用的已经是新页表了。
# 如果新页表里没有"当前正在执行的这段代码"，PC 立刻指向一片虚空 ——
# 而且此时连 trap 都处理不了（stvec 指向的地址同样没映射），CPU 直接卡死，
# 没有任何报错。
#
# 所以这段代码被放进 .text.trampoline 段，由链接脚本保证它独占一整页，
# 然后被映射到**每一个**地址空间的同一个虚拟地址（config::TRAMPOLINE）上。
# 换页表的前后，PC 指向的都是这同一页 —— 于是切换是无缝的。
#
# ## 三条信息从哪来
#
# __alltraps 换页表需要知道：内核的 satp、内核栈在哪、trap_handler 在哪。
# 这三样都不能放在寄存器里（寄存器正要被保存），也不能从内核数据结构读
# （还没切到内核页表）。
#
# 解法是：把它们**预先写进 TrapContext**。trap_return 在返回用户态之前
# 填好，__alltraps 陷入时直接从 TrapContext 里读出来。
#
# ## sscratch 的新含义
#
#   U 态运行期间： sp = 用户栈,       sscratch = TRAP_CONTEXT（固定虚拟地址）
#   进入 __alltraps 后： sp = TRAP_CONTEXT, sscratch = 用户栈
#
# 和第 3 周相比，sscratch 存的从"内核栈"变成了"TrapContext 的地址"。
# 因为现在内核栈在**另一个地址空间**里，用户页表下根本访问不到。

.altmacro

# 保存/恢复第 n 号通用寄存器。偏移正好是 n*8，
# 这就是 TrapContext 里保留 x0 位置的价值。
.macro SAVE_GP n
    sd x\n, \n*8(sp)
.endm
.macro LOAD_GP n
    ld x\n, \n*8(sp)
.endm

    .section .text.trampoline
    .globl __alltraps
    .globl __restore
    # stvec 要求 trap 入口 4 字节对齐（低 2 位用来编码模式）
    .align 2

# ---------------------------------------------------------------------------
# __alltraps：用户地址空间 → 内核地址空间
# ---------------------------------------------------------------------------
__alltraps:
    # 交换 sp 和 sscratch。此后：
    #   sp       → TrapContext（用户地址空间里的固定虚拟地址）
    #   sscratch → 用户栈（暂存，稍后存进 TrapContext）
    csrrw sp, sscratch, sp

    # 注意：这里**不再需要** `addi sp, sp, -34*8` 了。
    # 第 3-7 周 TrapContext 是临时压在内核栈上的，现在它有自己的固定页，
    # sp 一换就正好指着它。

    # --- 保存通用寄存器 ---
    sd x1, 1*8(sp)
    # x2 是 sp，原值在 sscratch 里，稍后处理
    sd x3, 3*8(sp)
    # x4 是 tp，用户程序不用它，跳过
    .set n, 5
    .rept 27
        SAVE_GP %n
        .set n, n+1
    .endr

    # --- 保存 CSR ---
    # t0/t1/t2（x5/x6/x7）已经存好了，可以随便用
    csrr t0, sstatus
    csrr t1, sepc
    sd t0, 32*8(sp)
    sd t1, 33*8(sp)
    csrr t2, sscratch
    sd t2, 2*8(sp)

    # --- 取出内核那三样东西 ---
    # 它们由 trap_return 在返回用户态之前预先填好
    ld t0, 34*8(sp)     # kernel_satp
    ld t1, 36*8(sp)     # trap_handler 的地址
    ld sp, 35*8(sp)     # kernel_sp（内核栈顶，在内核地址空间里）

    # --- 换到内核地址空间 ---
    # 这两条指令之后，取指就走内核页表了。它们本身位于跳板页上，
    # 而跳板页在两个地址空间里都映射到同一个虚拟地址 —— 所以没事。
    csrw satp, t0
    sfence.vma

    # 跳进内核的 trap 处理。用 jr 而不是 call：我们不打算回来，
    # 返回用户态走的是 trap_return → __restore 这条路。
    jr t1

# ---------------------------------------------------------------------------
# __restore：内核地址空间 → 用户地址空间
# ---------------------------------------------------------------------------
#
#   a0 = TrapContext 的虚拟地址（恒为 config::TRAP_CONTEXT）
#   a1 = 目标用户地址空间的 satp
#
# 和第 3-7 周不同，现在它是被 trap_return 用 `jr` 显式跳进来的，
# 而不是从 __alltraps 自然落下来 —— 因为中间隔着一次地址空间切换。
__restore:
    # 换到用户地址空间。此后 sp 还指着内核栈（已经不可用了），
    # 但我们马上就要重设它。
    csrw satp, a1
    sfence.vma

    # 供下一次 trap 使用
    csrw sscratch, a0
    # sp 指向 TrapContext（此刻在用户地址空间里可见）
    mv sp, a0

    # --- 先恢复 CSR ---
    # 必须赶在恢复通用寄存器之前，因为这里要用 t0/t1 当中转
    ld t0, 32*8(sp)     # sstatus：决定 sret 回到哪个特权级
    ld t1, 33*8(sp)     # sepc：   决定 sret 回到哪条指令
    csrw sstatus, t0
    csrw sepc, t1

    # --- 恢复通用寄存器 ---
    # 注意这会覆盖 a0/a1（x10/x11），所以上面用到它们的地方都在前面
    ld x1, 1*8(sp)
    ld x3, 3*8(sp)
    .set n, 5
    .rept 27
        LOAD_GP %n
        .set n, n+1
    .endr

    # 最后恢复用户栈指针（存在 x2 的位置）
    ld sp, 2*8(sp)

    # 回到 U 态，从 sepc 处继续执行
    sret

# ---------------------------------------------------------------------------
# __kernel_trap：内核**自己**触发 trap 时的入口
# ---------------------------------------------------------------------------
#
# 注意它在普通的 .text 段里，不在跳板页上 —— 内核态的 trap 只会在
# 内核地址空间里发生，不需要跨地址空间。
#
# __alltraps 假定 trap 一定来自用户态，上来就 `csrrw sp, sscratch, sp`。
# 可如果 trap 其实来自内核，这一下会把 sp 换成一个毫无意义的值，
# 接下来的"保存现场"会把寄存器写进非法地址，于是**真正的故障原因
# 被彻底掩盖**。
#
# 第 7 周开启分页时就是这么被坑了一次：内核往带 U 位的页写数据触发
# StorePageFault，报出来的却是 sp=0 之后的连锁反应。
#
# 所以内核态另设一个入口：什么都不换，带着当前完好的内核栈直接去 panic。
# 这样 scause / stval / sepc 都还是第一现场。

    .section .text
    .align 2
    .globl __kernel_trap
__kernel_trap:
    call    kernel_trap_handler

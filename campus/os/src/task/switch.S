# 任务切换。
#
# __switch(current: *mut TaskContext, next: *const TaskContext)
#
# 这段代码做的事听起来平淡：把当前寄存器存进 current，
# 再从 next 里加载出来。但它有一个反直觉的性质 ——
#
#   **它从一个任务进去，从另一个任务出来。**
#
# 调用它的是任务 A，而最后那条 `ret` 返回到的是任务 B 上次
# 调用 __switch 的位置。A 的执行流就停在这里了，
# 直到将来某个任务再 __switch 回它。
#
# 从 Rust 侧看，`__switch(a, b)` 就是一次普通的函数调用，会正常返回。
# 但"返回"的那一刻，可能已经过去了很久，中间跑过了别的任务。
#
# ## 为什么不用保存 t0-t6 / a0-a7
#
# 因为 __switch 是被 Rust 代码**主动调用**的普通函数。RISC-V 调用约定
# 规定：调用方负责保存自己还需要的 caller-saved 寄存器（t*、a*），
# 被调用方负责保存 callee-saved 寄存器（s0-s11）。
#
# 编译器已经在调用点替我们生成了保存 t*/a* 的代码。我们只要守住
# 被调用者这一侧的约定就行 —— 这就是为什么 TaskContext 只有 14 个字段，
# 而硬件触发的 TrapContext 需要 34 个。

.altmacro

.macro SAVE_SN n
    sd s\n, (\n+2)*8(a0)
.endm
.macro LOAD_SN n
    ld s\n, (\n+2)*8(a1)
.endm

    .section .text
    .globl __switch
__switch:
    # ---- 把当前执行流的状态存进 a0 指向的 TaskContext ----
    # 布局：偏移 0 = ra，偏移 8 = sp，偏移 16.. = s0~s11
    sd sp, 8(a0)
    sd ra, 0(a0)
    .set n, 0
    .rept 12
        SAVE_SN %n
        .set n, n+1
    .endr

    # ---- 从 a1 指向的 TaskContext 恢复另一个执行流 ----
    ld ra, 0(a1)
    .set n, 0
    .rept 12
        LOAD_SN %n
        .set n, n+1
    .endr
    # sp 必须最后换：上面几条指令还在用旧的栈（虽然只是取址，
    # 但保持"换栈是最后一步"这个习惯能避免很多麻烦）
    ld sp, 8(a1)

    # 跳到新任务的 ra。
    # 第一次被调度的任务，ra = __restore，于是它会返回用户态；
    # 之前让出过 CPU 的任务，ra = 它上次 __switch 的下一条指令。
    ret

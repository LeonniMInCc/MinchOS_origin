# CCNUOS 内核入口
#
# OpenSBI 跳转到这里时，我们拥有 S 态特权级，但什么都没有：
# 没有栈、没有清零的 .bss、没有 Rust 运行时。
# 这段汇编唯一的工作就是"把栈支起来"，然后马上把控制权交给 Rust。

    .section .text.entry
    .globl _start
_start:
    # sp 指向启动栈的栈顶（RISC-V 的栈向低地址生长）
    la      sp, boot_stack_top
    # 跳进 Rust 世界，不再返回
    call    rust_main

# 启动栈：64 KiB。放在 .bss.stack 段，由链接脚本保证它排在 sbss 之前。
    .section .bss.stack
    .globl boot_stack_lower_bound
boot_stack_lower_bound:
    .space 4096 * 16
    .globl boot_stack_top
boot_stack_top:

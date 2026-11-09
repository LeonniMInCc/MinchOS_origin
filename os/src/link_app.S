# 本文件由 os/build.rs 自动生成，请勿手动修改。
#
# _num_app 的布局：
#   [0]              程序个数 N
#   [1 ..= N+1]      N+1 个地址，第 i 个程序占 [app_i_start, app_i_end)

    .align 3
    .section .data
    .global _num_app
_num_app:
    .quad 5
    .quad app_0_start
    .quad app_1_start
    .quad app_2_start
    .quad app_3_start
    .quad app_4_start
    .quad app_4_end

    # 00hello_world
    .section .data
    .global app_0_start
    .global app_0_end
    .align 3
app_0_start:
    .incbin "../user/target/riscv64gc-unknown-none-elf/release/00hello_world.bin"
app_0_end:

    # 01power
    .section .data
    .global app_1_start
    .global app_1_end
    .align 3
app_1_start:
    .incbin "../user/target/riscv64gc-unknown-none-elf/release/01power.bin"
app_1_end:

    # 02bad_syscall_arg
    .section .data
    .global app_2_start
    .global app_2_end
    .align 3
app_2_start:
    .incbin "../user/target/riscv64gc-unknown-none-elf/release/02bad_syscall_arg.bin"
app_2_end:

    # 03bad_instruction
    .section .data
    .global app_3_start
    .global app_3_end
    .align 3
app_3_start:
    .incbin "../user/target/riscv64gc-unknown-none-elf/release/03bad_instruction.bin"
app_3_end:

    # 04bad_address
    .section .data
    .global app_4_start
    .global app_4_end
    .align 3
app_4_start:
    .incbin "../user/target/riscv64gc-unknown-none-elf/release/04bad_address.bin"
app_4_end:

/* 用户程序链接脚本
 *
 * 本章没有分页，所以所有用户程序都被硬链接到同一个物理地址
 * 0x80400000 —— 也就是内核 config.rs 里的 APP_BASE_ADDRESS。
 * 内核每次只能装一个程序，装新的就覆盖旧的。
 *
 * 第 7 周引入分页后，这个地址会变成"虚拟地址"，
 * 每个程序都可以以为自己独占它。
 */

OUTPUT_ARCH(riscv)
ENTRY(_start)

BASE_ADDRESS = 0x80400000;

SECTIONS
{
    . = BASE_ADDRESS;

    .text : {
        /* 入口必须是程序的第一条指令：内核直接跳到 BASE_ADDRESS */
        *(.text.entry)
        *(.text .text.*)
    }

    . = ALIGN(4K);
    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }

    . = ALIGN(4K);
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }

    .bss : {
        start_bss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        end_bss = .;
    }

    /* 用户程序不需要自带栈 —— 内核会在 sp 里放好 USER_STACK 的栈顶 */

    /DISCARD/ : {
        *(.eh_frame)
        *(.debug*)
    }
}

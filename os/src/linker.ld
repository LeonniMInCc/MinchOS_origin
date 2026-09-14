/* CCNUOS 内核链接脚本
 *
 * 约定：QEMU virt 机器上电后先跑 OpenSBI（位于 0x80000000），
 * OpenSBI 初始化完毕后跳转到 0x80200000 —— 也就是内核的第一条指令。
 * 所以我们必须把 .text.entry 精确地放在这个地址上。
 *
 * 同时导出一批符号（stext/etext/sbss/ebss/ekernel ...），
 * Rust 代码里用 extern "C" 引用它们，就能知道自己的内存布局。
 */

OUTPUT_ARCH(riscv)
ENTRY(_start)

BASE_ADDRESS = 0x80200000;

SECTIONS
{
    . = BASE_ADDRESS;
    skernel = .;

    stext = .;
    .text : {
        /* 入口汇编必须是整个内核的第一段代码 */
        *(.text.entry)
        *(.text .text.*)
    }
    . = ALIGN(4K);
    etext = .;

    srodata = .;
    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }
    . = ALIGN(4K);
    erodata = .;

    sdata = .;
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }
    . = ALIGN(4K);
    edata = .;

    /* 启动栈也放在 .bss 里，但要排在 sbss 之前：
     * 清零 .bss 的代码自己正跑在这个栈上，不能把它一起清掉。 */
    sbss_with_stack = .;
    .bss : {
        *(.bss.stack)
        sbss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
    }
    . = ALIGN(4K);
    ebss = .;

    ekernel = .;

    /DISCARD/ : {
        *(.eh_frame)
        *(.debug_gdb_scripts)
    }
}

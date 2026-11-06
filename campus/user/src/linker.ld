/* 用户程序链接脚本
 *
 * 从第 8 周起，**所有用户程序都链接到同一个地址**。
 *
 * 这在第 4–7 周是做不到的：那时没有（或只有一个共享的）地址空间，
 * 所有程序同时躺在同一片物理内存里，必须各占一个互不重叠的槽位，
 * 于是要用 tools/build-user.py 逐个改基址重新链接。
 *
 * 现在每个进程有自己的地址空间，0x10000 在不同进程里指向不同的物理页。
 * 那个脚本因此被删掉了，构建回归成一条朴素的 `cargo build --release`。
 *
 * 这是"抽象带来简化"最直观的一个例子：多出来的一层间接（页表）
 * 反而让上层少了一整个构建步骤。
 *
 * 0x10000 这个地址没什么特别，只要落在用户地址空间的低端、
 * 离栈和跳板页足够远即可。选它是因为传统上 Unix 也用类似的低地址。
 */

OUTPUT_ARCH(riscv)
ENTRY(_start)

BASE_ADDRESS = 0x10000;

SECTIONS
{
    . = BASE_ADDRESS;

    /* 每个段都要页对齐并单独成段：内核会按程序头逐段建立映射，
     * 代码段给 R-X、只读段给 R--、数据段给 RW-。
     * 两个段挤在同一页里的话，那一页只能取并集权限，保护就失效了。 */
    .text : {
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

    /* 用户程序不需要自带栈 —— 内核建地址空间时会给它划一块，
     * 并在下面留一页 guard page */

    /DISCARD/ : {
        *(.eh_frame)
        *(.debug*)
    }
}

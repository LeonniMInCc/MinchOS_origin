/* 用户程序链接脚本
 *
 * 本章没有分页，所有用户程序同时躺在同一片物理内存里，各占一个
 * 128 KiB 的槽位：
 *
 *     程序 0 → 0x80800000
 *     程序 1 → 0x80820000
 *     程序 2 → 0x80840000
 *     ...
 *
 * 程序里的每一个绝对地址（函数地址、全局变量、字符串字面量）在链接时
 * 就被写死了，所以一个链接到 0x80800000 的程序放到 0x80820000 上运行，
 * 会立刻跳到错误的地方。每个程序必须单独链接一次。
 *
 * 基址由 tools/build-user.py 通过链接器的 --defsym 传进来：
 *
 *     -Clink-arg=--defsym=APP_BASE=0x80820000
 *
 * 这样做的好处是基址成了 RUSTFLAGS 的一部分，而 cargo 的构建指纹
 * 包含 RUSTFLAGS —— 换了基址必然重新链接，不需要任何删文件的偏方。
 *
 * 没有传 APP_BASE 时退化到默认值，直接在 user/ 里跑 cargo build 也能工作。
 *
 * 第 7 周引入分页后，这个地址会变成**虚拟地址**，所有程序都可以
 * 以为自己独占同一个地址，而实际占用互不相同的物理页。
 * 那时 --defsym 这套机制就可以退休了。
 */

OUTPUT_ARCH(riscv)
ENTRY(_start)

BASE_ADDRESS = DEFINED(APP_BASE) ? APP_BASE : 0x80800000;

SECTIONS
{
    . = BASE_ADDRESS;

    .text : {
        /* 入口必须是程序的第一条指令：内核直接跳到槽位基址 */
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

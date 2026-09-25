/* linker.ld */
OUTPUT_ARCH(riscv)
ENTRY(_start)

BASE_ADDRESS = 0x80200000;

SECTIONS
{
    . = BASE_ADDRESS;

    .text : {
        *(.text.entry)
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.*)
    }

    .data : {
        *(.data .data.*)
    }

    .bss : {
        *(.bss .bss.*)
        . = ALIGN(4K);
        PROVIDE(boot_stack_bottom = .);
        . += 4096 * 4;
        PROVIDE(boot_stack_top = .);
    }
}

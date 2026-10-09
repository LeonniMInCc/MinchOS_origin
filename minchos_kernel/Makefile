# CCNUOS ch01 Makefile

TARGET := riscv64gc-unknown-none-elf
MODE   := release
KERNEL_ELF := target/$(TARGET)/$(MODE)/os
KERNEL_BIN := $(KERNEL_ELF).bin

OBJCOPY := rust-objcopy --binary-architecture=riscv64

ifeq ($(MODE), release)
	MODE_ARG := --release
endif

build:
	cargo build $(MODE_ARG)

$(KERNEL_BIN): build
	$(OBJCOPY) $(KERNEL_ELF) --strip-all -O binary $@

kernel: $(KERNEL_BIN)

clean:
	cargo clean

run: kernel
	qemu-system-riscv64 \
		-machine virt \
		-nographic \
		-bios default \
		-kernel $(KERNEL_ELF)

asm:
	rust-objdump -S --arch-name=riscv64 $(KERNEL_ELF) | less

.PHONY: build kernel clean run asm

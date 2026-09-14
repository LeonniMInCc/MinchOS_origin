# CCNUOS 顶层构建脚本
#
#   make run            构建并在 QEMU 中运行
#   make debug          以调试模式启动 QEMU（挂起，等待 GDB 连接）
#   make gdb            另开一个终端连上去
#   make asm            反汇编内核，检查生成的代码
#   LOG=DEBUG make run  调高日志级别

TARGET          := riscv64gc-unknown-none-elf
MODE            ?= release
LOG             ?= INFO

OS_DIR          := os
KERNEL_ELF      := $(OS_DIR)/target/$(TARGET)/$(MODE)/os
KERNEL_BIN      := $(KERNEL_ELF).bin

# OpenSBI 固件加载在 0x80000000，初始化完成后跳转到这里。
# 这个地址必须和 os/src/linker.ld 里的 BASE_ADDRESS 完全一致。
KERNEL_ENTRY_PA := 0x80200000

# Homebrew 装的 rustup 是 keg-only，不在默认 PATH 里。
# 如果你的 rustc 已经在 PATH 中（比如官方 rustup-init 装的），下面会自动用你的。
RUSTUP_BIN := /opt/homebrew/opt/rustup/bin
export PATH := $(RUSTUP_BIN):$(PATH)

RUSTC   := $(shell command -v rustc 2>/dev/null || echo $(RUSTUP_BIN)/rustc)
CARGO   := $(shell command -v cargo 2>/dev/null || echo $(RUSTUP_BIN)/cargo)
SYSROOT := $(shell $(RUSTC) --print sysroot 2>/dev/null)

# 直接用 rustup llvm-tools 组件自带的工具，省掉 cargo-binutils 的编译时间
OBJCOPY := $(shell find $(SYSROOT)/lib/rustlib -name 'llvm-objcopy' -type f 2>/dev/null | head -1)
OBJDUMP := $(shell find $(SYSROOT)/lib/rustlib -name 'llvm-objdump' -type f 2>/dev/null | head -1)

ifeq ($(MODE), release)
CARGO_FLAGS := --release
else
CARGO_FLAGS :=
endif

QEMU       := qemu-system-riscv64
# 用 -kernel 而不是 -device loader：QEMU 11 自带的 OpenSBI 是 fw_dynamic 形态，
# 它要靠 QEMU 填进来的启动信息才知道下一级该跳到哪。用 -device loader 只是把
# 字节搬进内存，固件的 "Next Address" 会是 0，于是 OpenSBI 打完 banner 就卡死。
# -kernel 则会让 QEMU 把内核加载到 0x80200000 并把这个地址告诉 OpenSBI。
QEMU_ARGS  := -machine virt \
              -nographic \
              -bios default \
              -kernel $(KERNEL_BIN)

DEMO_LOG   := docs/demo.log
DEMO_HTML  := docs/demo.html

.PHONY: all build run debug gdb clean fmt clippy asm env check demo

all: build

# 先确认工具链齐全，缺什么立刻报出来，别等到链接阶段才炸
env:
	@test -x "$(RUSTC)" || { echo "找不到 rustc：请先 rustup default stable"; exit 1; }
	@command -v $(QEMU) >/dev/null || { echo "找不到 $(QEMU)：请先 brew install qemu"; exit 1; }
	@test -n "$(OBJCOPY)" || { echo "找不到 llvm-objcopy：请先 rustup component add llvm-tools"; exit 1; }

# 注意：必须 cd 进 os/ 再调 cargo。cargo 是按"当前工作目录"向上查找
# .cargo/config.toml 的，用 --manifest-path 的话 os/.cargo/config.toml
# 会被完全忽略，于是按宿主机架构编译，报一堆 "unknown register a0"。
build: env
	@cd $(OS_DIR) && LOG=$(LOG) $(CARGO) build $(CARGO_FLAGS)
	@# 剥掉 ELF 头，得到可以直接加载到 0x80200000 的裸二进制
	@$(OBJCOPY) --strip-all $(KERNEL_ELF) -O binary $(KERNEL_BIN)
	@echo "内核镜像: $(KERNEL_BIN)  ($$(wc -c < $(KERNEL_BIN) | tr -d ' ') 字节)"

run: build
	@echo "启动 QEMU（Ctrl-A 然后按 X 可强制退出）"
	@echo "------------------------------------------------------------"
	@$(QEMU) $(QEMU_ARGS)

# 录一次启动，渲染成可在浏览器里回放的单文件 HTML。
# 适合放进答辩材料，或者发给没装 QEMU 的人看。
demo: build
	@bash tools/capture.sh $(DEMO_LOG) $(QEMU) $(QEMU_ARGS)
	@python3 tools/ansi2html.py $(DEMO_LOG) $(DEMO_HTML)
	@echo "用浏览器打开：$(CURDIR)/$(DEMO_HTML)"

# 挂起在第一条指令，等 GDB 连 localhost:1234
debug: build
	@$(QEMU) $(QEMU_ARGS) -s -S

gdb:
	@command -v riscv64-elf-gdb >/dev/null || { \
		echo "缺少 riscv64-elf-gdb，装一下："; \
		echo "  brew install riscv64-elf-gdb"; exit 1; }
	@riscv64-elf-gdb \
		-ex 'file $(KERNEL_ELF)' \
		-ex 'set arch riscv:rv64' \
		-ex 'target remote localhost:1234'

asm: build
	@$(OBJDUMP) -d --mcpu=generic-rv64 $(KERNEL_ELF) | less

check: env
	@cd $(OS_DIR) && $(CARGO) check $(CARGO_FLAGS)

fmt:
	@cd $(OS_DIR) && $(CARGO) fmt

clippy:
	@cd $(OS_DIR) && $(CARGO) clippy $(CARGO_FLAGS)

clean:
	@cd $(OS_DIR) && $(CARGO) clean

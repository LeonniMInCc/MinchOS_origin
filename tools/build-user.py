#!/usr/bin/env python3
"""逐个编译用户程序，每个链接到不同的基址。

    用法: python3 tools/build-user.py

本章还没有分页，所有用户程序同时躺在同一片物理内存里，各占一个
APP_SIZE_LIMIT 大小的槽位。程序里的绝对地址在链接时就写死了，
所以必须为每个程序单独生成一份链接脚本。

这个脚本是"没有分页"这件事在构建系统上的代价。第 7 周之后就能删掉它。
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
USER = ROOT / "user"
BIN_SRC = USER / "src" / "bin"
TEMPLATE = USER / "src" / "linker.ld.in"
LINKER = USER / "src" / "linker.ld"
TARGET = "riscv64gc-unknown-none-elf"
OUT = USER / "target" / TARGET / "release"

# 必须和 os/src/config.rs 里的常量保持一致
APP_BASE_ADDRESS = 0x8040_0000
APP_SIZE_LIMIT = 0x2_0000


def tool(name: str) -> str:
    """定位 rustc / cargo。Homebrew 的 rustup 是 keg-only，不在默认 PATH 里。"""
    found = shutil.which(name)
    if found:
        return found
    fallback = Path("/opt/homebrew/opt/rustup/bin") / name
    if fallback.exists():
        return str(fallback)
    sys.exit(f"找不到 {name}，请先安装 Rust 工具链")


def find_objcopy() -> str:
    """rustup 的 llvm-tools 里自带 objcopy，不必额外装 cargo-binutils。"""
    sysroot = subprocess.run(
        [tool("rustc"), "--print", "sysroot"], capture_output=True, text=True, check=True
    ).stdout.strip()
    for path in Path(sysroot, "lib", "rustlib").rglob("llvm-objcopy"):
        if path.is_file():
            return str(path)
    sys.exit("找不到 llvm-objcopy，请先 rustup component add llvm-tools")


def app_names() -> list[str]:
    """按文件名排序 —— 这也决定了程序编号和加载槽位。"""
    return sorted(p.stem for p in BIN_SRC.glob("*.rs"))


def main() -> int:
    apps = app_names()
    if not apps:
        print("user/src/bin 下没有找到任何程序")
        return 1

    objcopy = find_objcopy()
    template = TEMPLATE.read_text(encoding="utf-8")

    print(f"编译 {len(apps)} 个用户程序：")
    for app_id, app in enumerate(apps):
        base = APP_BASE_ADDRESS + app_id * APP_SIZE_LIMIT
        LINKER.write_text(template.replace("%APP_BASE%", hex(base)), encoding="utf-8")

        # 删掉上一次的产物，强制 cargo 用新的链接脚本重新链接。
        # 不删的话 cargo 只看源码有没有变，会直接复用旧二进制 ——
        # 于是程序 3 带着程序 0 的基址跑起来，症状是启动即跑飞。
        (OUT / app).unlink(missing_ok=True)

        result = subprocess.run(
            [tool("cargo"), "build", "--release", "--bin", app],
            cwd=USER,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            sys.stderr.write(result.stderr)
            return result.returncode

        subprocess.run(
            [objcopy, "--strip-all", str(OUT / app), "-O", "binary", str(OUT / f"{app}.bin")],
            check=True,
        )
        size = (OUT / f"{app}.bin").stat().st_size
        flag = "  ⚠️ 超出槽位!" if size > APP_SIZE_LIMIT else ""
        print(f"  [{app_id}] {app:<20} base={base:#x}  {size:>6} 字节{flag}")
        if size > APP_SIZE_LIMIT:
            return 1

    # 保留最后生成的 linker.ld（已 gitignore）：cargo 的 rustflags 指向它，
    # 删掉的话直接在 user/ 里跑 cargo 会报"找不到链接脚本"。
    return 0


if __name__ == "__main__":
    sys.exit(main())

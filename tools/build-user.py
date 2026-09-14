#!/usr/bin/env python3
"""逐个编译用户程序，每个链接到不同的基址。

    用法: python3 tools/build-user.py

本章还没有分页，所有用户程序同时躺在同一片物理内存里，各占一个
APP_SIZE_LIMIT 大小的槽位。程序里的绝对地址在链接时就写死了，
所以必须为每个程序单独链接一次。

基址通过链接器的 `--defsym` 传给 `user/src/linker.ld`。这样做的关键好处是：
**基址成了 RUSTFLAGS 的一部分，而 cargo 的构建指纹包含 RUSTFLAGS** ——
换了基址必然重新链接，正确性由构建系统保证。

早先的版本是"改写链接脚本 + 删掉旧产物"，那是错的：cargo 会从
`target/.../deps/` 的缓存里重新拷一份，根本不重新链接，于是程序带着
上一次的基址跑起来，而构建日志一切正常。

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
TARGET = "riscv64gc-unknown-none-elf"
OUT = USER / "target" / TARGET / "release"

# 必须和 os/src/config.rs 里的常量保持一致
APP_BASE_ADDRESS = 0x8080_0000
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


def build_env(base: int) -> dict:
    """构造编译单个程序所需的环境。

    两件事：

    1. 把 rustup 的 bin 目录补进 PATH。cargo 自己会去 PATH 里找 rustc，
       只用绝对路径调 cargo 是不够的 —— 它照样会报
       `could not execute process 'rustc -vV'`。
    2. 用 RUSTFLAGS 指定链接脚本和基址。注意 RUSTFLAGS 环境变量会
       **整体覆盖** .cargo/config.toml 里的 rustflags，所以链接脚本
       那一项也得在这里重复一遍。
    """
    env = os.environ.copy()
    env["PATH"] = str(Path(tool("cargo")).parent) + os.pathsep + env.get("PATH", "")
    env["RUSTFLAGS"] = f"-Clink-arg=-Tsrc/linker.ld -Clink-arg=--defsym=APP_BASE={base:#x}"
    return env


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
    print(f"编译 {len(apps)} 个用户程序：")

    for app_id, app in enumerate(apps):
        base = APP_BASE_ADDRESS + app_id * APP_SIZE_LIMIT
        elf, stripped = OUT / app, OUT / f"{app}.elf"

        # 先删掉上一次的 .elf。内核 .incbin 嵌进去的正是它，而它不受
        # cargo 管理 —— 万一 cargo 失败，旧文件会原地留下，内核照样
        # 构建成功，跑的却是上一次的陈旧代码。一个安静到可怕的失败模式。
        stripped.unlink(missing_ok=True)

        result = subprocess.run(
            [tool("cargo"), "build", "--release", "--bin", app],
            cwd=USER,
            capture_output=True,
            text=True,
            env=build_env(base),
        )
        if result.returncode != 0:
            sys.stderr.write(result.stderr)
            return result.returncode

        # 保留 ELF 格式（不加 -O binary），只去掉符号表和调试信息。
        # 内核需要程序头表才能知道每段该装到哪、该给什么权限。
        subprocess.run([objcopy, "--strip-all", str(elf), str(stripped)], check=True)

        size = stripped.stat().st_size
        if size > APP_SIZE_LIMIT:
            print(f"  [{app_id}] {app}：{size} 字节，超出槽位上限 {APP_SIZE_LIMIT}")
            return 1
        print(f"  [{app_id}] {app:<20} base={base:#x}  {size:>6} 字节")

    return 0


if __name__ == "__main__":
    sys.exit(main())

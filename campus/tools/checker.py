#!/usr/bin/env python3
"""判题引擎。

`tools/grade.py` 用它，别的什么都不用它 —— 这个文件不该有 `main`。

# 为什么只有一个引擎

第 16 周写过一套回归测试，本来可以再写一套判题器。**那会是个错误**：
参考实现改了，回归测试跟着改，判题器忘了改，于是学生做对了却被判错。
两套判据一定会漂移，这只是时间问题。

所以判据只有一份（`docs/checkpoints.toml`），引擎也只有一个：

    make regress        拿参考实现去判题 —— 全过才说明判据本身没坏
    make grade CH=6     拿学生的代码去判题

**"回归测试"和"判题"是同一件事的两个方向。**

# 判据只有五种

| 类型 | 检查什么 |
|---|---|
| `build` | 能不能零告警编译 |
| `unit` | 宿主机单元测试（不需要 QEMU） |
| `selfcheck` | 开机自检的输出 —— 不喂任何命令 |
| `run` | 跑 QEMU、喂命令、断言输出 |
| `source` | 源码里必须/不许出现某些模式 |

刻意做少。判据类型一多，写判据的人就会开始为每道题发明新机制，
而清单的可读性是这套东西能不能长期维护的关键。
"""

import re
import subprocess
import sys
import time
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

ANSI = re.compile(r"\x1b\[[0-9;]*m")
BOOT_WAIT = 2.5  # 等内核启动完再喂命令
TIMEOUT = 90


# ---------------------------------------------------------------------------
# 结果
# ---------------------------------------------------------------------------


@dataclass
class Result:
    name: str
    kind: str
    points: int
    earned: int
    fails: list = field(default_factory=list)
    log: str = ""

    @property
    def ok(self):
        return not self.fails


# ---------------------------------------------------------------------------
# QEMU
# ---------------------------------------------------------------------------


class Runner:
    """跑 QEMU，并且**记住跑过的结果**。

    一次启动要 3–5 秒，而很多判据用的是同一组命令（尤其是不喂任何命令的
    开机自检 —— 十几章都要看它）。按命令列表缓存之后，一轮判题从
    几十次启动降到十几次。
    """

    def __init__(self, qemu_cmd, verbose=False):
        self.qemu = qemu_cmd
        self.verbose = verbose
        self._cache = {}
        self.boots = 0

    def boot(self, commands):
        key = tuple(commands)
        if key not in self._cache:
            self.boots += 1
            if self.verbose:
                print(f"      [启动 QEMU] {commands or '（不喂命令）'}", file=sys.stderr)
            self._cache[key] = self._boot(commands)
        return self._cache[key]

    def _boot(self, commands):
        # 不喂命令时也要给一个 exit，否则 shell 会一直等到超时
        script = ("\n".join(list(commands) + ["exit"]) + "\n").encode()
        proc = subprocess.Popen(
            self.qemu,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        # 太早喂的话字符会落在固件初始化串口之前 —— 真机上也一样会丢
        time.sleep(BOOT_WAIT)
        try:
            out, _ = proc.communicate(script, timeout=TIMEOUT)
            timed_out = False
        except subprocess.TimeoutExpired:
            proc.kill()
            out, _ = proc.communicate()
            timed_out = True
        return ANSI.sub("", out.decode("utf-8", "replace")), timed_out


# ---------------------------------------------------------------------------
# 断言
# ---------------------------------------------------------------------------


def assert_output(out, timed_out, must, must_not):
    """返回失败原因。空列表 = 通过。"""
    fails = []
    if timed_out:
        fails.append(f"超时：{TIMEOUT}s 内没有退出")
    for s in must:
        if s not in out:
            fails.append(f"缺少应有的输出：{s!r}")
    for s in must_not:
        if s in out:
            fails.append(f"出现了不该出现的输出：{s!r}")
    return fails


# ---------------------------------------------------------------------------
# 五种判据
# ---------------------------------------------------------------------------


def _shell(cmd, cwd=ROOT, env=None):
    import os

    e = dict(os.environ)
    # Homebrew 的 rustup 是 keg-only，CI 和本机都可能不在 PATH 里
    e["PATH"] = "/opt/homebrew/opt/rustup/bin:" + e.get("PATH", "")
    if env:
        e.update(env)
    p = subprocess.run(
        cmd, cwd=cwd, env=e, shell=True, capture_output=True, text=True, timeout=600
    )
    return p.returncode, p.stdout + p.stderr


def check_build(spec, runner, meta):
    """零告警编译。

    不能事后 grep 输出：cargo 只在**重新编译**时才打印告警，
    缓存命中时一声不吭，grep 会漏掉一切。所以要让告警变成硬错误。

    # 为什么是 `cargo rustc --` 而不是 `RUSTFLAGS`

    第一版写的是 `RUSTFLAGS='-D warnings' cargo build`，结果链接直接失败，
    报一堆 `undefined symbol: ekernel`。原因是
    **`RUSTFLAGS` 环境变量会整个覆盖掉 `.cargo/config.toml` 里的
    `build.rustflags`**，而链接脚本正是从那里来的
    （`-Clink-arg=-Tsrc/linker.ld`）。

    `cargo rustc -- <args>` 则是**追加**到最终那次 rustc 调用上，
    配置里的 flags 原样保留。而且额外参数进指纹，所以也能强制重编。

    这是这个项目收集到的第三个 cargo 陷阱，前两个是：
    `--manifest-path` 会忽略 `.cargo/config.toml`（第 1 周）、
    链接脚本不算构建输入（第 6、8 周）。
    """
    code, log = _shell("cd os && cargo rustc --release -- -D warnings 2>&1")
    fails = [] if code == 0 else ["编译没通过，或者有告警（告警按错误处理）"]
    return fails, log[-3000:]


def check_unit(spec, runner, meta):
    manifest = spec.get("manifest", "easy-fs/Cargo.toml")
    filt = spec.get("filter", "")
    code, log = _shell(f"cargo test --manifest-path {manifest} {filt} 2>&1")
    fails = []
    if code != 0:
        fails.append("单元测试未全部通过")
    least = spec.get("at_least")
    if least:
        m = re.search(r"(\d+) passed", log)
        n = int(m.group(1)) if m else 0
        if n < least:
            fails.append(f"通过 {n} 个测试，少于要求的 {least} 个（是不是删掉了测试？）")
    return fails, log[-3000:]


def _dedup(seq):
    """保序去重。判据自己的 must 和全局断言撞车时，别把同一条报两遍。"""
    seen, out = set(), []
    for x in seq:
        if x not in seen:
            seen.add(x)
            out.append(x)
    return out


def _run_like(spec, runner, meta, commands):
    out, timed_out = runner.boot(commands)
    must = list(spec.get("must", []))
    must_not = list(spec.get("must_not", []))
    # 全局断言对每一次启动都成立，除非这条判据显式豁免
    if not spec.get("no_global"):
        must += meta.get("global_must", [])
        must_not += meta.get("global_must_not", [])
    return assert_output(out, timed_out, _dedup(must), _dedup(must_not)), out[-3000:]


def check_selfcheck(spec, runner, meta):
    return _run_like(spec, runner, meta, [])


def check_run(spec, runner, meta):
    return _run_like(spec, runner, meta, spec.get("commands", []))


def strip_comments(src):
    """去掉 Rust 源码里的注释，保留字符串字面量。

    # 为什么必须去

    变异测试当场抓到的：把 `easy_fs::sync_all_blocks()` 的调用删掉，
    判据却照样通过 —— 因为那个函数的**文档注释里**提到了这个名字，
    grep 全文就命中了。

    **注释里说做了，不等于代码里做了。** 一个只 grep 全文的源码判据，
    学生写一行 `// sync_all_blocks` 就能骗过去。
    """
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if c == '"':  # 字符串字面量：原样保留，里面的 // 不算注释
            out.append(c)
            i += 1
            while i < n:
                out.append(src[i])
                if src[i] == "\\":
                    i += 2
                    if i - 1 < n:
                        out.append(src[i - 1])
                    continue
                if src[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        if src.startswith("//", i):
            while i < n and src[i] != "\n":
                i += 1
            continue
        if src.startswith("/*", i):
            depth, i = 1, i + 2  # Rust 的块注释可以嵌套
            while i < n and depth:
                if src.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif src.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
            continue
        out.append(c)
        i += 1
    return "".join(out)


def check_source(spec, runner, meta):
    """源码模式检查。

    主要用途不是判对错，而是**堵住绕过判题的路**：把 `todo!()` 换成
    硬编码的答案、把测试标成 `#[ignore]`、干脆删掉断言。

    注释一律不算数，见 [`strip_comments`]。
    """
    fails = []
    files = []
    for pat in spec.get("files", ["os/src/**/*.rs"]):
        files += sorted(ROOT.glob(pat))
    if not files:
        return [f"没有匹配到任何文件：{spec.get('files')}"], ""
    code = {f: strip_comments(f.read_text(encoding="utf-8", errors="replace")) for f in files}
    blob = "\n".join(code.values())
    for s in spec.get("must", []):
        if s not in blob:
            fails.append(f"源码里找不到 {s!r}（注释里出现不算）")
    for s in spec.get("must_not", []):
        if s in blob:
            hits = [f.name for f, c in code.items() if s in c]
            fails.append(f"源码里不该出现 {s!r}（见 {', '.join(hits[:3])}）")
    return fails, f"检查了 {len(files)} 个文件（已剔除注释）"


CHECKS = {
    "build": check_build,
    "unit": check_unit,
    "selfcheck": check_selfcheck,
    "run": check_run,
    "source": check_source,
}


# ---------------------------------------------------------------------------
# 清单
# ---------------------------------------------------------------------------


def load(path=None):
    path = Path(path) if path else ROOT / "docs" / "checkpoints.toml"
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    meta = data.get("meta", {})
    chapters = sorted(data.get("chapter", []), key=lambda c: c["n"])
    return meta, chapters


def run_chapter(chapter, runner, meta):
    results = []
    for spec in chapter.get("check", []):
        fn = CHECKS.get(spec["type"])
        if fn is None:
            results.append(
                Result(spec.get("name", "?"), spec["type"], 0, 0, ["未知的判据类型"])
            )
            continue
        pts = spec.get("points", 0)
        try:
            fails, log = fn(spec, runner, meta)
        except Exception as e:  # 判据自己炸了，要报出来而不是当成学生错
            fails, log = [f"判据执行出错：{e}"], ""
        results.append(
            Result(spec.get("name", spec["type"]), spec["type"], pts, 0 if fails else pts, fails, log)
        )
    return results

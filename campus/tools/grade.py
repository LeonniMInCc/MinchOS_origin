#!/usr/bin/env python3
"""按 docs/checkpoints.toml 判题。

    python3 tools/grade.py [--ch N] [--json] [-v] -- <qemu 命令...>

通常通过 Makefile 调用：

    make regress        全部章节（= 拿参考实现验证判据本身没坏）
    make grade CH=6     只判第 6 章
    make grade FORMAT=json

# 参考实现必须拿满分

这不是自夸，是**判题器自己的测试**。判据写错了、写成了永远为真的废话、
或者引用了一句已经不存在的输出 —— 都会在这里当场暴露。

所以 `make regress` 不及格时，先怀疑判据，再怀疑内核。
"""

import argparse
import json
import re
import sys
from pathlib import Path

import checker

G, R, Y, DIM, OFF = "\033[32m", "\033[31m", "\033[33m", "\033[2m", "\033[0m"


def persistence_check(runner):
    """跨重启的持久化：写 → 关机 → 重启 → 读，内容必须原样还在。

    单独拎出来，因为它是清单里唯一一条**跨两次启动**的判据，
    表达不进那套 must / must_not 里。

    它也是第 16 周抓到那个 bug 的判据 —— 块缓存不同步的话，
    文件还在但内容是空的。而且"同一次运行里读得到"完全不能说明问题，
    那是缓存命中。
    """
    out1, t1 = runner.boot(["17write"])
    # 第二次启动必须是**新的一次** boot，不能吃缓存
    key = ("16cat", "__persist__")
    if key not in runner._cache:
        runner._cache[key] = runner._boot(["16cat"])
    out2, t2 = runner._cache[key]

    fails = checker.assert_output(out1, t1, ["write: 往"], ["[KERNEL PANIC]"])
    fails += checker.assert_output(out2, t2, ["cat: 共"], ["打不开", "[KERNEL PANIC]"])
    stamp = re.search(r"时间戳 (\d+) ms", out1)
    if not stamp:
        fails.append("第一次启动没打出时间戳，无法验证是同一份内容")
    elif stamp.group(1) not in out2:
        fails.append(f"重启后读到的不是上次写的内容（时间戳 {stamp.group(1)} 对不上）")
    return checker.Result(
        "持久化：重启后文件内容原样还在", "persist", 15, 0 if fails else 15, fails, out2[-2000:]
    )


def main():
    ap = argparse.ArgumentParser(add_help=True)
    ap.add_argument("--ch", type=int, help="只判某一章")
    ap.add_argument("--json", action="store_true", help="输出机器可读的成绩单")
    # 不写 stdout：make 的构建输出也在 stdout 上，`make grade > score.json`
    # 会把编译日志一起吞进去，JSON 当场坏掉。这是实测踩到的。
    ap.add_argument("--out", help="成绩单写到文件（默认 stdout）")
    ap.add_argument("-v", "--verbose", action="store_true", help="失败时打印输出")
    ap.add_argument("qemu", nargs=argparse.REMAINDER)
    a = ap.parse_args()

    qemu = [x for x in a.qemu if x != "--"]
    if not qemu:
        print("缺少 QEMU 命令。通常应该用 make grade 调用。", file=sys.stderr)
        return 2

    meta, chapters = checker.load()
    if a.ch:
        chapters = [c for c in chapters if c["n"] == a.ch]
        if not chapters:
            print(f"清单里没有第 {a.ch} 章", file=sys.stderr)
            return 2

    runner = checker.Runner(qemu, verbose=a.verbose and not a.json)
    report, total, earned, failed = [], 0, 0, 0

    if not a.json:
        print(f"\nCCNUOS 判题　{len(chapters)} 章\n")

    for ch in chapters:
        results = checker.run_chapter(ch, runner, meta)
        # 第 16 章额外挂一条跨重启的判据
        if ch["n"] == 16:
            results.append(persistence_check(runner))

        p = sum(r.points for r in results)
        e = sum(r.earned for r in results)
        total += p
        earned += e
        # 不能只看分数：门槛判据（points = 0）失败时分数一点不掉，
        # 但它恰恰是"有没有作弊"这种最该拦住的事。变异测试当场抓到过：
        # 源码里留一个 todo!()，判据红了，总分还是 100%。
        failed += sum(1 for r in results if not r.ok)
        report.append(
            {
                "chapter": ch["n"],
                "name": ch["name"],
                "tag": ch.get("tag", ""),
                "points": p,
                "earned": e,
                "checks": [
                    {"name": r.name, "type": r.kind, "points": r.points,
                     "earned": r.earned, "fails": r.fails}
                    for r in results
                ],
            }
        )

        if a.json:
            continue
        bad = any(not r.ok for r in results)
        head = f"{R}✗{OFF}" if bad else f"{G}✓{OFF}"
        print(f"{head} 第 {ch['n']:>2} 章　{ch['name']}　{DIM}{e}/{p}{OFF}")
        for r in results:
            mark = f"{G}通过{OFF}" if r.ok else f"{R}失败{OFF}"
            pts = f"{DIM}{r.earned}/{r.points}{OFF}" if r.points else f"{DIM}门槛{OFF}"
            print(f"    {mark}  {r.name}  {pts}")
            for f in r.fails:
                print(f"          {Y}{f}{OFF}")
            if r.fails and a.verbose and r.log:
                for line in r.log.splitlines()[-25:]:
                    print(f"          {DIM}| {line}{OFF}")

    if a.json:
        blob = json.dumps(
            {"total": total, "earned": earned, "failed_checks": failed,
             "pass": failed == 0, "chapters": report},
            ensure_ascii=False, indent=2,
        )
        if a.out:
            Path(a.out).write_text(blob + "\n", encoding="utf-8")
            print(f"成绩单已写入 {a.out}：{earned}/{total}，"
                  f"{'通过' if failed == 0 else f'{failed} 条判据未通过'}")
        else:
            print(blob)
    else:
        pct = earned / total * 100 if total else 0
        color = G if failed == 0 else (Y if pct >= 60 else R)
        print(f"\n总分：{color}{earned}/{total}{OFF}（{pct:.0f}%）"
              f"　{DIM}启动 QEMU {runner.boots} 次{OFF}")
        if failed:
            # 门槛判据不值分，所以"满分"和"通过"是两件事，必须分开说
            print(f"{R}{failed} 条判据未通过{OFF}"
                  + (f"{Y}（其中有不计分的门槛判据）{OFF}" if earned == total else ""))
            print(f"{DIM}加 V=1 看失败判据的实际输出{OFF}")

    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())

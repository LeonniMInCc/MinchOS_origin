#!/usr/bin/env python3
"""CCNUOS 回归测试。

    用法: python3 tools/regress.py <qemu 命令...>
          （通常通过 make regress 调用）

# 它检查什么

每个用例是"一串喂给 shell 的命令 + 一组对输出的断言"。断言分两种：

- **must**：必须出现的字符串。程序干对了才会打出来。
- **must_not**：绝不能出现的字符串。

`must_not` 是这套测试里更值钱的一半。用户程序里那些
"如果你看到这行，说明内核没拦住我 —— 那是个严重问题"
就是为此写的：**程序自己知道什么是失败，并且会把失败喊出来。**
测试只要盯着那句话有没有出现就行，不必去理解它在干什么。

另外有三条**全局**断言，对每一次启动都成立（死锁用例除外）：

1. 不出现 `[KERNEL PANIC]`
2. 出现"无泄漏"——整条 RAII 链的端到端检验
3. 不出现"串口输入缓冲区溢出"

# 为什么分成好几次启动而不是一次跑完

因为 `exit` 会关机，而死锁用例更是会让内核以错误码关机。
分组还有个好处：一组挂了，其他组的结果仍然有效，
不至于一个失败就什么信息都拿不到。
"""

import re
import subprocess
import sys
import time

# ---------------------------------------------------------------------------
# 用例
# ---------------------------------------------------------------------------

# (名字, 喂给 shell 的命令, 必须出现, 绝不能出现)
CASES = [
    (
        "用户态与系统调用",
        ["00hello_world"],
        ["你好，我是跑在 U 态的第一个用户程序。", "现在我要 exit(0) 了。"],
        [],
    ),
    (
        "协作式调度：三个任务交替",
        ["01yield_a", "02yield_b", "03yield_c"],
        ["[A] 四轮跑完", "[B] 四轮跑完", "[C] 四轮跑完", "隔离完好"],
        [],
    ),
    (
        "抢占式调度：不让出也会被切走",
        ["06timer"],
        ["[计时]"],
        [],
    ),
    # 这两个从头到尾一次都不调 yield。跑得完就说明时钟中断确实在抢占 ——
    # 否则第一个就会一直占着 CPU，第二个永远轮不上，整轮直接超时。
    (
        "纯计算程序不会饿死系统",
        ["04spin_x", "05spin_y"],
        ["[X] 全部算完", "[Y] 全部算完"],
        [],
    ),
    # 下面四个是"坏"程序。它们故意越界，内核必须拦住并只杀掉它们自己。
    # 这一组最能说明问题：四个进程被杀，shell 和内核毫发无损。
    (
        "内存隔离：非法系统调用参数",
        ["07bad_syscall_arg"],
        ["被内核拒绝了"],
        ["内核居然照做了"],
    ),
    (
        "特权隔离：用户态执行 sret",
        ["08bad_instruction"],
        [],
        ["说明内核没拦住我"],
    ),
    (
        "地址隔离：写空指针",
        ["09bad_address"],
        [],
        ["说明写成功了"],
    ),
    (
        "内存隔离：写内核代码段",
        ["10bad_kernel_write"],
        [],
        ["说明内存隔离没生效"],
    ),
    (
        "fork：地址空间真的是独立的",
        ["11forktest"],
        ["[fork] 父进程回收了 pid"],
        [],
    ),
    (
        "管道：单条与三级流水线",
        ["15pipetest", "12words | 13upper | 14wc"],
        ["[管道·子] 读到 EOF", "行，", "字节"],
        [],
    ),
    (
        "文件系统：写、读、列目录、重定向",
        ["17write", "16cat", "18ls", "12words > regress.txt", "16cat regress.txt"],
        ["write: 往", "cat: 共", "共 ", "个文件"],
        ["打不开"],
    ),
    (
        "同步：哲学家不死锁",
        ["19philosophers"],
        ["位哲学家都吃完了，没有人饿死"],
        ["居然吃上了"],
    ),
    (
        "同步：信号量强制顺序",
        ["20semaphore"],
        ["顺序执行完毕"],
        [],
    ),
    (
        "同步：条件变量广播唤醒",
        ["21condvar"],
        ["个工人全部完成"],
        [],
    ),
]

# 死锁用例单独处理：它**期望**内核报错关机，所以全局断言不适用。
DEADLOCK_CASE = (
    "死锁必须被发现，而不是伪装成正常关机",
    ["22deadlock"],
    ["死锁：就绪队列已空", "故跳过页帧核对"],
    ["就绪队列已空，全部进程执行完毕", "居然吃上了"],
)

# 跨重启的持久化：两次启动，第二次要能读到第一次写的内容。
# 这个用例的价值在于它**只有真的落盘了才会过** —— 块缓存里的数据
# 活不过关机。
PERSIST_WRITE = ["17write"]
PERSIST_READ = ["16cat"]

GLOBAL_MUST = ["无泄漏"]
GLOBAL_MUST_NOT = ["[KERNEL PANIC]", "串口输入缓冲区溢出"]

ANSI = re.compile(r"\x1b\[[0-9;]*m")
BOOT_WAIT = 2.5
TIMEOUT = 90


def run(qemu_cmd, commands, timeout=TIMEOUT):
    """启动一次 QEMU，喂进命令，返回去掉颜色的完整输出。"""
    script = ("\n".join(commands) + "\nexit\n").encode()
    proc = subprocess.Popen(
        qemu_cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    # 等内核启动完再喂。太早的话字符会落在固件初始化串口之前，
    # 那段时间敲的东西真机上也一样会丢。
    time.sleep(BOOT_WAIT)
    try:
        out, _ = proc.communicate(script, timeout=timeout)
    except subprocess.TimeoutExpired:
        proc.kill()
        out, _ = proc.communicate()
        return ANSI.sub("", out.decode("utf-8", "replace")), True
    return ANSI.sub("", out.decode("utf-8", "replace")), False


def check(out, timed_out, must, must_not):
    """返回失败原因的列表。空列表表示通过。"""
    fails = []
    if timed_out:
        fails.append(f"超时（{TIMEOUT}s 未退出）")
    for s in must:
        if s not in out:
            fails.append(f"缺少应有的输出：{s!r}")
    for s in must_not:
        if s in out:
            fails.append(f"出现了不该出现的输出：{s!r}")
    return fails


def report(name, fails, out, verbose):
    if not fails:
        print(f"  \033[32m通过\033[0m  {name}")
        return True
    print(f"  \033[31m失败\033[0m  {name}")
    for f in fails:
        print(f"          {f}")
    if verbose:
        print("        ---- 输出 ----")
        for line in out.splitlines()[-40:]:
            print(f"        | {line}")
    return False


def main():
    argv = sys.argv[1:]
    verbose = "-v" in argv
    qemu_cmd = [a for a in argv if a != "-v"]
    if not qemu_cmd:
        print(__doc__.strip(), file=sys.stderr)
        return 2

    print(f"CCNUOS 回归测试　共 {len(CASES) + 2} 个用例，覆盖全部 23 个演示程序\n")
    passed = failed = 0

    print("普通用例（每个都另外检查：无 panic、无页帧泄漏、串口无溢出）")
    for name, cmds, must, must_not in CASES:
        out, timed_out = run(qemu_cmd, cmds)
        fails = check(out, timed_out, must + GLOBAL_MUST, must_not + GLOBAL_MUST_NOT)
        if report(name, fails, out, verbose):
            passed += 1
        else:
            failed += 1

    print("\n特殊用例")
    name, cmds, must, must_not = DEADLOCK_CASE
    out, timed_out = run(qemu_cmd, cmds)
    fails = check(out, timed_out, must, must_not + ["[KERNEL PANIC]"])
    if report(name, fails, out, verbose):
        passed += 1
    else:
        failed += 1

    # 持久化要两次启动：写 → 关机 → 重启 → 读
    out1, t1 = run(qemu_cmd, PERSIST_WRITE)
    out2, t2 = run(qemu_cmd, PERSIST_READ)
    fails = check(out1, t1, ["write: 往"] + GLOBAL_MUST, GLOBAL_MUST_NOT)
    fails += check(out2, t2, ["cat: 共"] + GLOBAL_MUST, ["打不开"] + GLOBAL_MUST_NOT)
    # 光能读到还不够 —— 得确认读到的就是上一次写进去的那份
    stamp = re.search(r"时间戳 (\d+) ms", out1)
    if stamp and stamp.group(1) not in out2:
        fails.append(f"重启后读到的内容不是上次写的（时间戳 {stamp.group(1)} 对不上）")
    elif not stamp:
        fails.append("第一次启动没打出时间戳，无法验证是同一份内容")
    if report("持久化：重启后文件内容原样还在", fails, out1 + out2, verbose):
        passed += 1
    else:
        failed += 1

    total = passed + failed
    print(f"\n结果：{passed}/{total} 通过", end="")
    print("" if failed == 0 else f"，\033[31m{failed} 个失败\033[0m")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

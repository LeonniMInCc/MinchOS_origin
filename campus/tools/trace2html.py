#!/usr/bin/env python3
"""把 CCNUOS 的事件流渲染成一页可视化。

    用法: python3 tools/trace2html.py <事件日志> <输出 html>

内核在关键时刻打印一行结构化事件，形如：

    @EV sched t=123456 pid=2
    @EV sys   t=123460 pid=2 id=64
    @EV area  t=123470 pid=2 vpn=0x10 pages=2 ppn=0x80a12 perm=R-XU

这个脚本把它们变成四张图：调度甘特图、系统调用时序、进程树、地址空间对照。

**内核不知道图长什么样，这个脚本不知道内核怎么实现。**
两边只通过那一行文本耦合 —— 于是可视化可以随便改，内核一行都不用动。
"""

import bisect
import html
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

# 事件可能紧跟在用户程序的输出后面（串口是共享的），所以按模式搜索，
# 而不是按行切分。事件本身是完整的 —— 内核不可抢占，
# 一条事件打印到一半不会被切走。
EVENT_RE = re.compile(r"@EV (\w+) t=(\d+)((?: \w+=\S+)*)")

# 系统调用号 → 名字。和 os/src/syscall/mod.rs 保持一致。
SYSCALL_NAMES = {
    24: "dup", 56: "open", 57: "close", 59: "pipe", 63: "read", 64: "write",
    93: "exit", 124: "yield", 169: "get_time", 172: "getpid",
    220: "fork", 221: "exec", 260: "waitpid",
    1000: "listdir",
    1010: "mutex_create", 1011: "mutex_lock", 1012: "mutex_unlock",
    1020: "sem_create", 1021: "sem_down", 1022: "sem_up",
    1030: "condvar_create", 1031: "condvar_wait", 1032: "condvar_signal",
    1033: "condvar_broadcast",
}

PALETTE = [
    "#6ba6ff", "#6bcf7f", "#ffd166", "#ff6b6b", "#d4bfff",
    "#5ccfe6", "#e6b450", "#f78fb3", "#9ae6b4", "#c3a6ff",
]


def color(pid):
    return PALETTE[pid % len(PALETTE)]


def esc(s):
    return html.escape(str(s))


# ---------------------------------------------------------------------------
# 解析
# ---------------------------------------------------------------------------


def parse(text):
    """把日志里的事件抽出来，按时间排序。"""
    events = []
    for kind, t, rest in EVENT_RE.findall(text):
        fields = {}
        for pair in rest.split():
            key, _, value = pair.partition("=")
            try:
                fields[key] = int(value, 0)
            except ValueError:
                fields[key] = value
        events.append((int(t), kind, fields))
    events.sort(key=lambda e: e[0])
    return events


def build_slices(events):
    """从事件推出"每个进程从什么时候跑到什么时候"。

    # 这里有个必须处理的观测缺口

    内核只在**被调度上来**时报告一次，从不报告"我交出了 CPU"。
    天真的做法是"一个区间从 sched 一直画到下一个 sched" —— 但那是错的：
    进程阻塞之后，如果没有别人就绪，CPU 会停在 idle 的 `wfi` 上，
    这期间**没有任何 sched 事件**。于是那段空闲会被算在阻塞的进程头上，
    甘特图上出现一条几秒长的假运行区间（第一版就是这么画错的）。

    所以区间的结束时刻取两者中**先到的那个**：
    下一次 `sched`，或者这个进程自己的 `block` / `exit`。
    后两者恰好就是"它停止运行"的时刻。

    返回 (运行区间, 空闲区间)。
    """
    # 每个 pid 停止运行的时刻表，排好序供二分查找
    stop_times = defaultdict(list)
    for t, kind, f in events:
        if kind in ("block", "exit") and "pid" in f:
            stop_times[f["pid"]].append(t)

    runs = []
    scheds = [(t, f["pid"]) for t, kind, f in events if kind == "sched"]
    for i, (start, pid) in enumerate(scheds):
        limit = scheds[i + 1][0] if i + 1 < len(scheds) else events[-1][0]
        stops = stop_times[pid]
        j = bisect.bisect_right(stops, start)
        if j < len(stops):
            limit = min(limit, stops[j])
        if limit > start:
            runs.append((pid, start, limit))

    # 空闲 = 观测窗口里没有任何进程在跑的那些段
    idle, cursor = [], None
    for _, a, b in runs:
        if cursor is not None and a > cursor:
            idle.append((cursor, a))
        cursor = b if cursor is None else max(cursor, b)
    return runs, idle


# ---------------------------------------------------------------------------
# 时间轴：把长时间空闲折叠掉
# ---------------------------------------------------------------------------

GAP_THRESHOLD_US = 15_000  # 超过 15 ms 的空闲就折叠
GAP_DRAW_PX = 26           # 折叠后画多宽


class Axis:
    """分段线性的时间轴。

    一次交互式运行里，绝大部分时间 CPU 都闲着等你敲键盘 ——
    实测 2.9 s 的窗口里有 2.8 s 是空闲。按真实比例画的话，
    所有活动会挤成一根看不见的竖线。

    所以长空闲段被折叠成固定宽度的窄条，并**在图上标出折叠了多久**。
    这是性能剖析工具的常规做法：压缩可以，但不能不说。
    """

    def __init__(self, runs, idle, plot_w, left):
        t0 = min(a for _, a, _ in runs)
        t1 = max(b for _, _, b in runs)
        self.folded = [(a, b) for a, b in idle if b - a > GAP_THRESHOLD_US]

        # 把整条时间轴切成"正常段"和"折叠段"
        pieces, cursor = [], t0
        for a, b in self.folded:
            if a > cursor:
                pieces.append((cursor, a, False))
            pieces.append((a, b, True))
            cursor = b
        if cursor < t1:
            pieces.append((cursor, t1, False))
        if not pieces:
            pieces = [(t0, max(t1, t0 + 1), False)]

        real = sum(b - a for a, b, fold in pieces if not fold)
        scale_w = plot_w - GAP_DRAW_PX * len(self.folded)
        us_per_px = max(real, 1) / max(scale_w, 1)

        self.pieces, x = [], left
        for a, b, fold in pieces:
            w = GAP_DRAW_PX if fold else (b - a) / us_per_px
            self.pieces.append((a, b, x, w, fold))
            x += w
        self.starts = [p[0] for p in self.pieces]
        self.t0, self.t1, self.left, self.right = t0, t1, left, x

    def map(self, t):
        i = max(bisect.bisect_right(self.starts, t) - 1, 0)
        a, b, x, w, fold = self.pieces[i]
        if fold:
            return x + w / 2
        if b <= a:
            return x
        return x + (min(t, b) - a) / (b - a) * w


FOLD_PATTERN = (
    '<defs><pattern id="fold" width="6" height="6" patternTransform="rotate(45)" '
    'patternUnits="userSpaceOnUse">'
    '<line x1="0" y1="0" x2="0" y2="6" stroke="#3a4050" stroke-width="2"/>'
    "</pattern></defs>"
)


# ---------------------------------------------------------------------------
# 图一：调度甘特图
# ---------------------------------------------------------------------------


def gantt_svg(runs, idle, events, width=980, row_h=24):
    pids = sorted({pid for pid, _, _ in runs})
    row = {pid: i for i, pid in enumerate(pids)}
    left = 62
    axis = Axis(runs, idle, width - left - 12, left)
    top = 8
    plot_h = len(pids) * row_h
    height = plot_h + 44

    p = [f'<svg viewBox="0 0 {width} {height}" xmlns="http://www.w3.org/2000/svg" '
         f'font-family="SF Mono, Menlo, monospace" font-size="11">', FOLD_PATTERN]

    # 折叠段：斜纹铺底，让"这里时间被跳过了"一目了然
    for a, b, x, w, fold in axis.pieces:
        if not fold:
            continue
        p.append(f'<rect x="{x:.1f}" y="{top}" width="{w:.1f}" height="{plot_h}" '
                 f'fill="url(#fold)"><title>跳过 {(b - a) / 1000:.0f} ms 空闲</title></rect>')
        p.append(f'<text x="{x + w / 2:.1f}" y="{top + plot_h + 14}" fill="#8a8f98" '
                 f'text-anchor="middle" font-size="9">≈{(b - a) / 1000:.0f}ms</text>')

    for pid in pids:
        y = row[pid] * row_h + top
        p.append(f'<rect x="{left}" y="{y}" width="{axis.right - left:.1f}" '
                 f'height="{row_h - 5}" fill="#1b1e24" rx="2"/>')
        p.append(f'<text x="{left - 8}" y="{y + (row_h - 5) / 2}" fill="#8a8f98" '
                 f'text-anchor="end" dominant-baseline="middle">pid {pid}</text>')

    # 运行区间。很短的区间按最小 1.2px 画 —— 宁可看得见，
    # 也不要因为不足一个像素就整个消失。
    for pid, a, b in runs:
        y = row[pid] * row_h + top
        x0, x1 = axis.map(a), axis.map(b)
        p.append(
            f'<rect x="{x0:.1f}" y="{y}" width="{max(x1 - x0, 1.2):.1f}" '
            f'height="{row_h - 5}" fill="{color(pid)}" rx="1.5">'
            f'<title>pid {pid} 运行 {(b - a) / 1000:.3f} ms</title></rect>'
        )

    # 阻塞点用空心圈标出 —— 区分"被抢占"和"主动睡下"
    for t, kind, f in events:
        if kind != "block" or f.get("pid") not in row:
            continue
        y = row[f["pid"]] * row_h + top + (row_h - 5) / 2
        p.append(f'<circle cx="{axis.map(t):.1f}" cy="{y:.1f}" r="3.4" fill="#14161a" '
                 f'stroke="#ff6b6b" stroke-width="1.5">'
                 f'<title>pid {f["pid"]} 在此阻塞</title></circle>')

    base = top + plot_h + 2
    p.append(f'<line x1="{left}" y1="{base}" x2="{axis.right:.1f}" y2="{base}" stroke="#2b3038"/>')
    # 刻度只打在未折叠的段上 —— 折叠段里时间不是线性的，标刻度会骗人
    for a, b, x, w, fold in axis.pieces:
        if fold:
            continue
        for t in (a, b):
            p.append(f'<line x1="{axis.map(t):.1f}" y1="{base}" '
                     f'x2="{axis.map(t):.1f}" y2="{base + 4}" stroke="#2b3038"/>')
        p.append(f'<text x="{x + w / 2:.1f}" y="{base + 26}" fill="#8a8f98" '
                 f'text-anchor="middle" font-size="9">'
                 f'{(a - axis.t0) / 1000:.0f}–{(b - axis.t0) / 1000:.0f} ms</text>')

    p.append("</svg>")
    return "".join(p), axis


# ---------------------------------------------------------------------------
# 图二：系统调用时序
# ---------------------------------------------------------------------------


def syscall_timeline(events, axis, width=980, height=48):
    """每次系统调用一根竖线，按发起它的进程上色。

    甘特图回答"谁在跑"，这张图回答"它跑的时候在干什么"。
    """
    calls = [(t, f["pid"], f["id"]) for t, kind, f in events if kind == "sys"]
    if not calls:
        return "<p>没有系统调用事件</p>"

    p = [f'<svg viewBox="0 0 {width} {height}" xmlns="http://www.w3.org/2000/svg" '
         f'font-family="SF Mono, Menlo, monospace" font-size="11">', FOLD_PATTERN]
    p.append(f'<rect x="{axis.left}" y="8" width="{axis.right - axis.left:.1f}" '
             f'height="32" fill="#1b1e24" rx="2"/>')
    for a, b, x, w, fold in axis.pieces:
        if fold:
            p.append(f'<rect x="{x:.1f}" y="8" width="{w:.1f}" height="32" fill="url(#fold)"/>')
    for t, pid, sid in calls:
        name = SYSCALL_NAMES.get(sid, f"#{sid}")
        p.append(f'<line x1="{axis.map(t):.1f}" y1="10" x2="{axis.map(t):.1f}" y2="38" '
                 f'stroke="{color(pid)}" stroke-width="1.1" opacity="0.8">'
                 f'<title>pid {pid}　{name}　t={(t - axis.t0) / 1000:.3f} ms</title></line>')
    p.append(f'<text x="{axis.left - 8}" y="25" fill="#8a8f98" text-anchor="end" '
             f'dominant-baseline="middle">syscall</text>')
    p.append("</svg>")
    return "".join(p)


def syscall_table(events):
    counts = Counter()
    for _, kind, f in events:
        if kind == "sys":
            counts[SYSCALL_NAMES.get(f["id"], f"#{f['id']}")] += 1
    if not counts:
        return ""
    top = counts.most_common()
    peak = top[0][1]
    rows = "".join(
        f'<tr><td class="name">{esc(name)}</td>'
        f'<td class="bar"><i style="width:{n / peak * 100:.1f}%"></i></td>'
        f'<td class="num">{n}</td></tr>'
        for name, n in top
    )
    return f'<div class="panel"><table class="syscalls">{rows}</table></div>'


# ---------------------------------------------------------------------------
# 图三：进程树
# ---------------------------------------------------------------------------


def process_tree(events):
    """从 new / exit 事件重建进程树。

    # PID 会被回收，所以它不能当节点的身份

    第一版直接拿 pid 当树的节点，画出来 `pid 2` 在同一层出现了两次 ——
    因为 `11forktest` 退出后，`19philosophers` 拿到了同一个号。
    PID 是**当前存活进程**的唯一标识，不是进程的历史标识。

    所以每个 `new` 事件都建一个独立节点，父节点取"此刻还活着的、
    pid 等于 ppid 的那个节点"。号码被复用的地方会显式标出来 ——
    这件事本身值得看见。
    """
    nodes, live, roots = [], {}, []
    for t, kind, f in events:
        if kind == "new":
            pid, ppid = f["pid"], f["ppid"]
            idx = len(nodes)
            nodes.append({"pid": pid, "born": t, "kids": [], "code": None})
            parent = live.get(ppid)
            # ppid == pid 是 initproc 的约定；父节点找不到时也当根，
            # 免得日志被截断时整棵树消失
            if ppid == pid or parent is None:
                roots.append(idx)
            else:
                nodes[parent]["kids"].append(idx)
            live[pid] = idx
        elif kind == "exit":
            idx = live.pop(f["pid"], None)
            if idx is not None:
                nodes[idx]["code"] = f.get("code", 0)

    if not nodes:
        return "<p>没有进程事件</p>"

    t0 = min(n["born"] for n in nodes)
    reuse = Counter()
    lines = []

    def label(idx, note=""):
        n = nodes[idx]
        pid = n["pid"]
        reuse[pid] += 1
        again = (f'　<span class="bad">第 {reuse[pid]} 次用到这个号</span>'
                 if reuse[pid] > 1 else "")
        if n["code"] is None:
            tail = '　<span class="muted">未退出</span>'
        else:
            cls = "ok" if n["code"] == 0 else "bad"
            tail = f'　<span class="{cls}">退出码 {n["code"]}</span>'
        age = f'　<span class="muted">生于 {(n["born"] - t0) / 1000:.0f} ms</span>'
        return f'<span style="color:{color(pid)}">●</span> pid {pid}{note}{again}{age}{tail}'

    def walk(idx, prefix, last):
        lines.append(prefix + ("└─ " if last else "├─ ") + label(idx))
        kids = nodes[idx]["kids"]
        for i, kid in enumerate(kids):
            walk(kid, prefix + ("   " if last else "│  "), i == len(kids) - 1)

    for r in roots:
        lines.append(label(r, " (initproc)"))
        kids = nodes[r]["kids"]
        for i, kid in enumerate(kids):
            walk(kid, "", i == len(kids) - 1)

    return "<pre class='tree'>" + "\n".join(lines) + "</pre>"


# ---------------------------------------------------------------------------
# 图四：地址空间对照
# ---------------------------------------------------------------------------

PAGE = 4096
SV39_VPN_BITS = 27
# 内核报的是地址和权限，不是名字 —— **起名是展示层的事**，
# 内核不该关心。这里按地址和权限反推。
NAMED_VA = {
    0xFFFFFFFFFFFFF000: "跳板页 trampoline",
    0xFFFFFFFFFFFFE000: "TrapContext",
}


def va_of(vpn):
    """虚拟页号 → 完整的 64 位虚拟地址。

    内核报出来的 `vpn` 是 SV39 的 27 位页号，`vpn << 12` 只有 39 位。
    而 SV39 要求高 25 位必须是第 38 位的**符号扩展** —— 所以
    `0x7fffffe` 这个页号对应的真实地址是 `0xffff_ffff_ffff_e000`，
    不是 `0x7f_ffff_e000`。不做这一步，跳板页和 TrapContext
    就会显示成两个莫名其妙的低地址。
    """
    va = vpn << 12
    if va >> 38 & 1:
        va -= 1 << 39
    return va & 0xFFFFFFFFFFFFFFFF


def area_name(area, stack_vpn):
    va = va_of(area["vpn"])
    if va in NAMED_VA:
        return NAMED_VA[va]
    perm = str(area["perm"])
    if "U" not in perm:
        return "内核段"
    if area["vpn"] == stack_vpn:
        return "用户栈"
    if "X" in perm:
        return ".text　代码"
    if "W" in perm:
        return ".data / .bss"
    return ".rodata　只读数据"


MAX_SPACES_SHOWN = 6


def address_spaces(events):
    """每个进程的地址空间：一样的虚拟页，落在不一样的物理页上。

    一个 pid 会**报告多次** —— 每次 `exec` 换掉地址空间就重报一轮。
    所以要按"一串连续的 area 事件"分组，只留每个 pid 最后那一组，
    否则 shell 的三代地址空间会被拼成一张互相矛盾的表。
    """
    spaces = {}
    prev_pid = None
    for _, kind, f in events:
        if kind != "area":
            prev_pid = None  # 中间夹了别的事件，说明这一轮报完了
            continue
        if f["pid"] != prev_pid:
            spaces[f["pid"]] = []
            prev_pid = f["pid"]
        spaces[f["pid"]].append(f)

    if not spaces:
        return ('<p class="muted">日志里没有 area 事件 —— '
                "这些事件只在进程创建和 exec 时输出，检查一下捕获是否完整。</p>")

    shown = sorted(spaces)[:MAX_SPACES_SHOWN]
    blocks = []
    for pid in shown:
        areas = sorted(spaces[pid], key=lambda a: a["vpn"])
        # 低地址里最后一段就是用户栈（ELF 各段之后、跳板页之前）
        low = [a for a in areas if va_of(a["vpn"]) < 0x1000000]
        stack_vpn = low[-1]["vpn"] if low else None

        rows, prev_end = [], None
        for a in areas:
            # 相邻段之间正好空一页，那就是 guard page
            if prev_end is not None and 0 < a["vpn"] - prev_end <= 2:
                hole = a["vpn"] - prev_end
                tag = ("guard page —— 不映射，栈写穿立刻缺页"
                       if hole == 1 else f"未映射空洞 {hole} 页")
                rows.append(f'<tr class="hole"><td colspan="4">{esc(tag)}</td></tr>')
            prev_end = a["vpn"] + a["pages"]

            perm_html = "".join(
                f"<b>{c}</b>" if c != "-" else "<i>-</i>" for c in str(a["perm"])
            )
            rows.append(
                f'<tr><td class="va">{va_of(a["vpn"]):#018x}</td>'
                f'<td class="pg">{a["pages"]}p</td>'
                f'<td class="perm">{perm_html}</td>'
                f'<td class="nm">{esc(area_name(a, stack_vpn))}'
                f'<span class="muted">　→ ppn {a["ppn"]:#x}</span></td></tr>'
            )
        blocks.append(
            f'<div class="space"><h3><span style="color:{color(pid)}">●</span> pid {pid}'
            f'<span class="muted">　{len(areas)} 段 / '
            f'{sum(a["pages"] for a in areas)} 页</span></h3>'
            f'<table class="areas">{"".join(rows)}</table></div>'
        )
    tail = ""
    if len(spaces) > len(shown):
        tail = (f'<p class="note">另有 {len(spaces) - len(shown)} 个进程的地址空间'
                f"没有列出，布局和上面同程序的那几个完全一致。</p>")
    return f'<div class="spaces">{"".join(blocks)}</div>{tail}'


# ---------------------------------------------------------------------------
# 概览
# ---------------------------------------------------------------------------


def summary(events, runs):
    c = Counter(kind for _, kind, _ in events)
    busy = sum(b - a for _, a, b in runs)
    window = max(b for _, _, b in runs) - min(a for _, a, _ in runs)
    cards = [
        ("进程", c.get("new", 0)),
        ("上下文切换", c.get("sched", 0)),
        ("系统调用", c.get("sys", 0)),
        ("阻塞 / 唤醒", f'{c.get("block", 0)} / {c.get("wake", 0)}'),
        ("CPU 忙", f"{busy / max(window, 1) * 100:.1f}%"),
        ("观测窗口", f"{window / 1000:.0f} ms"),
    ]
    return "".join(f'<div class="card"><b>{v}</b><span>{k}</span></div>' for k, v in cards)


TEMPLATE = """<!DOCTYPE html>
<html lang="zh-CN"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>CCNUOS 运行可视化</title>
<style>
  :root{--bg:#14161a;--panel:#1b1e24;--border:#2b3038;--fg:#d4d4d4;--muted:#8a8f98;--accent:#6ba6ff}
  *{box-sizing:border-box}
  body{margin:0;padding:30px 20px 60px;background:var(--bg);color:var(--fg);
       font-family:-apple-system,BlinkMacSystemFont,"PingFang SC","Segoe UI",sans-serif;
       display:flex;justify-content:center}
  .wrap{width:100%;max-width:1020px}
  h1{font-size:20px;margin:0 0 4px}
  h2{font-size:14px;margin:30px 0 10px}
  h3{font-size:12.5px;margin:0 0 8px;font-weight:600}
  .sub{color:var(--muted);font-size:13px;margin-bottom:18px}
  .note{color:var(--muted);font-size:12px;margin:8px 0 0;line-height:1.75}
  .note b{color:var(--fg);font-weight:600}
  .cards{display:flex;gap:9px;flex-wrap:wrap}
  .card{background:var(--panel);border:1px solid var(--border);border-radius:8px;
        padding:10px 15px;min-width:92px}
  .card b{display:block;font-size:18px;color:var(--accent);font-weight:600}
  .card span{font-size:11px;color:var(--muted)}
  .panel{background:var(--panel);border:1px solid var(--border);border-radius:10px;padding:14px}
  .panel.tight{padding:8px 14px;margin-bottom:10px}
  pre.tree{margin:0;font-family:"SF Mono",Menlo,monospace;font-size:12.5px;line-height:1.85}
  .ok{color:#6bcf7f} .bad{color:#ff6b6b} .muted{color:var(--muted)}
  table.syscalls{width:100%;border-collapse:collapse;font-size:12.5px}
  table.syscalls td{padding:3px 6px}
  td.name{font-family:"SF Mono",Menlo,monospace;width:148px}
  td.num{text-align:right;color:var(--muted);width:54px}
  td.bar i{display:block;height:9px;background:var(--accent);border-radius:2px;opacity:.72}
  .spaces{display:flex;gap:16px;flex-wrap:wrap}
  .space{flex:1 1 450px;min-width:420px}
  table.areas{width:100%;border-collapse:collapse;
              font-family:"SF Mono",Menlo,monospace;font-size:11.5px}
  table.areas td{padding:2.5px 5px;border-top:1px solid var(--border)}
  td.va{color:var(--accent)} td.pg{color:var(--muted);text-align:right;width:36px}
  td.perm{width:52px} td.perm b{color:#ffd166} td.perm i{color:#454b57;font-style:normal}
  tr.hole td{color:#ff6b6b;font-size:10.5px;text-align:center;background:#1f1416}
</style></head><body><div class="wrap">
<h1>CCNUOS 运行可视化</h1>
<div class="sub">由 <code>make trace</code> 生成 · 素材是内核打印的 __COUNT__ 条结构化事件</div>
<div class="cards">__SUMMARY__</div>

<h2>1　调度甘特图</h2>
<div class="panel tight">__GANTT__</div>
<p class="note">每行一个进程，色块是它<b>真正占着 CPU</b> 的时间；红圈是<b>阻塞</b>点 ——
那不是被抢占，是进程主动睡下等事件。斜纹区是<b>折叠掉的空闲</b>：交互式运行里 CPU
绝大部分时间在等人敲键盘，按真实比例画的话所有活动会挤成一根看不见的竖线。
折叠了多久都标在轴上，鼠标停上去有确切数字。</p>

<h2>2　系统调用时序</h2>
<div class="panel tight">__SYSTIME__</div>
__SYSTABLE__
<p class="note">上图每根竖线是一次系统调用，按发起它的进程上色，横轴和甘特图<b>完全对齐</b> ——
甘特图回答"谁在跑"，这张图回答"它跑的时候在干什么"。竖线密集处通常是在刷屏。
下面的柱状表列出全部种类，那就是 CCNUOS 对用户态暴露的<b>完整</b>接口面。</p>

<h2>3　进程树</h2>
<div class="panel">__TREE__</div>
<p class="note">除 <code>initproc</code> 之外<b>每一个</b>进程都是 <code>fork</code> 出来的。
shell 执行一条命令的动作就是：fork 一个自己，再让子进程 <code>exec</code> 成目标程序。</p>

<h2>4　地址空间</h2>
<div class="panel">__SPACES__</div>
<p class="note">同一个程序在不同进程里，虚拟地址<b>完全一样</b>，
而右边的物理页号<b>各不相同</b> —— 这就是分页在做的事。
另外两处：没有任何一段同时可写又可执行（<code>W</code> 和 <code>X</code> 从不并存）；
最高处的 <code>TrapContext</code> <b>不带 <code>U</code></b>，所以用户态碰不到它。
表里看不到跳板页，是因为它不是一个"逻辑段"——
它被直接写进页表，不归任何 <code>MapArea</code> 管，所以内核也就没有为它报告 area 事件。</p>
</div></body></html>
"""


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    src, dst = Path(sys.argv[1]), Path(sys.argv[2])
    if not src.exists():
        print(f"找不到 {src}", file=sys.stderr)
        return 1

    events = parse(src.read_text(encoding="utf-8", errors="replace"))
    if not events:
        print(f"{src} 里没有事件。是不是忘了用 TRACE=1 构建？", file=sys.stderr)
        return 1

    runs, idle = build_slices(events)
    if not runs:
        print(f"{src} 里没有调度事件，画不出甘特图。", file=sys.stderr)
        return 1

    gantt, axis = gantt_svg(runs, idle, events)
    page = (TEMPLATE
            .replace("__COUNT__", str(len(events)))
            .replace("__SUMMARY__", summary(events, runs))
            .replace("__GANTT__", gantt)
            .replace("__SYSTIME__", syscall_timeline(events, axis))
            .replace("__SYSTABLE__", syscall_table(events))
            .replace("__TREE__", process_tree(events))
            .replace("__SPACES__", address_spaces(events)))

    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_text(page, encoding="utf-8")
    folded = sum(b - a for a, b in axis.folded)
    print(f"[trace2html] {src} → {dst}")
    print(f"             {len(events)} 条事件，{len(runs)} 个运行区间，"
          f"折叠了 {folded / 1000:.0f} ms 空闲")
    return 0


if __name__ == "__main__":
    sys.exit(main())

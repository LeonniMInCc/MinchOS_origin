#!/usr/bin/env python3
"""把 CCNUOS 的串口日志渲染成一个可回放的单文件 HTML。

    用法: python3 tools/ansi2html.py <输入日志> <输出 html>

内核的日志带 ANSI 颜色转义（`\\x1b[34m` 之类）。终端认识它们，浏览器不认识，
所以这里把转义序列翻译成带 class 的 <span>，再配一段 JS 逐字回放，
在浏览器里复现"看着内核一行行启动"的感觉。

生成的 HTML 不依赖任何外部资源，可以直接双击打开、发给别人、或者放进
答辩材料里。
"""

import json
import re
import sys
from pathlib import Path

# 匹配 SGR（Select Graphic Rendition）转义序列，也就是颜色控制
SGR = re.compile(r"\x1b\[([0-9;]*)m")
# 其他 ANSI 控制序列（光标移动之类），直接丢掉
OTHER_ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")

# ANSI 颜色码 → CSS class。只覆盖内核实际用到的和常见的几个。
COLORS = {
    "0": None,  # reset
    "30": "c-black",
    "31": "c-red",
    "32": "c-green",
    "33": "c-yellow",
    "34": "c-blue",
    "35": "c-magenta",
    "36": "c-cyan",
    "37": "c-white",
    "90": "c-gray",
    "91": "c-red",
    "92": "c-green",
    "93": "c-bright-yellow",
    "94": "c-blue",
    "95": "c-magenta",
    "96": "c-cyan",
    "97": "c-white",
}


def parse(text: str) -> list[list]:
    """把带 ANSI 转义的文本切成 [class, 文本] 段落列表。"""
    segments: list[list] = []
    current = None  # 当前生效的颜色 class
    pos = 0

    for match in SGR.finditer(text):
        chunk = text[pos : match.start()]
        if chunk:
            segments.append([current, chunk])

        # 一个转义序列里可能有多个参数，如 \x1b[1;31m
        for code in (match.group(1) or "0").split(";"):
            code = code.lstrip("0") or "0"
            if code in COLORS:
                current = COLORS[code]
        pos = match.end()

    tail = text[pos:]
    if tail:
        segments.append([current, tail])

    # 清掉残留的非颜色控制序列，并合并相邻同色段
    merged: list[list] = []
    for cls, chunk in segments:
        chunk = OTHER_ANSI.sub("", chunk).replace("\r", "")
        if not chunk:
            continue
        if merged and merged[-1][0] == cls:
            merged[-1][1] += chunk
        else:
            merged.append([cls, chunk])
    return merged


TEMPLATE = r"""<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>CCNUOS 启动回放</title>
<style>
  :root {
    --bg: #14161a;
    --panel: #1b1e24;
    --border: #2b3038;
    --fg: #d4d4d4;
    --muted: #8a8f98;
    --accent: #6ba6ff;
  }
  * { box-sizing: border-box; }
  body {
    margin: 0; padding: 32px 20px;
    background: var(--bg); color: var(--fg);
    font-family: -apple-system, BlinkMacSystemFont, "PingFang SC", "Segoe UI", sans-serif;
    display: flex; flex-direction: column; align-items: center;
  }
  .wrap { width: 100%; max-width: 900px; }
  h1 { font-size: 20px; font-weight: 600; margin: 0 0 4px; }
  .sub { color: var(--muted); font-size: 13px; margin-bottom: 20px; }
  .sub code {
    background: var(--panel); padding: 2px 6px; border-radius: 4px;
    border: 1px solid var(--border); font-size: 12px;
  }
  .term {
    background: var(--panel); border: 1px solid var(--border);
    border-radius: 10px; overflow: hidden;
    box-shadow: 0 8px 32px rgba(0,0,0,.4);
  }
  .bar {
    display: flex; align-items: center; gap: 8px;
    padding: 10px 14px; border-bottom: 1px solid var(--border);
    background: #202429;
  }
  .dot { width: 11px; height: 11px; border-radius: 50%; }
  .dot.r { background: #ff5f57; } .dot.y { background: #febc2e; } .dot.g { background: #28c840; }
  .title { margin-left: 8px; color: var(--muted); font-size: 12px; }
  pre {
    margin: 0; padding: 16px 18px; height: 62vh; min-height: 360px;
    overflow-y: auto; white-space: pre-wrap; word-break: break-all;
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 12.5px; line-height: 1.55;
  }
  .cursor {
    display: inline-block; width: 7px; height: 14px;
    background: var(--fg); vertical-align: -2px;
    animation: blink 1.05s steps(2, start) infinite;
  }
  @keyframes blink { to { visibility: hidden; } }

  .ctl {
    display: flex; align-items: center; gap: 10px; flex-wrap: wrap;
    margin-top: 14px;
  }
  button {
    background: var(--panel); color: var(--fg);
    border: 1px solid var(--border); border-radius: 6px;
    padding: 7px 14px; font-size: 13px; cursor: pointer;
    font-family: inherit;
  }
  button:hover { border-color: var(--accent); color: var(--accent); }
  label { color: var(--muted); font-size: 12px; display: flex; align-items: center; gap: 7px; }
  input[type=range] { width: 130px; accent-color: var(--accent); }
  .progress {
    flex: 1; min-width: 120px; height: 3px;
    background: var(--border); border-radius: 2px; overflow: hidden;
  }
  .progress > i { display: block; height: 100%; width: 0; background: var(--accent); }

  /* ANSI 颜色，按深色终端的可读性调过 */
  .c-black   { color: #4a4f57; }
  .c-red     { color: #ff6b6b; }
  .c-green   { color: #6bcf7f; }
  .c-yellow  { color: #e6b450; }
  .c-blue    { color: #6ba6ff; }
  .c-magenta { color: #d4bfff; }
  .c-cyan    { color: #5ccfe6; }
  .c-white   { color: #e6e6e6; }
  .c-gray    { color: #8a8f98; }
  .c-bright-yellow { color: #ffd166; }
</style>
</head>
<body>
<div class="wrap">
  <h1>CCNUOS 启动回放</h1>
  <div class="sub">
    QEMU virt / RISC-V 64 串口输出的真实记录，由 <code>make demo</code> 生成 ·
    <span id="meta"></span>
  </div>

  <div class="term">
    <div class="bar">
      <span class="dot r"></span><span class="dot y"></span><span class="dot g"></span>
      <span class="title">qemu-system-riscv64 — serial console</span>
    </div>
    <pre id="out"></pre>
  </div>

  <div class="ctl">
    <button id="play">⏸ 暂停</button>
    <button id="restart">↻ 重播</button>
    <button id="skip">⏭ 跳到结尾</button>
    <label>速度 <input id="speed" type="range" min="200" max="20000" step="200" value="4000"></label>
    <span class="progress"><i id="bar"></i></span>
  </div>
</div>

<script>
const SEGMENTS = __SEGMENTS__;
const TOTAL = SEGMENTS.reduce((n, s) => n + s[1].length, 0);

const out = document.getElementById('out');
const bar = document.getElementById('bar');
const playBtn = document.getElementById('play');
const speedEl = document.getElementById('speed');
document.getElementById('meta').textContent = TOTAL + ' 字符';

let shown = 0, playing = true, last = null;

function esc(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function render() {
  let budget = shown, html = '';
  for (const [cls, text] of SEGMENTS) {
    if (budget <= 0) break;
    const part = text.length <= budget ? text : text.slice(0, budget);
    budget -= part.length;
    html += cls ? '<span class="' + cls + '">' + esc(part) + '</span>' : esc(part);
  }
  if (shown < TOTAL) html += '<span class="cursor"></span>';
  out.innerHTML = html;
  bar.style.width = (TOTAL ? (shown / TOTAL * 100) : 100) + '%';
}

function tick(now) {
  if (last === null) last = now;
  const dt = (now - last) / 1000;
  last = now;

  if (playing && shown < TOTAL) {
    shown = Math.min(TOTAL, shown + Math.ceil(dt * Number(speedEl.value)));
    render();
    out.scrollTop = out.scrollHeight;   // 跟着输出往下滚
    if (shown >= TOTAL) setPlaying(false);
  }
  requestAnimationFrame(tick);
}

function setPlaying(v) {
  playing = v;
  playBtn.textContent = v ? '⏸ 暂停' : '▶ 播放';
}

playBtn.onclick = () => {
  if (shown >= TOTAL) { shown = 0; render(); }
  setPlaying(!playing);
};
document.getElementById('restart').onclick = () => {
  shown = 0; render(); out.scrollTop = 0; setPlaying(true);
};
document.getElementById('skip').onclick = () => {
  shown = TOTAL; render(); out.scrollTop = out.scrollHeight; setPlaying(false);
};

render();
requestAnimationFrame(tick);
</script>
</body>
</html>
"""


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2

    src, dst = Path(sys.argv[1]), Path(sys.argv[2])
    if not src.exists():
        print(f"找不到输入日志：{src}", file=sys.stderr)
        return 1

    raw = src.read_text(encoding="utf-8", errors="replace")
    segments = parse(raw)

    # ensure_ascii=True 让中文和转义字符都变成 \uXXXX，
    # 嵌进 <script> 里绝不会因为引号或 </script> 而破坏 HTML
    payload = json.dumps(segments, ensure_ascii=True)
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_text(TEMPLATE.replace("__SEGMENTS__", payload), encoding="utf-8")

    chars = sum(len(s[1]) for s in segments)
    print(f"[ansi2html] {src} → {dst}（{len(segments)} 个彩色段，{chars} 字符）")
    return 0


if __name__ == "__main__":
    sys.exit(main())

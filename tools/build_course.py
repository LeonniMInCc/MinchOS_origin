#!/usr/bin/env python3
"""把 docs/ 下的章节编译成一个可切章阅读的单文件课程站点。

    用法: python3 tools/build_course.py <输出 html>
          （通常通过 make course 调用）

# 为什么自己写 markdown 渲染

只用到 markdown 的一个很小的子集（标题、代码块、表格、引用、列表、
行内强调），而这些章节是自己写的，格式可控。为此引入一个第三方库
（还要求学生装 pip 包）不值得 —— 和内核那边"不依赖外部 crate"
是同一个理由。

# 这个文件是实验平台的地基

课程站点最终要做成：每章 = **原理文本 + 对应代码 + 练习 + 自动判题**。
现在先把前三样做出来，并且把练习**结构化地**抽出来（而不是混在正文里），
判题系统接上来的时候才有东西可挂。
"""

import html
import json
import re
import sys
from pathlib import Path

DOCS = Path(__file__).resolve().parent.parent / "docs"

# 附录按这个顺序排。键是 docs/ 下的文件名，值是侧栏里显示的标题。
APPENDIX = [
    ("RUNNING.md", "怎么跑起来"),
    ("REPORT.md", "实验报告"),
    ("ROADMAP-AIOS.md", "AIOS 篇路线图（第 17–24 周）"),
    ("PLATFORM.md", "实验平台设计"),
]

# ---------------------------------------------------------------------------
# markdown → html
# ---------------------------------------------------------------------------

_CODE_SLOT = "\x00%d\x00"


def _link(m):
    text, href = m.group(1), m.group(2)
    # 章节之间的互链走站内锚点，不要跳出去打开一个 .md 源文件
    ref = re.fullmatch(r"(?:\./)?(ch\d+|REPORT)\.md", href)
    if ref:
        return f'<a href="#{ref.group(1).lower()}" data-jump="{ref.group(1).lower()}">{text}</a>'
    # 指向仓库里源码的链接：站点是从仓库根目录起服务的，相对路径原样可用
    cls = ' class="src"' if href.startswith("../") else ""
    ext = ' target="_blank" rel="noopener"' if href.startswith("http") else ""
    return f'<a href="{href}"{cls}{ext}>{text}</a>'


def inline(s):
    """行内元素。先把代码片段藏起来，免得里面的 * 和 [ 被当成标记。"""
    slots = []

    def stash(m):
        slots.append(m.group(1))
        return _CODE_SLOT % (len(slots) - 1)

    s = re.sub(r"`([^`]+)`", stash, s)
    s = html.escape(s)
    s = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", _link, s)
    s = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", s)
    s = re.sub(r"(?<![*\w])\*([^*\n]+)\*(?![*\w])", r"<em>\1</em>", s)
    return re.sub(
        "\x00(\\d+)\x00",
        lambda m: "<code>" + html.escape(slots[int(m.group(1))]) + "</code>",
        s,
    )


def _list_block(lines, i, out):
    """连续的列表行。按缩进决定嵌套层级。"""
    item = re.compile(r"^(\s*)(?:([-*])|(\d+)\.)\s+(.*)$")
    stack = []  # [(缩进, 标签)]
    while i < len(lines):
        m = item.match(lines[i])
        if not m:
            # 缩进的续行属于上一个 li（章节里大量使用）
            if stack and lines[i].strip() and lines[i][:1] in " \t":
                out.append(" " + inline(lines[i].strip()))
                i += 1
                continue
            break
        indent, tag = len(m.group(1)), "ul" if m.group(2) else "ol"
        while stack and indent < stack[-1][0]:
            out.append(f"</li></{stack.pop()[1]}>")
        if not stack or indent > stack[-1][0]:
            out.append(f"<{tag}>")
            stack.append((indent, tag))
        else:
            out.append("</li>")
        out.append("<li>" + inline(m.group(4)))
        i += 1
    while stack:
        out.append(f"</li></{stack.pop()[1]}>")
    return i


def render(md):
    lines = md.split("\n")
    out, i = [], 0
    while i < len(lines):
        line = lines[i]

        # 代码块：里面的一切都不做 markdown 处理
        if line.startswith("```"):
            lang = line[3:].strip()
            i += 1
            buf = []
            while i < len(lines) and not lines[i].startswith("```"):
                buf.append(lines[i])
                i += 1
            i += 1
            cls = f' class="lang-{lang}"' if lang else ""
            out.append(f"<pre{cls}><code>{html.escape(chr(10).join(buf))}</code></pre>")
            continue

        if not line.strip():
            i += 1
            continue

        if re.fullmatch(r"-{3,}", line.strip()):
            out.append("<hr>")
            i += 1
            continue

        m = re.match(r"^(#{1,6})\s+(.*)$", line)
        if m:
            n = len(m.group(1))
            text = m.group(2)
            # 用小节号做锚点，方便直接链到某一节
            sec = re.match(r"^(\d+\.\d+)\s", text)
            anchor = f' id="s{sec.group(1)}"' if sec else ""
            out.append(f"<h{n}{anchor}>{inline(text)}</h{n}>")
            i += 1
            continue

        # 表格：当前行和下一行分别是 |...| 和 |---|
        if line.lstrip().startswith("|") and i + 1 < len(lines) and re.match(
            r"^\s*\|[\s:|-]+\|\s*$", lines[i + 1]
        ):
            def cells(row):
                return [c.strip() for c in row.strip().strip("|").split("|")]

            head = cells(line)
            i += 2
            rows = []
            while i < len(lines) and lines[i].lstrip().startswith("|"):
                rows.append(cells(lines[i]))
                i += 1
            th = "".join(f"<th>{inline(c)}</th>" for c in head)
            tb = "".join(
                "<tr>" + "".join(f"<td>{inline(c)}</td>" for c in r) + "</tr>"
                for r in rows
            )
            out.append(f"<table><thead><tr>{th}</tr></thead><tbody>{tb}</tbody></table>")
            continue

        # 引用块：剥掉 > 之后递归渲染，这样引用里的代码块和列表也正常
        if line.lstrip().startswith(">"):
            buf = []
            while i < len(lines) and (
                lines[i].lstrip().startswith(">") or (buf and not lines[i].strip())
            ):
                if not lines[i].strip():
                    # 空行只有在后面还有引用时才算引用的一部分
                    if not (i + 1 < len(lines) and lines[i + 1].lstrip().startswith(">")):
                        break
                    buf.append("")
                else:
                    buf.append(re.sub(r"^\s*>\s?", "", lines[i]))
                i += 1
            out.append("<blockquote>" + render("\n".join(buf)) + "</blockquote>")
            continue

        if re.match(r"^\s*(?:[-*]|\d+\.)\s+", line):
            i = _list_block(lines, i, out)
            continue

        # 段落：连续的非空行合成一段
        buf = []
        while i < len(lines) and lines[i].strip() and not re.match(
            r"^(#{1,6}\s|```|>|\s*(?:[-*]|\d+\.)\s|\s*\|)", lines[i]
        ) and not re.fullmatch(r"-{3,}", lines[i].strip()):
            buf.append(lines[i].strip())
            i += 1
        if buf:
            out.append("<p>" + inline(" ".join(buf)) + "</p>")
        else:
            i += 1
    return "".join(out)


# ---------------------------------------------------------------------------
# 抽取章节结构
# ---------------------------------------------------------------------------


def split_exercises(md):
    """把练习一节从正文里摘出来，单独结构化。

    练习是实验平台的核心对象 —— 判题、进度、提交都挂在它上面。
    留在正文里的话，它就只是一段文字。
    """
    m = re.search(r"^## [\d.]+\s*练习\s*$", md, re.M)
    if not m:
        return md, []
    start = m.start()
    nxt = re.search(r"^## ", md[m.end():], re.M)
    end = m.end() + nxt.start() if nxt else len(md)
    body = md[m.end():end]

    items = []
    # 每条练习是一个顶层有序列表项，可能跨多行
    for im in re.finditer(r"^(\d+)\.\s+(.*?)(?=^\d+\.\s|\Z)", body, re.M | re.S):
        text = im.group(2).strip()
        title = re.match(r"\*\*(.+?)\*\*", text)
        items.append(
            {
                "n": int(im.group(1)),
                "title": title.group(1) if title else text.split("。")[0][:40],
                "html": render(text),
            }
        )
    return md[:start] + md[end:], items


def load_chapter(path):
    md = path.read_text(encoding="utf-8")
    title = re.match(r"^#\s+(.*)", md).group(1).strip()
    tagm = re.search(r"对应 tag `(v[\d.]+)`", md)
    num = re.search(r"第\s*(\d+)\s*章", title)
    body, exercises = split_exercises(md)
    # 正文里的大标题由外面的页眉负责，删掉避免重复
    body = re.sub(r"^#\s+.*\n", "", body, count=1)
    return {
        "id": path.stem.lower(),
        "n": int(num.group(1)) if num else 0,
        "title": re.sub(r"^第\s*\d+\s*章[　\s]*", "", title),
        "tag": tagm.group(1) if tagm else "",
        "html": render(body),
        "exercises": exercises,
        "words": len(re.sub(r"\s", "", md)),
    }


# ---------------------------------------------------------------------------
# 页面
# ---------------------------------------------------------------------------

CSS = """
:root{--bg:#14161a;--panel:#1b1e24;--panel2:#20242b;--border:#2b3038;
      --fg:#d8dae0;--muted:#8a8f98;--accent:#6ba6ff;--ok:#6bcf7f;--warn:#ffd166;--bad:#ff6b6b}
*{box-sizing:border-box;margin:0;padding:0}
html{scroll-behavior:smooth}
body{background:var(--bg);color:var(--fg);font-size:15px;line-height:1.8;
     font-family:-apple-system,BlinkMacSystemFont,"PingFang SC","Segoe UI",sans-serif}
a{color:var(--accent);text-decoration:none}
a:hover{text-decoration:underline}
code{font-family:"SF Mono",Menlo,monospace;font-size:.88em;background:#0d0f12;
     padding:1.5px 6px;border-radius:4px;color:var(--warn);word-break:break-word}
pre{background:#0d0f12;border:1px solid var(--border);border-radius:8px;padding:14px 16px;
    overflow:auto;margin:16px 0}
pre code{background:none;padding:0;color:#c8ccd4;font-size:12.5px;line-height:1.7}

#shell{display:flex;min-height:100vh}

/* 侧栏 */
#side{width:268px;flex:none;background:var(--panel);border-right:1px solid var(--border);
      position:sticky;top:0;height:100vh;display:flex;flex-direction:column}
#brand{padding:18px 18px 12px}
#brand b{font-size:17px;letter-spacing:-.3px}
#brand span{display:block;font-size:11.5px;color:var(--muted);margin-top:3px}
#q{margin:0 14px 10px;padding:7px 11px;background:var(--bg);color:var(--fg);
   border:1px solid var(--border);border-radius:7px;font-size:12.5px;width:calc(100% - 28px);
   font-family:inherit}
#q:focus{outline:none;border-color:var(--accent)}
#toc{overflow-y:auto;flex:1;padding:0 10px 18px}
.navsec{font-size:10.5px;color:#565c66;letter-spacing:.8px;padding:12px 8px 5px}
.nav{display:flex;align-items:baseline;gap:8px;padding:6px 9px;border-radius:6px;
     cursor:pointer;font-size:13px;color:var(--fg);line-height:1.5}
.nav:hover{background:var(--panel2)}
.nav.on{background:#243044;color:#fff}
.nav .idx{color:var(--muted);font-size:11px;font-variant-numeric:tabular-nums;
          min-width:17px;text-align:right;flex:none}
.nav.on .idx{color:var(--accent)}
.nav .dot{width:6px;height:6px;border-radius:50%;flex:none;margin-left:auto;
          align-self:center;background:transparent}
.nav.done .dot{background:var(--ok)}
#prog{padding:10px 18px;border-top:1px solid var(--border);font-size:11.5px;color:var(--muted)}
#progbar{height:3px;background:var(--border);border-radius:2px;margin-top:6px;overflow:hidden}
#progbar i{display:block;height:100%;background:var(--ok);width:0;transition:width .3s}

/* 正文 */
#main{flex:1;min-width:0;display:flex;justify-content:center;padding:0 40px 90px}
#page{width:100%;max-width:790px}
.head{padding:34px 0 18px;border-bottom:1px solid var(--border);margin-bottom:26px}
.kicker{font-size:11.5px;color:var(--muted);letter-spacing:.6px}
h1{font-size:29px;line-height:1.3;margin:7px 0 14px;letter-spacing:-.5px}
.meta{display:flex;gap:9px;flex-wrap:wrap;align-items:center}
.chip{background:var(--panel);border:1px solid var(--border);border-radius:99px;
      padding:3px 11px;font-size:11.5px;color:var(--muted)}
.chip b{color:var(--accent);font-weight:600}
.gitcmd{font-family:"SF Mono",Menlo,monospace;font-size:11.5px;background:#0d0f12;
        border:1px solid var(--border);border-radius:99px;padding:3px 11px;color:var(--warn);
        cursor:pointer}
.gitcmd:hover{border-color:var(--accent)}

article h2{font-size:20px;margin:36px 0 12px;padding-top:6px;letter-spacing:-.3px}
article h3{font-size:16px;margin:26px 0 9px;color:#eceef2}
article p{margin:12px 0}
article ul,article ol{margin:12px 0 12px 22px}
article li{margin:5px 0}
article blockquote{border-left:3px solid var(--accent);background:#181c22;
                   padding:10px 16px;margin:18px 0;border-radius:0 6px 6px 0}
article blockquote p{margin:6px 0}
article table{border-collapse:collapse;width:100%;margin:18px 0;font-size:13.5px}
article th,article td{border:1px solid var(--border);padding:7px 11px;text-align:left;
                      vertical-align:top}
article th{background:var(--panel);color:var(--muted);font-weight:500}
article hr{border:none;border-top:1px solid var(--border);margin:28px 0}
article strong{color:#fff}
a.src{color:#9ae6b4}

/* 练习 */
#ex{margin-top:46px;border-top:1px solid var(--border);padding-top:26px}
#ex>h2{margin-top:0;display:flex;align-items:center;gap:10px}
#ex>h2 .cnt{font-size:12px;color:var(--muted);font-weight:400}
.task{background:var(--panel);border:1px solid var(--border);border-radius:9px;
      padding:13px 16px;margin:11px 0;display:flex;gap:13px;align-items:flex-start}
.task.ok{border-color:#2f5f3f;background:#172019}
.task input{margin-top:6px;width:15px;height:15px;accent-color:var(--ok);
            cursor:pointer;flex:none}
.task .body{min-width:0;flex:1}
.task .body p:first-child{margin-top:0}
.task .body p:last-child{margin-bottom:0}
.task .no{color:var(--accent);font-weight:600;font-variant-numeric:tabular-nums}
.note{background:#1a1d24;border:1px dashed var(--border);border-radius:9px;
      padding:12px 16px;margin:18px 0;font-size:12.5px;color:var(--muted);line-height:1.75}
.note b{color:var(--fg)}

/* 翻章 */
.pager{display:flex;justify-content:space-between;gap:14px;margin-top:44px;
       border-top:1px solid var(--border);padding-top:20px}
.pager a{background:var(--panel);border:1px solid var(--border);border-radius:9px;
         padding:11px 16px;flex:1;max-width:330px;font-size:13px}
.pager a:hover{border-color:var(--accent);text-decoration:none}
.pager a span{display:block;font-size:10.5px;color:var(--muted);margin-bottom:2px}
.pager .r{text-align:right;margin-left:auto}

#top{position:fixed;right:22px;bottom:22px;width:36px;height:36px;border-radius:50%;
     background:var(--panel);border:1px solid var(--border);color:var(--muted);
     cursor:pointer;font-size:15px;display:none}
#top:hover{color:var(--accent);border-color:var(--accent)}

@media (max-width:900px){
  #shell{flex-direction:column}
  #side{width:100%;height:auto;position:static;border-right:none;
        border-bottom:1px solid var(--border)}
  #toc{max-height:210px}
  #main{padding:0 18px 60px}
}
"""

JS = """
const SEEN='ccnuos.read', DONE='ccnuos.done';
const store=k=>{try{return JSON.parse(localStorage.getItem(k))||{}}catch(e){return{}}};
const save=(k,v)=>{try{localStorage.setItem(k,JSON.stringify(v))}catch(e){}};
let read=store(SEEN), done=store(DONE), cur=null;

function paint(){
  let n=0;
  document.querySelectorAll('.nav').forEach(el=>{
    const id=el.dataset.id, ok=!!read[id];
    el.classList.toggle('done', ok);
    el.classList.toggle('on', id===cur);
    if(ok) n++;
  });
  const total=CH.length;
  document.getElementById('prognum').textContent=n+' / '+total;
  document.getElementById('progbar').firstElementChild.style.width=(n/total*100)+'%';
}

function show(id, push=true){
  const c=CH.find(x=>x.id===id); if(!c) return;
  cur=id;
  document.getElementById('page').innerHTML=view(c);
  read[id]=true; save(SEEN,read); paint();
  wire(c);
  if(push && location.hash.slice(1)!==id) history.pushState({id},'','#'+id);
  scrollTo(0,0);
  document.title=c.n?`第 ${c.n} 章 ${c.title} · CCNUOS`:c.title+' · CCNUOS';
}

function view(c){
  const i=CH.indexOf(c), prev=CH[i-1], next=CH[i+1];
  // 第一章没有上一个 tag 可比，只能 git show
  const cmd=c.tag?(c.prevTag?`git diff ${c.prevTag}..${c.tag}`:`git show ${c.tag}`):'';
  const git=cmd?`<span class="gitcmd" data-copy="${cmd}" title="点一下复制">${cmd} ⧉</span>`:'';
  const chips=[
    c.tag?`<span class="chip">tag <b>${c.tag}</b></span>`:'',
    c.exercises.length?`<span class="chip">练习 <b>${c.exercises.length}</b></span>`:'',
    `<span class="chip">约 ${Math.round(c.words/400)*400} 字</span>`, git
  ].join('');
  let ex='';
  if(c.exercises.length){
    ex='<section id="ex"><h2>练习 <span class="cnt">勾选状态存在你自己的浏览器里</span></h2>'
      + c.exercises.map(t=>{
          const k=c.id+'#'+t.n, on=done[k]?' ok':'';
          return `<div class="task${on}" data-k="${k}">
            <input type="checkbox" ${done[k]?'checked':''}>
            <div class="body"><span class="no">${t.n}.</span> ${t.html}</div></div>`;
        }).join('')
      + `<div class="note"><b>自动判题还没有接上。</b>目前这些勾选只是给你自己记进度用的，
         存在浏览器本地，换台机器就没了。真正的做法是把每条练习变成一个可执行的判据 ——
         详见路线图里的实验平台部分。</div></section>`;
  }
  const pager=`<nav class="pager">
    ${prev?`<a href="#${prev.id}" data-jump="${prev.id}"><span>上一章</span>${prev.n?'第 '+prev.n+' 章 ':''}${prev.title}</a>`:'<span></span>'}
    ${next?`<a class="r" href="#${next.id}" data-jump="${next.id}"><span>下一章</span>${next.n?'第 '+next.n+' 章 ':''}${next.title}</a>`:''}
  </nav>`;
  return `<header class="head">
      <div class="kicker">${c.n?'第 '+c.n+' 章':'附录'}</div>
      <h1>${c.title}</h1><div class="meta">${chips}</div></header>
    <article>${c.html}</article>${ex}${pager}`;
}

function wire(c){
  document.querySelectorAll('.task input').forEach(box=>{
    box.onchange=()=>{
      const el=box.closest('.task'), k=el.dataset.k;
      box.checked?done[k]=true:delete done[k];
      el.classList.toggle('ok', box.checked); save(DONE,done);
    };
  });
  document.querySelectorAll('[data-copy]').forEach(el=>{
    el.onclick=async()=>{
      try{await navigator.clipboard.writeText(el.dataset.copy);
          const t=el.textContent; el.textContent='已复制';
          setTimeout(()=>el.textContent=t,1100);}catch(e){}
    };
  });
}

// 站内跳转统一走 show()，不真的换页
addEventListener('click',e=>{
  const a=e.target.closest('[data-jump]');
  if(a){e.preventDefault(); show(a.dataset.jump);}
});
addEventListener('popstate',()=>show(location.hash.slice(1)||CH[0].id,false));

addEventListener('keydown',e=>{
  if(/^(INPUT|TEXTAREA)$/.test(e.target.tagName)) return;
  const i=CH.findIndex(x=>x.id===cur);
  if(e.key==='ArrowRight'&&CH[i+1]) show(CH[i+1].id);
  if(e.key==='ArrowLeft'&&CH[i-1]) show(CH[i-1].id);
  if(e.key==='/'){e.preventDefault(); document.getElementById('q').focus();}
});

document.getElementById('q').oninput=e=>{
  const q=e.target.value.trim().toLowerCase();
  document.querySelectorAll('.nav').forEach(el=>{
    el.style.display=!q||el.textContent.toLowerCase().includes(q)?'':'none';
  });
  document.querySelectorAll('.navsec').forEach(el=>{
    let n=el.nextElementSibling, any=false;
    while(n&&n.classList.contains('nav')){ if(n.style.display!=='none') any=true; n=n.nextElementSibling; }
    el.style.display=any?'':'none';
  });
};

// 不能叫 top —— window.top 已经占了这个名字，const 会当场报重复声明
const toTop=document.getElementById('top');
toTop.onclick=()=>scrollTo({top:0,behavior:'smooth'});
addEventListener('scroll',()=>toTop.style.display=scrollY>600?'block':'none');

show(location.hash.slice(1)||CH[0].id,false);
"""


def build(chapters, out_path):
    nav = ['<div class="navsec">正文</div>']
    for c in chapters:
        if c["n"] == 0:
            nav.append('<div class="navsec">附录</div>')
        nav.append(
            f'<div class="nav" data-id="{c["id"]}" data-jump="{c["id"]}">'
            f'<span class="idx">{c["n"] or "—"}</span>'
            f'<span>{inline(c["title"])}</span><span class="dot"></span></div>'
        )

    page = f"""<!DOCTYPE html>
<html lang="zh-CN"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>CCNUOS 课程</title>
<style>{CSS}</style></head><body>
<div id="shell">
  <aside id="side">
    <div id="brand"><b>CCNUOS</b><span>用 Rust 从零写一个 RISC-V 操作系统</span></div>
    <input id="q" placeholder="搜索章节（按 / 聚焦）" autocomplete="off">
    <nav id="toc">{''.join(nav)}</nav>
    <div id="prog">已读 <span id="prognum">0 / 0</span><div id="progbar"><i></i></div></div>
  </aside>
  <main id="main"><div id="page"></div></main>
</div>
<button id="top" title="回到顶部">↑</button>
<script>const CH={json.dumps(chapters, ensure_ascii=False)};</script>
<script>{JS}</script>
</body></html>"""
    out_path.write_text(page, encoding="utf-8")


def main():
    if len(sys.argv) != 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    out = Path(sys.argv[1])

    chapters = [load_chapter(p) for p in sorted(DOCS.glob("ch*.md"))]
    chapters.sort(key=lambda c: c["n"])
    # 每章配一条 git diff，范围是"上一章的 tag 到这一章的 tag"
    for prev, cur in zip(chapters, chapters[1:]):
        cur["prevTag"] = prev["tag"]

    # 附录：不属于任何一章，但学生要看的东西
    for fname, title in APPENDIX:
        f = DOCS / fname
        if not f.exists():
            continue
        md = f.read_text(encoding="utf-8")
        chapters.append(
            {
                "id": f.stem.lower().replace("-", ""),
                "n": 0,
                "title": title,
                "tag": "",
                "html": render(re.sub(r"^#\s+.*\n", "", md, count=1)),
                "exercises": [],
                "words": len(re.sub(r"\s", "", md)),
            }
        )

    build(chapters, out)
    ex = sum(len(c["exercises"]) for c in chapters)
    print(f"[build_course] {len(chapters)} 章、{ex} 道练习 → {out}")
    print(f"             {out.stat().st_size // 1024} KiB，单文件，无外部依赖")
    return 0


if __name__ == "__main__":
    sys.exit(main())

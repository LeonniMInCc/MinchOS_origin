# CCNUOS 线上实验平台设计

> 目标：学生从 git 拉代码，对着章节学原理，做练习，提交后系统自动判对错。

---

## 1. 一个决定性的前提

**判题器必须和 `make regress` 是同一个东西。**

第 16 周已经有了一套 16 个用例的端到端回归测试。如果判题再另写一套，
两套判据一定会漂移 —— 参考实现改了，回归测试跟着改，判题器忘了改，
于是学生做对了却判错。

所以平台不新建判题引擎，而是**把回归测试的引擎抽出来复用**：

```
tools/checker.py     判题引擎（从 regress.py 抽出来）
  ├── tools/regress.py    参考实现自检：我的代码还对不对
  └── tools/grade.py      学生判题：你的代码对不对
```

同一个引擎、同一批判据。参考实现必须能拿满分 —— 这是判题器自身的测试。

---

## 2. 三层内容，只有中间一层能自动判

这是设计上最要紧的一次诚实。

| 层 | 是什么 | 能自动判吗 |
|---|---|---|
| **章节文本** | 原理讲解，`docs/chXX.md` | 不需要判 |
| **章节验收（checkpoint）** | "这一章的功能做出来了吗" | **能，且必须能** |
| **练习（exercise）** | 86 道开放式思考题 | **大部分不能** |

练习里有大量这种题：

> 把 `Mutex::lock` 里的 `loop` 改成 `if`，多跑几次，看它在什么情况下会出错。

它的价值在于**学生观察到了什么**，不在于代码长什么样。
硬要自动判，只会把它异化成"猜测试用例"。

所以平台的判题对象是 **checkpoint**，不是 exercise。
练习保留为开放式作业，由人看、或者由 AI 助教给反馈（第 6 节）。

> uCore / rCore 的实验也是这么分的：lab 有自动测试，思考题交报告。
> 这不是妥协，是对"什么东西可以被机器判断"的正确认识。

---

## 3. 仓库怎么组织：tag 对

每一章有两个 tag：

```
v0.6-start    骨架：该学生写的地方挖空成 todo!()，其余完整
v0.6          参考实现
```

学生的工作流：

```bash
git clone <课程仓库> && cd ccnuos
git checkout -b my-ch06 v0.6-start   # 从骨架开始
# ...写代码...
make grade CH=6                       # 本地先自测
git push origin my-ch06               # 交上去，CI 自动判
```

**骨架由参考实现生成，不手工维护。** `tools/make_skeleton.py` 按源码里的标记
挖空：

```rust
pub fn frame_alloc() -> Option<FrameTracker> {
    // BEGIN-SKELETON: 从空闲链表里取一页；没有就尝试从未分配区域推进
    //   提示：先看 recycled，再看 current < end
    // END-SKELETON
    todo!("实现物理页帧分配")
}
```

这样参考实现永远是唯一来源，骨架跟着它走，不会脱节。

---

## 4. 判据长什么样

每个 checkpoint 是一组**可执行判据**。判据只有五种，够用且好懂：

| 类型 | 检查什么 | 例子 |
|---|---|---|
| `build` | 能不能编译，有没有告警 | 零告警是硬要求 |
| `unit` | `cargo test` 的某个子集 | easy-fs 的 18 个测试 |
| `run` | 跑 QEMU，喂命令，断言输出 | 就是 regress 的用例 |
| `selfcheck` | 内核启动自检必须通过 | "无泄漏"、无 panic |
| `source` | 源码必须/不许包含某模式 | 防止把 `todo!()` 换成硬编码答案 |

清单写成一个文件，和章节并列：

```toml
[[checkpoint]]
ch = 6
name = "物理页帧分配器"
points = 40

  [[checkpoint.check]]
  type = "unit"
  filter = "frame_allocator"
  points = 15

  [[checkpoint.check]]
  type = "selfcheck"
  must = ["页帧自检通过"]
  points = 15

  [[checkpoint.check]]
  type = "run"
  commands = ["11forktest"]
  must = ["页帧核对", "无泄漏"]
  must_not = ["[KERNEL PANIC]"]
  points = 10
```

`must` / `must_not` 这套是第 16 周就验证过的 —— 而且
**`must_not` 是更值钱的一半**：那些"坏"程序自己会喊
"如果你看到这行，说明内核没拦住我"，判据盯着那句话就够了。

---

## 5. 判题在哪跑：CI

学生 push 到自己的分支 → CI 跑 `make grade` → 结果回写成 PR 评论。

```yaml
# .github/workflows/grade.yml
on: [push, pull_request]
jobs:
  grade:
    runs-on: ubuntu-latest
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
      - run: sudo apt-get install -y qemu-system-misc
      - run: rustup target add riscv64gc-unknown-none-elf
      - run: make grade CH=${{ ... }} FORMAT=json > score.json
      - uses: actions/upload-artifact@v4
        with: { name: scorecard, path: score.json }
```

**选 CI 而不是自建服务的三个理由：**

1. **几乎零运维。** 不用管沙箱逃逸、并发排队、机器扩容 —— 这些都是真问题，
   而且和教学无关。
2. **天然有提交历史。** 谁什么时候做了什么，git 记得清清楚楚。
   抄袭检测不需要额外系统。
3. **学生顺便学会了 CI。** 这本身就是该学的东西。

代价是判题环境由托管平台决定，跑得比本机慢。可以接受 —— 一次判题约 3 分钟。

---

## 6. 防作弊

| 手段 | 防什么 |
|---|---|
| **隐藏判据** | 公开的判据只占 70%，剩下 30% 只在 CI 里跑 |
| **参数随机化** | 页数、进程数、缓冲区大小每次随机，硬编码答案会挂 |
| `source` 判据 | 禁止删测试、禁止 `#[ignore]`、禁止留 `todo!()` |
| 提交历史 | 一次 commit 就交出完整实现的，人工复核 |
| 参考实现必须满分 | 判题器自己的正确性检查 |

**不做**的：不查代码相似度。在一个所有人都照着同一份骨架、
实现同一个算法的课里，相似度天然极高，误报会淹没真信号。
提交历史是更可靠的信号。

---

## 7. 站点

`docs/index.html` 已经做出来了（`make course`）：17 章可切换、
86 道练习结构化抽出、阅读进度本地保存。

接下来加三样：

1. **checkpoint 面板**：每章显示判据清单和本地自测命令
2. **成绩回显**：读 CI 产出的 `score.json`，在章节页上显示通过/未通过
3. **AI 助教（可选）**：开放式练习提交后，把学生的代码 + 那道题的标准答案要点
   一起给模型，返回**引导性提问**而不是答案

> 第 3 条要克制。助教的作用是"你有没有想过 X"，不是给答案。
> 给答案的助教会毁掉这门课 —— 这门课最值钱的部分恰恰是学生自己踩坑。

---

## 8. 实施顺序

| 步骤 | 产出 | 工作量 |
|---|---|---|
| 1 | `tools/checker.py`：从 `regress.py` 抽出引擎 | 小 |
| 2 | `docs/checkpoints.toml`：16 章的判据清单 | 中 |
| 3 | `tools/grade.py` + `make grade` | 小 |
| 4 | `tools/make_skeleton.py` + 各章 `-start` tag | **大** |
| 5 | CI workflow | 小 |
| 6 | 站点接成绩回显 | 中 |

**第 4 步是真正的大活** —— 16 章的骨架要挖得恰到好处：
挖太多学生无从下手，挖太少学不到东西。而且每挖一处都要配提示。

建议顺序：先做 1–3 和 5（判题链路打通，用参考实现验证），
再挑 2–3 章试做骨架，验证颗粒度合适了，才批量做剩下的。

---

## 9. 和 uCore / rCore 的区别

清华那套做得很好，这套要不一样才有存在的理由：

| | uCore / rCore | CCNUOS |
|---|---|---|
| 讲解 | 独立的实验指导书 | **每章 = 一个 tag 的 diff**，代码演进史就是教材 |
| 坑 | 通常不写 | **专门写**，实验报告最有价值的一节全是坑 |
| 可视化 | 无 | 调度甘特图 / 系统调用时序 / 地址空间对照 |
| 结论 | 讲道理 | **实测数字**，测不出来就写测不出来 |
| 后半程 | 更多 OS 特性 | **AIOS**：同一批概念在 AI 负载下重新出现 |

最后一行是真正的差异化。学生学完能说出
"PagedAttention 就是分页"，而且两边都自己写过 —— 这是别处给不了的。

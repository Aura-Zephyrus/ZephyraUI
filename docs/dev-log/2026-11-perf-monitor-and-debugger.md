# 性能监测与调试器：拿 CLI 当 AI 用一遍，量出来的三件事

> 现象：AI 要问「这一帧慢在哪」，答不上来；`perf profile` 报「2500 fps」而应用
> 实际只画了 9 帧；「一边盯着看、一边动手点」做不到 —— 第二个客户端的命令
> **根本发不过去**。

这一轮做三件事：**用 CLI 把应用当 AI 用一遍**（找出真实卡点）→ **按量出来的
数字优化 core 与 base** → **写 CLI 的性能监测与调试器**。

---

## 一、优化：core 与 base

### 数字怎么量的

`gallery --bench`（release，1400×800，213 个四边形，稳态 120 帧取中位数）。

⚠ **先修了一处测量错误**，否则后面的数字都不可信：

`--bench` 的「各阶段之和 / 合计」一直报 **62%**，读的人以为「38% 的帧时间
没被计时」，于是去找「漏掉的代码」—— 而那 38% 就是 `cache_us`
（`FrameCache::frame` 整段）。原因：报表加的是 `view + diff + layout + paint
+ expand`，而 `diff` / `layout` / `paint` 在 `FrameCache` 引入之后**恒为 0**
（它们搬进了 core），真正在跑的那段 `cache_us` 被漏掉了。

改成加 `view + cache + expand`（`cache` 已含那五段，全加起来会算两遍），
占比回到 **100.0%**。这一处修在 `gallery/src/bin/main.rs` 与
`gallery/src/bin/main.rs` 两份 bench 里（它们必须同步 —— 两处独立实现的
时间数字不能对比）。

### 改了什么

| 改动 | 位置 | 为什么 |
|---|---|---|
| **树里每节点每帧少一次堆分配** | `zephyra-core/src/diff.rs` | 对账时要「本帧声明了哪些键」的清单，原来是个每节点新建的 `Vec<&'static PropertyKey>` |
| **样式注入的缓存查找不再分配** | `zephyra-style/src/apply.rs` | 键是 `(NodeTag, Vec<&'static str>)`，查找前先 `clone()` 出一份向量 —— 每节点每帧一次 |
| **属性值不再被无谓克隆** | `zephyra-core/src/property.rs` | `set_local_if_changed` / `set_styled_if_changed` 按值收参 ⇒ 调用点就得克隆；而 `PropertyValue::Filter` 持一个 `Vec<Filter>` |
| **声明树不再为丢弃的变更记录克隆** | `zephyra-core/src/property.rs` + `element.rs` | `Element::with` 拿到的 `PropertyChange` 直接丢掉，但值已经被克隆了一遍 |
| **`Str` 相等先比指针** | `zephyra-core/src/property.rs` | `Arc<str>` 的相等是**逐字节比字符串**；而声明树每帧重建时字符串是**共享**的（同一个 `Arc`），指针一比就知道没变 |
| **脏矩形暂存跨帧复用** | `zephyra-core/src/frame.rs` | 每帧新建一个 `Vec` 是一次堆分配，而稳态下脏区扫描每帧都跑 |

### 实测

```text
gallery（release，1400×800，213 四边形，稳态）
                    改前      改后
整帧中位              79 µs →   66 µs   （−16%）
每帧分配次数          390   →  291    （−25%）
每帧分配字节        61912   → 59720
core 段（cache）      ~30 µs → ~20 µs
「各阶段之和/合计」    62.3%  → 100.0%  （报表 bug 修好）
```

分段（改后，稳态）：`view 33 µs` / `diff 9` / `style 7` / `scan 2` /
`layout 0` / `paint 0` / `expand 15`。

⚠ `layout` 与 `paint` 是 0 的意思是「那两段**没跑**」（稳态下树没变，
core 直接跳过），不是「不花钱」。**这条语义必须写在字段文档里** ——
恒 0 的字段比没有字段更糟。

resize 路径（每帧改视口 ⇒ layout 与 paint 真的跑）：
`diff 10 / style 8 / layout 33 / paint 25` µs，整帧 **127 µs**（改前 131 µs）。

### 剩下的分配在哪

改后每帧仍有 291 次分配、59.7 KB。按大小分桶看：

```text
≤ 512 字节：每帧 99 次，33.3 KB   ← 声明树的属性表 Vec（增长步）
≤ 256 字节：每帧 76 次，10.5 KB
≤1024 字节：每帧 16 次， 8.2 KB
```

这些是**声明树重建**的代价：`Element` 每帧重建 ⇒ 每个带属性的元素一个
`PropertyStore.local` 向量，而向量是**逐步增长**的（4 → 8 → 16 个条目），
所以一个 10 属性的元素要分配 3 次。

**这不是能靠改 core 消掉的**：根因是「声明树每帧重建」这个架构。gallery
的 zypui 部分已经有 per-element 缓存（`zypui/src/runtime.rs`），手写的
`Element` 树还没有。要消掉它得把声明树也缓存起来 —— 那是独立一轮。

---

## 二、性能监测：`zephyra perf`

### 为什么要有它

`bridge` 能问「现在是什么」，但**问不出「刚才那 5 秒里哪一段慢」**。

之前唯一的性能入口是 MCP 的 `analyze_performance`（起 MCP server、走
JSON-RPC）—— 那是给 AI 的形态：一次调用一个来回。人要的是「开着窗口看
一会儿数字」。

| 形态 | 适合 |
|---|---|
| `zephyra perf profile` | **取一次报告**（百分位 + fps）—— AI 与 CI 都能用 |
| `zephyra perf watch` | **盯着看**（实时表）—— 人盯着窗口时用 |

### 数据从哪来

两条路都只用**已有的**桥命令，不新增协议：

- `stats --json` → 最后一帧的快照（四边形数、脏区、**分段耗时**、已渲染帧数）
- `profile_start` / `profile_report` → 宿主侧的**逐帧**采样与百分位

**分段耗时是 core 自己测的**（新增 `zephyra_core::frame::StageTimings`）：
`diff` / `style` / `scan` / `layout` / `paint` / `total`。

为什么让 **core** 测而不是宿主测：那五段里有四段在 core 里（对账、样式、
脏区扫描、布局、绘制），宿主只看得到「整帧多久」。而「这一帧慢在哪」是
**读出来的**，不是猜的。

每段一次 `Instant::now` 在 release 下约 20 ns，五段共约 100 ns ——
相对一帧 30~80 µs 是 0.1% 量级。**恒开**的好处：任何一次「这帧为什么慢」
都能直接读到分段数字，不需要改代码重编。

---

## 三、调试器：`zephyra debug`

| 子命令 | 干什么 |
|---|---|
| `watch <命令…>` | 反复问一条桥命令，**只在答案变了时**打印 |
| `tree [路径] [--depth N]` | 打印一棵子树（客户端裁剪） |
| `snapshot <文件>` | 把当前布局树存成一份 |
| `diff <文件>` | 与存的那份比 —— 哪些节点变了 |

`watch` 的形态是「**反复问同一个问题，只在答案变了的时候才说话**」——
这正是调试真正难的地方（「这个按钮的 `Pressed` 属性到底有没有被写上？」
「我点了它，为什么画面没变？」）。一条 `props` 的输出有十几行；每 200 ms
打一次，一天下来是几十万行噪音，而要看的恰恰是**变化的那一刻**。

`diff` 按**路径**索引（不是整行）：树的缩进会变（深度变了），而路径不会。

---

## 四、量出来的三个真问题

这三个都不是「设计时想到的」，是**拿 CLI 当 AI 用**才撞出来的。

### （一）桥只支持一个客户端 ⇒ 「盯着看」与「动手改」不能同时

v1 的桥只有一个连接槽：已有客户端在位时**不 accept** 第二个连接。

实测症状：`zephyra debug watch "props 0.1.0.2"` 占着连接，于是
`zephyra bridge "click_path 0.1.0.2"` **根本发不过去** —— 表现是
「点了没反应」，而应用那边一切正常（退出码 0、输出看着像样）。

调试器的用法恰恰是「一边盯着、一边点」。所以改成**多连接**：

- `conn: Option<Connection>` → `conns: Vec<Connection>`（上限 8）；
- `wait_frames` 的延迟应答**每个连接一份**（共用一份会让「A 等 5 帧、
  B 等 1 帧」时 B 的应答被 A 吞掉）；
- `on_bridge_disconnect` 的判据从「少了一个」改成「**一个都不剩**」——
  「A 正拖着、B 断了」不该把 A 的按下态清掉。

### （二）宿主在有调试器连着时**空转到 2500 Hz**

宿主的事件循环只有两档：`Poll`（立刻返回，全速空转）与 `WaitUntil(t)`。
TCP 数据**唤不醒** Wait —— 所以「有客户端」就意味着「只能 Poll」。

实测：连着一个 `perf profile` 时，事件循环跑到 **2500 Hz**，而应用其实只画了
**9 帧**。CPU 从「无客户端时的 0 s/3 s」涨到「有客户端时的 ~0.5 s/3 s」
（≈17% → 0.5%）。

判据改成两档：

- **有未处理的数据** ⇒ Poll（数据在流，全速才对）；
- **有客户端但没数据** ⇒ `WaitUntil(now + 10 ms)`（命令最多晚 10 ms）。

⚠ 判据只能是「**缓冲区里有没有字节**」—— 数据到达时主机**不知道**，只能等
下一次醒来去读。这就是那 10 ms 的来源。

### （三）性能采样把「事件循环 tick」当成「帧」

`perf profile` 报「**2500 fps**」，而 `stats` 里的已渲染帧数是 **9**。

根因：`profiler.record()` 在宿主帧循环里**无条件**调用 —— 而帧循环在没有
重画时也会跑（桥连着时更是高频）。于是「没画任何东西的 tick」被当成了帧。

改法：宿主只记**真的重画了的帧**（`redrawn`）。于是：

- 静止的应用 ⇒ `frames=0`，并明确说明「应用是静止的（那本身就是结论）」；
- 有交互 ⇒ 报的是**真实的帧率**与**真实的分段百分位**。

顺带修一处「小谎」：`MCP screenshot` 的回执报的是**线上**字节数（BMP），
而落盘的是 PNG —— 报「3360054 bytes」而文件只有 743039 字节，会让人以为
文件被截断。改成报**落盘后**的大小。

### （四）`stats` 在静止的应用上报 `quads=0`

跳过的帧画了 **0** 个四边形，而宿主**无条件**把 `report.quads` 写进
`last_stats` —— 于是 `stats` 报 `quads=0`，读的人以为「屏幕上是空的」，
而画面上明明有 73 个节点的界面。

规则（`zephyra_fx::bridge::stats_after_skipped`）：**跳过的帧保留上一次
画出来的** `quads` / `skipped` / `damage`，其余（分段耗时、脏节点数、
悬停项）取这一帧的 —— 那些是「这一帧发生了什么」。

---

## 排除的假设

- ❌「`--bench` 报 62% 是有代码没被计时。」——不是。是**报表自己**漏了
  `cache_us`（那五段在 `FrameCache` 引入后恒为 0）。真相：占比恒为 100%，
  从来没漏过。
- ❌「每帧 390 次分配是 core 的问题。」——一半一半。core 侧（对账 + 样式
  注入）占约 99 次，改完剩 291；剩下的主体是**声明树重建**（应用侧的
  `Element` 树），那是架构问题不是 core 问题。
- ❌「`PropertyValue::Filter` 的克隆是那 291 次分配里的大头。」——不是。
  gallery 稳态下的滤镜链是**空**的（空 `Vec` 克隆不分配）。实测分配数
  **没有变**（291 → 291），省下的是克隆动作本身（`style` 段 8 → 7 µs）。
  受益的是**稳态下带着非空滤镜链**的应用。
- ❌「多连接会不安全（一个客户端的 `wait_frames` 会被另一个吃掉）。」——会，
  所以 `pending_wait` 是**每个连接一份**，不是共享一份。
- ❌「`on_bridge_disconnect` 该在**任一**客户端断开时触发。」——不该。
  「A 正拖着、B 断了」时触发会把 A 的按下态清掉 —— 那是 B 的锅。
  判据是「**一个都不剩**」。
- ❌「`request_inner_size` 回 `None` = 改窗口失败。」——错（上一轮已记）。
- ❌「性能采样要连「没重画的帧」一起统计，才算完整。」——不算。那会把 fps
  撑成假数字（2500 vs 9）。要问「静止时每帧花多少」，看 `stats` 的
  `stages` 字段（core 每帧都测）。

---

## 结论

- **core 的每节点每帧分配是能消掉的**：对账的键清单与样式缓存查找各一处，
  合计每帧少 99 次分配。整帧 −16%。
- **「这一帧慢在哪」该由 core 测**：分段耗时进 `stats`，于是性能排查从
  「猜」变成「读」。
- **桥要多客户端**：调试器的基本用法就是「一边盯着、一边动手」。
- **宿主不能因为有调试器就空转**：Poll vs WaitUntil 分档，实测 CPU
  17% → 0.5%。
- **性能采样只能记真的重画了的帧**：否则 fps 是假数字。
- **`stats` 要说清「画面上有什么」**：跳过的帧保留上一次的 `quads`。

---

## 门禁

- `zephyra-fx`（166 条）：
  - `two_clients_can_both_send_commands` —— 两个客户端各自的命令都能到达。
  - `pending_work_is_false_for_an_idle_connected_client` —— 「连上但没数据」
    ≠「有活要干」（判据错了就 2500 Hz 空转）。
  - `a_skipped_frame_keeps_the_last_drawn_quads` —— 跳过的帧保留 `quads`。
  - `profile_report_carries_fps_and_per_stage_percentiles` —— 报告带 fps 与
    分段百分位。
  - `profile_report_explains_an_idle_app` —— 没有采样时明说「静止」，不印 0 fps。
  - `stage_diff unavailable` —— 分段不可用时明说，而不是印一排 0。
- `zephyra-cli`（44 条）：
  - `perf_and_debug_are_registered_and_documented` —— 两个命令在表里、
    帮助里能看到。
  - `value_flags_cover_every_option_the_commands_document` —— **扫整张命令表**：
    凡写成 `--xxx <...>` 的都必须在 `VALUE_FLAGS` 里（不列进去 ⇒ 取值会被
    当成位置参数 ⇒ 被拼进桥命令里 ⇒ 应用收到一条莫名其妙的命令，而 CLI
    一切正常）。
  - `perf_and_debug_value_flags_are_parseable` —— 取值不漏进位置参数。
  - `debug_watch_takes_the_whole_command_as_positional` —— `debug watch` 的
    命令是**位置参数的全部剩余部分**。
  - `commands::debug`（4 条）：`subtree_keeps_the_root_and_its_descendants` /
    `subtree_does_not_match_a_sibling_with_a_longer_number`（`0.1` 不能匹配
    `0.10`）/ `depth_limits_the_subtree` /
    `index_is_keyed_by_path_not_by_the_whole_line`。
  - `commands::perf`（2 条）：`stats_json_is_parsed_by_field_name` /
    `missing_stages_degrade_to_zero`。

---

## 真机复核（gallery，D3D12，release，1400×800，`ZEPHYRA_DEBUG_PORT=7879`）

```text
perf profile --seconds 1（静止）    → frames=0 +「应用是静止的（那本身就是结论）」
perf profile --seconds 2（点击 3 次）→ frames=4 fps=1.8
                                     stage_diff p50=133us stage_layout p50=70us stage_paint p50=914us
perf watch（拖动滑块）              → core_us 从 165 → 66 → 122（帧成本随交互变化）
debug watch "props 0.1.0.3" + 点击   → +886ms 检出变化：
                                       style nav-item → style nav-item / selected / hovered
                                       Background #00000000 → $control_tertiary
debug snapshot + 点击 + debug diff  → 新增 17，消失 11，变了 33（共 99 → 105 个节点）
CPU（连着调试器，应用静止）        → 0.016 s / 3 s（修前 ≈ 0.5 s / 3 s）
```

# 语法宪法

> 本文件是仓库**唯一的语法权威**。AGENTS.md 只放决策过程与指针，不复制细节。
> 新增任何语法表面，必须先在这里登记并说明「现有五种归属为何装不下」；未经登记一律拒绝。

---

## 1. 四所有权（不可越界）

| 关注 | 唯一归属 | 禁止 |
|---|---|---|
| 状态值 + 流转规则 | Rust 控件模块（`ZephyraUI/src/checkbox.rs` / `toggle.rs` …） | 不建节点树、不写外观值 |
| 节点树 / 分类名 / key / 资源引用 | `ZephyraUI/ui/controls.zypui` | 不写颜色、尺寸、动画关键帧 |
| 颜色 / 尺寸 / 圆角 / 字体 | `ZephyraUI/controls-*.zstyle` | 不写结构、状态、行为；颜色必须 `@token` |
| 状态存在（跨帧） | 宿主应用 | 不进模板、不进控件库 |
| 时间 → 帧 | `SvgTimeline`（`zephyra-svg-anim`，纯数据） | 不持有节点、不进 core |
| 部署配置 | `ZephyraUI.json` / `ZEPHYRA_*`（仅 debug 构建） | 不放外观或结构 |

`zephyra_core::theme::roles` 是 token 名单的**唯一权威**；`ZephyraUI/src/tokens.rs` 与
`.zstyle` 的 `@token` 都只是镜像，由门禁逐项比对，拼错在 `cargo test` 阶段报红。

## 2. 六步决策过程

新增一个「可声明的东西」时，按顺序问，命中即停：

1. 是外观值？→ `.zstyle`，颜色走 `@token`
2. 是结构 / 分类名 / key / 资源引用？→ `.zypui`
3. 是状态值或流转规则？→ Rust 控件模块
4. 是时间 / 程序化逻辑？→ 应用侧 driver，不进模板
5. 是部署配置？→ `ZephyraUI.json` / env（debug 限定）
6. **都不是** → 先别加语法。能不能用现有语法的「引用」表达（id / `@token` / class 名 / 资源路径）？
   能就用引用；不能，才在本文件登记新表面并说明理由。

第 6 条是防漂移的核心：**新语法必须有书面豁免，默认不允许新增**。

## 3. 语言表面表

| 表面 | 唯一职责 | 权威 | 验证 |
|---|---|---|---|
| `.zstyle` | 外观值（CSS 风味但自有语义） | 自身 + `@token` | `zstylec` 编译、`props_map.rs` 报错 |
| `.zypui` | 结构、分类名、key、资源引用 | 自身（唯一结构源） | `zypuic` 编译 + `tests/gates.rs` |
| `src/*`（控件） | 状态值 + 流转规则 | 自身 | 单测 |
| `ui/animations/*.svg` | 动画资源（SMIL 数据） | 自身 | 能力矩阵测试 |
| `SvgTimeline` | 时间 → 帧（纯数据） | 自身 | 能力矩阵测试 |
| 应用侧 driver | 时钟推进、progress 绑定 | 自身 | 端到端测试 |
| `core::theme::roles` | token 名单 | 唯一权威 | `tokens.rs` 镜像门禁 |

## 4. 逐项裁决

| 现存现象 | 裁决 | 依据 |
|---|---|---|
| `.zstyle` 的 CSS 风味、`superellipse()`、`10px` | **保留** | 刻意「CSS 写法 + 我们语义」，前端经验可迁移；拼错报错 |
| `@token` 四处镜像 | **保留** | 唯一权威明确，镜像有门禁比对 |
| class 词序语义 | **废除** | 样式匹配是集合判断（`zephyra-style/src/ir.rs`），顺序只是门禁强加的 |
| `label_key` / `label` 双通道 | **保留，但编译期强制** | 静态 vs 动态是真实语义差异；不再靠文本扫描与记忆 |
| XML 属性里的三元 / `{}` 插值 / `tr()` / `hover(key)` | **保留并正名** | 它已是 `zypui-compile` 的真实解析器，名为「zypui 表达式语言」，见第 7 节 |
| `CLASS_*` 常量 ↔ `.zstyle` 选择器字符串镜像 | **加门禁** | 拼错静默失效，必须有测试 |
| 运行时 `@keyframes` / CSS 动画语法 | **禁止** | 运行时只有 SMIL；CSS 仅作构建期导入工具的输入 |

旧语法**不做兼容层**：不符合裁决的模板直接报错或门禁报红。

## 5. 动画语法规则

- **运行时只有 SMIL**（`zephyra-svg-anim` 解析 `<animate>` / `<animateTransform>` /
  `<animateMotion>`），加少量扩展字段（`direction` / `backwards` / `steps(n)`）。
- **CSS `@keyframes` 只作构建期导入**：`zanim import` 把 CSS 翻译成普通 SMIL 文本，
  输出与手写 SMIL 走同一条运行时路径。库在主仓（共享 IR），bin 在 CLI 仓。
- **`.zypui` 的 `animation` 只接受 id / 路径**，和 `icon="save"` 同类；绝不接受语法串。
- **预制动画颜色必须 `@token`**，禁止 RGBA 硬编码；viewBox 统一 24×24，接入层缩放到节点矩形。
- **`animation-progress` / `animation-play-state` 是 `.zypui` 属性**，驱动来自应用绑定；
  不进 `.zstyle`，`affects_layout = false`。
- **`transition` 反向 / 打断**由应用侧 driver 用 `seek()` 实现，不新增语法。
- **`steps(n)` 由 `zanim` 构建期导入器映射成 `calcMode="discrete"` 阶跃**；
  运行时引擎没有 steps 变体。当前实现按关键帧离散跳变，尚未做完整的 n 段等分展开——
  需要精确 `steps(n, end)` 语义时，手写 SMIL 的 `keyTimes` 更直接。
- **时钟归应用**：应用用 `set_anim_clock_ms()`（gallery）注入时钟，`HostWindow::
  set_post_diff()` 挂 `AnimDriver`；钩子在 `FrameCache` 的 diff + 样式之后、布局之前
  运行，只写 paint 属性。core 的逐字节 diff 天然检测变化，**不改 diff 决策**。
  测试不注入时钟 ⇒ progress 固定 0 ⇒「无输入 diff 是 no-op」成立。

## 6. 动画能力清单（"99%" 的可验收定义）

按**真实 UI 使用频率**统计，不是 CSS 规范条目数。清单外的明确不支持，不静默缺失。

### Tier A — 引擎已具备，回归测试锁定

duration / delay（含负值）、iteration-count、cubic-bezier、linear/discrete、
fill-mode: forwards、add/accumulate 合成、end/min/max/restart、
opacity / transform / fill / stroke / path morph / offset-path。

### Tier B — 本计划补齐

`direction`、`backwards/both`、`play-state`、`steps(n)`、transition 反向打断、
token 颜色替换、viewBox 缩放、滚动 progress 经 `seek()` 驱动。

### Tier C — 明确不支持

`filter: blur/brightness/saturate`、`box-shadow/text-shadow`、渐变 stop 动画、
文本属性动画、布局盒属性动画（width/height/padding/margin）。
理由：要改三个渲染后端着色器，或撞 `affects_layout` 门禁；真实占比小。

每条 Tier A/B 都有一行「构造 SMIL → `seek()` → 断言 `SvgFrame` 字段」的测试。

## 7. zypui 表达式语言（EBNF）

`Expr` 定义在 `zypui/src/expr.rs`，求值发生在**每帧实例化期**（框架每帧重建 Element 树），
不是常驻响应式属性引擎。形状借鉴 Slint，但无属性间引用（那会要求布局后求值，与本框架冲突）。

```ebnf
expr        = ternary ;
ternary     = or ( "?" expr ":" expr )? ;
or          = and ( "||" and )* ;
and         = equality ( "&&" equality )* ;
equality    = relational ( ("==" | "!=") relational )* ;
relational  = additive ( ("<" | "<=" | ">" | ">=") additive )* ;
additive    = multiplicative ( ("+" | "-") multiplicative )* ;
multiplicative = unary ( ("*" | "/" | "%") unary )* ;
unary       = ("!" | "-") unary | postfix ;
postfix     = primary ( "." IDENT )* ;
primary     = STRING | NUMBER | BOOL | IDENT | "@" IDENT
            | CALL "(" ( expr ( "," expr )* )? ")" | "(" expr ")" ;
```

- `STRING` 用单引号；`NUMBER` 为 f32；`BOOL` 为 `true` / `false`。
- `IDENT` 沿作用域链解析（组件实参 / `for` 变量），未命中在 `zypuic` 编译期报错。
- `@IDENT` 是宿主绑定，未绑定求值为假（编译期只警告——绑定表在宿主手里）。
- `CALL` 仅允许白名单：`tr` / `min` / `max` / `range` / `str` / `at` / `hover` / `press` / `focused`。
- 内置裸名：`hover` / `press` / `focused` / `key`。
- 文本通道（**硬 break，无兼容层**）：`TEXT_ATTRS`（`label` / `title` / `text` /
  `placeholder` / `hint` / `desc` / `tip`）的值**必须是 `{…}` 表达式或绑定**；
  裸字面量在 `zypuic` 编译期直接报错。确需字面量（符号、数字、语言自称）就显式写成
  `{'English'}` / `{'+'}`。`label_key` 刻意不在表内：它是语义 key 通道，由控件
  内部 `tr()` 翻译；i18n 完整性由 gallery 的 `every_label_key_exists_in_every_language`
  门禁按**值**检查。旧的「源码文本扫描 + 按值白名单」门禁已删除——编译器接管后它
  既冗余又脆弱。

## 8. 防偏移机制

- **书面**：本文件是唯一语法权威；AGENTS.md 只放决策过程与指针，不复制细节。
- **可执行**：第 4 节每条裁决都有具名测试；削弱门禁会让负向测试红。
- **流程**：新增语法表面必须先在本文件登记并说明「现有五种归属为何装不下」。
  这是评审规则，无法自动化——AGENTS.md 明确写「未经登记的新语法一律拒绝」。

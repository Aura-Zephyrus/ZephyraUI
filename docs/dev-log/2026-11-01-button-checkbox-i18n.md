# Button / CheckBox 与 i18n 深度融合

> 2026-11-01。控件模板的文案实参收**语义 key**，翻译发生在**控件内部**。

## 现象

gallery 的 Button / CheckBox 演示页，切语言时**有的字变了、有的没变**。

真机 dump（`--route reference --family 1`）：

```
反馈      act:feedback
Toggle me state-check          ← 英文，不跟着变
全选      group:checks
Enable feature  check:alpha
Send analytics check:beta
```

`反馈` 走了 `tr('action.feedback')` ⇒ 跟着语言变；`Toggle me` 是模板里
**硬编码的英文字面量** ⇒ 永远不变。

## 排查

### 1. 为什么现有门禁没拦住

`every_tr_key_used_in_source_exists_in_every_language` 查的是「**用到的 key**
有没有译文」—— 单向的。`label="Toggle me"` 里的 `Toggle me` **不是 key**，
压根不会被这条门禁看见；而裸字面量本身也从来不被查。

翻遍模板：约 30 处 `label=` / `title=` / `text=` 是硬编码英文。

### 2. 控件层那条纪律没兑现

`ZephyraUI/Cargo.toml` 的注释明写：

> 控件的**文案**归应用，不归这里 …… 控件只发 key，文案表是应用的。
> 所以 `zephyra_core::i18n`（BCP47 / Messages）**不**在这里出现。

而控件模板里是：

```xml
<text key="label" text="{label}" class="…"/>
```

`label` 收的是应用**已经翻译好**的文案 —— 控件层从头到尾没见过 key。
「控件只发 key」这条纪律写了很久，**但没有兑现**，于是「控件层不发面向用户
的文案」这条更严的纪律**没法做成门禁**（判据压根不存在）。

### 3. 复选框的状态模型也是「文案」而非「key」

`ZephyraUI/src/checkbox.rs` 的 `Checkbox { key, label, state, disabled }` ——
`label` 是文案。改成 key 时**类型没变**（都是 `&'static str`），于是静默改语义
会让「英文原文当 key 去查表」正好显示成原文，**画面上一模一样**。
⇒ 字段与方法一起改名（`label` → `label_key`），让**编译器**把旧调用点指出来。

## 结论

### 控件层

新建 `ZephyraUI/src/i18n.rs`：

- `NAMESPACE = "zephyra"` —— 控件层 key 的前缀（与应用层 `ui.*` 隔开）；
- `CHECKBOX_SELECT_ALL = "zephyra.checkbox.select-all"` —— 复选框组组头的
  **控件语义**（组头 = 全选 / 全部不选，这个含义由控件定义，不由应用定义）；
- `BUILTIN_TEXT` —— 控件层自带的**三语言默认译文**；
- `KEY_PARAMS` —— 契约表（组件 → 哪个实参收语义 key）：门禁的真相来源；
- `merge_builtin(app)` —— 并表，**应用条目优先**（单向）。

控件模板（`ZephyraUI/ui/controls.zypui`）：

```xml
<component name="Button" params="label label_key icon variant material">
  <text key="label" text="{label_key ? tr(label_key) : label}" class="…"/>
```

**两条通道**：`label_key` 收语义 key（控件内部 `tr()`），`label` 收已翻译的
动态文案。不能让 `label` 独占 key 语义 —— 动态文案（「删除 3 项」这种每帧
不同的）就没地方放了，那是真实的功能回退。

### 应用层（gallery）

- `CATALOG` 新增 32 键 × 3 语言 = 96 条（模板里剩下的裸散文全量入表）；
- `messages()` 改成 `OnceLock` 缓存（**性能**：见下）；
- `CheckBoxGroup` 的组头 key 走**宿主绑定** `@check_group_header_key` ——
  Rust 模型是唯一真相源，模板不必再写一遍 key；
- `CHECK_LABELS` → `CHECK_LABEL_KEYS`（存 key 不存文案）。

### 性能：`tr()` 在每帧热路径上

`tr()` 是**逐元素**调的（模板里每个文字元素一次），而 `Messages` 内部是
`BTreeMap` —— 每次重建 = 200 多次节点分配 + 一次整表分配。
控件模板里的 `tr(label_key)` 把每帧 `tr()` 的次数**翻了一倍**。

⇒ `messages()` 改 `OnceLock`（文案表是编译期常量，建一次就够）。

## 门禁

四条新的（`gallery/tests/gates.rs`）：

| 门禁 | 判什么 |
|---|---|
| `control_layer_templates_emit_no_user_facing_text` | 控件层模板**零**裸文案（无豁免） |
| `app_templates_emit_no_user_facing_text` | 应用模板的裸文案只允许命中豁免表 |
| `every_label_key_exists_in_every_language` | 每个 `label_key` 在每门语言下都有译文 |
| `the_control_layer_key_param_contract_matches_the_template` | 契约表与模板文本对得上 |

加一条运行期的：`switching_the_language_changes_the_button_and_checkbox_labels`
—— 同一棵树切语言，按钮与复选框的**可访问名**（= 画面上的字）必须变。

豁免表 `LITERAL_ALLOWLIST` 按**值**列（不按行号 —— 改一行模板就全乱），
只收本该是字面量的东西：**命令行**、**符号**（`+` / `-`）、**数字**、
**语言自称**（English / 中文 / 日本語 —— 自称不是翻译，见
`docs/developer/i18n.md` §5.2）。

## 途中踩到的三个坑

### 1. `--lang` 在窗口路压根不读（真 bug）

`--route` 与 `--family` 都已经因为「只在一边解析」栽过。`--lang` 是第三次 ——
而且它恰好是**最需要在窗口路生效**的那个：用途正是「在本机（系统语言
zh-Hans）看另一种语言」。

真机症状：`--lang en` 开窗口，看到的仍是中文。

### 2. 就算读了，系统协商也会覆盖它

`Gallery::on_input` 的**系统语言协商**在第一帧把 `lang` 改回系统那个。
两个坑叠在一起 ⇒ 修法把不变量收进库：`Gallery::force_language(idx)` 同时置
「已协商」，两条路都调同一个 `apply_forced_language`。

**为什么这个 bug 特别难查**：在系统语言恰好就是目标语言的机器上完全看不出来。
而 `--lang en` / `--lang ja` 的用途正是「在本机（中文）看另一种语言」——
也就是这个 bug **恰好在它唯一有用的场景里发作**。

### 3. 门禁自己先红了

`the_control_layer_key_param_contract_matches_the_template` 第一次跑就红：
「组件 CheckBoxGroup 的体里没有 `tr(label_key)`」。

门禁写错了：`CheckBoxGroup` 是**转发**（把组头的 key 原样交给内层 `CheckBox`），
不是直接翻译。⇒ 规则改成「直接翻译**或**转发都算用上了」，但转发必须有
**终点**（契约表里至少有一个组件直接 `tr(param)`），否则「大家都在转发、
没人翻译」也能过 —— 那正是这条门禁要挡的。

## 验收

- `cargo test --workspace` 全绿（86 个测试二进制）；
- `cargo fmt --all -- --check` 与 `cargo clippy --workspace --all-targets -- -D warnings` 干净；
- gallery 门禁 50 条全绿（原 44 + 新 6）；
- **真机 UIA 跨进程 dump**（三种语言）：

  | 语言 | 组头（控件层自带） | 子项（应用表） | 按钮 |
  |---|---|---|---|
  | en | Select all | Enable feature / Send analytics / Beta channel / Locked option | Feedback |
  | ja | すべて選択 | 機能を有効にする / 分析を送信する / ベータチャンネル / ロックされたオプション | フィードバック |
  | zh-Hans | 全选 | 启用功能 / 发送统计 / 测试通道 / 锁定的选项 | 反馈 |

  组头那一句**应用一个字都没写** —— 它来自控件层的默认译文，且照样跟着语言变。

- 三态与无障碍未受影响：组头 `Indeterminate`、`check:alpha` = `On`、
  `check:beta`/`check:gamma` = `Off`、`check:locked` = `Off` 且 `enabled=false`。

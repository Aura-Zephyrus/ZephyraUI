# ZephyraUI —— 控件层

一句话：**控件只发结构与分类名，外观全在 `.zstyle`。**

它是一个**库**，不是应用。它不知道颜色、不写死尺寸、不决定一个控件出现在哪一页。
应用（`gallery/`）把控件摆进页面，并决定那一页长什么样。

## 这个 crate 里有什么

| 部分 | 内容 | 谁在消费 |
|---|---|---|
| `ui/controls.zypui` | 54 个控件的**结构**（声明式组件模板） | 应用的 `build.rs` 直接读这份文件编译 |
| `controls-*.zstyle` | 16 份控件样式表，`src/styles.rs` 用 `include_str!` 内嵌 | `STYLESHEETS` |
| `src/tokens.rs` | `@令牌` 名单镜像（对 `Theme::role()` 逐个校验） | `tokens::ALL` |

**没有 Rust 控件构造器。** 曾经有 59 个（`src/controls/*.rs`，1900 行），与
`.zypui` 逐字节 parity；全仓除 parity 测试外无人消费，却让每个控件改一次要改两处，
已删。结构的真相只有 `.zypui` 这一份。

## 依赖边界

```
zephyra-core   零依赖，禁 unsafe
     ↑
ZephyraUI     本 crate：控件样式表 + 令牌 + 结构声明（零后端）
     ↑
gallery        应用：编译 controls.zypui + 页面编排 + 路由 + i18n + 宿主接线
```

**不依赖任何渲染后端。** 不出现 `D3D` / `Vulkan` / `wgpu`，也不做 `#[cfg(windows)]`
分支挑后端 —— 后端由宿主 crate 的特性开关选，一个二进制只会有一个。

## 结构怎么被应用用上

应用在自己的 `build.rs` 里把**本目录的** `ui/controls.zypui` 与自己的页面模板
一起编译成一个二进制工件（见 `gallery/build.rs:15`）：

```rust
let controls = manifest.join("../ZephyraUI/ui/controls.zypui");
let shell    = manifest.join("frontend/shell.zypui");
zypuic::compile(&[(controls_name, controls_src), (shell_name, shell_src)])
```

两个文件必须在**同一张组件表**里解析：页面里的 `<Button/>` 引用与控件库里的
定义要能对上。运行时 `include_bytes!` + `decode`，零解析。

## 三条硬约定

### 1. 源码里没有外观

控件模板只写结构（标签）、`key` 与**分类名**。不写
`WIDTH` / `HEIGHT` / `BACKGROUND` / `CORNER_RADIUS` / `PADDING` / `SPACING` …

**唯一例外**是 `fill:*` 前缀的 key（进度条填充、滑条 thumb、滚动 thumb）：它们的
尺寸是**内容**，不是外观。见 `gallery/tests/gates.rs::only_fill_nodes_carry_local_width`。

门禁按**整棵树 × 全部页面 × 全部交互态**判，比你想的更容易红。

### 2. 文案归应用

控件**不发**可见文字，除了调用方**显式传进来**的那些。控件层里不许出现
「日一二三四五六」这种写死的中文表头（`DatePicker` 的星期头是参数，就是这条的产物）。

控件也不做 i18n：`zephyra_core::i18n` 不在这里。

### 3. 分类名必须存在

拼错分类名时**什么都不发生** —— 不报错、不崩、只是退回默认外观。
而「退回默认值」看起来就像「样式表没生效」，是本层最隐蔽的失败模式。

所以模板里的每个 `class="…"` 词都得在 `controls-*.zstyle` 里有对应规则，
门禁逐个查（`every_class_name_the_source_emits_exists_in_the_stylesheet`）。

## 控件清单

54 个组件，见 `ui/controls.zypui`（按族分段，每段有注释说明度量出处）：

Button / ToolButton / PillButton / RoundButton / Hyperlink / InfoBadge / DotBadge /
Card / SettingCard / ClipCard / Expander / TextLabel / Caption / CheckBox /
CheckBoxGroup / RadioButton / ToggleSwitch / Slider / ProgressBar / Segmented / NavItem / Menu /
MenuItem / MenuBar / NavSubItem / SelectorBar / ErrorPage / ProgressRing / InfoBar /
Toast / ToolTip / BusyIndicator / PageIndicator / DropDownButton / ComboBox /
SpinBox / ColorPicker / ListView / TableView / SwipeView / Avatar / SettingItem /
SettingExpander / AcrylicCard / CornerOverlay / Dialog / DatePicker / TimePicker /
TextField / TextArea / ScrollableTextArea / AutoSuggestBox / TextInputMenu …

⚠️ 控件名代表结构/视觉形态，**不保证**拥有原生控件的全部输入、弹层、滚动、焦点或
动画行为。已盘点的能力缺口和后续重写候选见
[控件能力缺口与重写清单](../docs/todo/control-capability-audit.md)。

### 状态模型在 Rust 侧（`src/checkbox.rs`）

复选框是第一个带**真三态**的控件族，于是它的状态模型也住在控件层：

| 类型 | 职责 |
|---|---|
| `CheckState` | 三态枚举（`Unchecked` / `Checked` / `Indeterminate`）+ 分类词 + 流转规则 |
| `Checkbox` | 单个复选框（key / label / 状态 / 禁用） |
| `CheckboxGroup` | 一个三态组头 + 一层两态子项；组头状态**由子项聚合派生** |

理由写在 `src/checkbox.rs` 的模块文档里，一句话版本：**状态值与流转规则是
规则，不是数据** —— 规则必须有一份权威实现，否则每个 app 各写一遍、各错一遍；
而状态**存在哪**（跨帧留存）永远是宿主的事。

## 样式表

16 份 `controls-*.zstyle`，由 `src/styles.rs` 用 `include_str!` 内嵌。

**内嵌而不是运行时读磁盘**：让「改了表忘了重编」变成**编译错误**而不是运行时的
静默失效。门禁吃的是**同一份** `STYLESHEETS`，所以不存在「图对、门禁绿」。

⚠ 加载顺序即优先级（`zstylec` 对同特异性规则是「后写的赢」）：`base` 必须最先，
它被几十个控件继承，挪到后面会让所有控件的图标与字号一起变。顺序表手写在
`styles.rs` 里 —— 漏一张表不会报错，加表时记得同步。

### zstylec 的硬限制

属性表只有 **32 项**，且：

| 限制 | 后果 |
|---|---|
| 没有 `min-height` / `margin-*` | 留白只能靠 `height` + `padding`，或 `translate-y` |
| `padding` 只吃单值 | `padding: 10px 16px` 解析失败 |
| 不支持 `rgba()` | 用 `#rrggbbaa` 八位十六进制 |

## 测试

```bash
cargo test -p zephyra-ui            # 本 crate：令牌镜像校验等
cargo test -p zephyra-gallery       # 控件的真正判据在这里
```

本 crate 自己只有少量单测；**控件的判据在 `gallery/tests/gates.rs`**（按整棵树判，
遍历全部页面 × 全部交互态，不按函数判 —— 逐函数查只能查到「我记得去查的那个」）。
另有 `interaction_tests.rs`（命中 / 捕获 / 拖动离位不触发点击）与
`hover_layout_tests.rs`（hover 改尺寸要重排）。

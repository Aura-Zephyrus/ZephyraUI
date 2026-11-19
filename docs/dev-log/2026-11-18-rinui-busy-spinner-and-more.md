# RinUI 进度环纯圆化 + BusyIndicator 1:1 + 再补 10 控件

> 2026-11-18 · `ZephyraUI` / `gallery`
> 参考 `E:\Rin-UI`（RinUI 源码）+ `https://ui.rinlit.cn/zh/StatusAndInfo/BusyIndicator.html`

## 现象

用户要求三件事：① 进度环改成纯圆；② BusyIndicator 按 RinUI 1:1 还原；③ 再继续做 10 个控件。

## 排查与实现

### 进度环纯圆

`.ring-track` 原用 `corner-radius: 158.4px` + `corner-shape: superellipse(1.5)` —— 画出来是
圆角方块。改成 `corner-radius: 30px`（半边长）+ `corner-shape: round`（= `superellipse(1)`），
得到真正的圆。同时按 RinUI `ProgressRing.qml` 去掉内嵌的百分比文本 —— 数值在 RinUI 里
由外部控件显示，环本身就是纯几何。

### BusyIndicator 1:1

读源码发现 `BusyIndicator.qml` **只有 11 行**：继承 `ProgressRing`，锁 `state = Running`、
`indeterminate = running`。也就是说它不是三颗点，而是一枚**旋转的圆端弧线**：

- `ProgressRing.qml:42-115`：`Canvas` 上一条 `lineWidth=6`、`lineCap="round"` 的圆弧，
  `size=56`、`radius=(min(w,h)-6)/2=25`；`startAngle` 两段 0→450→1080，`sweepAngle`
  0→180→0，每段 ≈1033ms（`progressBarAnimationSpeed 1550 / 1.5`），无限循环。
- 背景圆环 `backgroundColor` 默认 transparent。

本仓没有 Canvas 元素，但图标系统能解析 SVG 的 `M/L/A` 路径并按节点矩形等比缩放居中。
⇒ 按原版两段动画**采样 24 帧**，生成 24 枚 `busy-arc-N.svg`（外半径 24 / 内半径 18，
含圆端半帽），登记进 `icon::assets::REGISTRY`。`BusyIndicator` 模板改成一枚带 `icon`
的盒子，`BusyIndicatorState` 每 ≈86ms 推进一帧（2066/24），宿主把帧名缓存成 `&'static str`
绑定。三后端一行不改。

### 再补 10 个控件

照 RinUI 源码补状态模型（`zephyra_ui::catalog`）：`ToggleState`（ToggleButton/PillButton）、
`ToolButtonState`、`HyperlinkState`、`InfoBadgeState`、`ToolTipState`、`PageIndicatorState`、
`SettingItemState`、`SettingExpanderState`、`CardState`，加上新增的 `ToggleButton` 模板。
新路由 `catalog2` 一页展示全部 10 个，三语言文案齐全，点击全部接线。
关键语义照抄：InfoBadge `99+` / `count=-1` 空、PageIndicator 末页回绕、ToolButton 强调色、
Hyperlink 访问态、SettingItem 可点、SettingExpander 开合、Card 选中态。

## 排除的假设

❌「BusyIndicator 要新写绘制代码」—— 它完全复用 ProgressRing，只有状态不同。
❌「旋转弧线要后端新图元」—— SVG 路径 + 现有图标系统就够，24 帧采样平滑度已足够。
❌「10 个控件要新模板」—— 9 个模板已存在，只缺 Rust 状态模型与演示。

## 门禁踩坑

- `every_interactive_key_is_wired_or_whitelisted`：点击必须让**树指纹**（tag/key/class/text）
  变化 ⇒ ToolButton/Hyperlink 加 `highlighted`/`visited` 类，RoundButton 动作按钮加
  `checked` 绑定；按钮本身类变化即满足判据。
- `busy_indicator_advances_one_arc_frame_per_step`：f32 累加 24 帧会差一帧，测试里 +1ms 抵消。
- `busy-arc-N.svg` 在 `assets/busy_arcs.rs` 子模块里，`include_str!` 路径是 `icons/...`
  而不是 `assets/icons/...`。

## 门禁

- MCP 读回：catalog 页进度环为纯圆、BusyIndicator 为旋转弧线；catalog2 页 10 控件全部渲染。
- `cargo test --workspace`：**1735 passed / 0 failed**（+13 条新单测）。
- `cargo clippy --workspace --all-targets -- -D warnings` 0 警告；fmt 干净；doc 退出 0。

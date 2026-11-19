# RinUI 目录补充控件：9 个控件落地 + 选区贴字

> 2026-11-18 · `ZephyraUI` / `gallery`
> 参考 `https://ui.rinlit.cn/zh/components/`（RinUI v0.5.0，Fluent 风格 QML 库）

## 现象

用户截图：文本框选区的蓝条有 36px 高，比文字大一圈；并要求照 RinUI 目录继续完善控件，至少 7 个。

## 排查与实现

### 选区蓝条贴字

`box.selection` 高度写成 36px（控件高度），而锚点是含 padding 的内容原点 —— 蓝色块
上下各多出一截。改成文本行高 **18px**（与 `box.cursor` 同口径），圆角 4→2px。
MCP 截图复核：蓝条正好包住 `abc` 三个字。

### 9 个补充控件

RinUI 目录里大部分控件的**模板**（`controls.zypui`）与**样式**（`controls-*.zstyle`）
早已存在，缺的是「值 + 状态流转」的 Rust 模型与演示。本轮按 `Checkbox` / `ToggleSwitch`
的分工（状态在 `ZephyraUI/src`，宿主把 getter 喂给 zypui `@绑定`、把点击分派到 setter）补：

新模块 `ZephyraUI/src/catalog.rs`（9 个纯状态模型，无颜色/尺寸）：
`Progress`（进度条 + 进度环共用）、`InfoBarState`、`ColorPickerState`、`SpinBoxState`、
`BusyIndicatorState`、`ExpanderState`、`SelectorBarState`、`AvatarState`。

新路由 `catalog`（第 12 个导航项，`star` 图标）：一页 9 行 DemoRow 展示全部控件，
三语言文案齐全。点击接线：进度条点一下推进 10%、色板 16 格可点、数字框上下按钮、
信息栏关闭按钮、展开器标题、选择器栏三段。忙碌指示器按 120ms 推进帧序号，
`wants_frames` 只在 catalog 路由上为它泵帧（别的路由不空转）。

## 排除的假设

❌「要从零写控件模板」—— 模板与样式都在，缺的只是状态模型与演示，直接复用。
❌「新路由会撑爆导航栏」—— 实测 12 项最后一项底部 602 < 800，不溢出。
❌「忙碌指示器可以全局常驻泵帧」—— 门禁 `wants_frames_covers_every_animation_source`
   要求静止零帧；限定在当前路由才泵。

## 门禁踩坑

- `only_fill_nodes_carry_local_width`：进度条的 `fill:progress` 使计数 3→4。
- `every_interactive_key_is_wired_or_whitelisted`：门禁只认 tag/key/class/text 的树指纹，
  宽度变化不算「状态变化」⇒ 进度条点击额外切 `paused` 状态类；已选中的色块/选择器项
  再点本来就是 no-op ⇒ 加点击计数映射成 `tapped` 类；InfoBar 关闭按钮 `<else>` 分支
  换 `closed` 类（否则只有 `collapsed` 变化，指纹看不见）。
- 忙碌指示器 `wants_frames` 限定路由，否则静止页面空转。
- 三语言 i18n 全补（4 条门禁逐语言核数量与键）。

## 门禁

- MCP 读回：`lens:3` 玻璃趟；文本框输入实时显示、光标贴字、选区贴字；
  catalog 页 9 个控件全部渲染（截图复核）。
- `cargo test --workspace`：**1730 passed / 0 failed**（+8 条 `catalog_tests`）。
- `cargo clippy --workspace --all-targets -- -D warnings` 0 警告；fmt 干净；doc 退出 0。

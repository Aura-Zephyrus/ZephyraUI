# 2026-10-30-button-activation

> 现象：`Button` 是**纯受控声明**——只发 `interactive` 分类名，点击全靠应用在
> `on_input` 里用 `last_pressed`/`pressed` 过渡重建。它消费不到刚建好的地基：
> 不设 `FOCUSABLE` 进不了 Tab 序、没有 `focused` 类出不了焦点环、Space/Enter 不会
> 触发点击、`DISABLED` 也由样式决定而非控件语义。作为「用 Rust 写原生控件」的
> 第一个打样，Button 需要这些**键盘可达的原生交互行为**。

## 排查

先读了 WinUI 的 `ButtonBase`（`dxaml/xcp/dxaml/lib/ButtonBase_Partial.cpp` +
`dxaml/xcp/components/controls/KeyDownUp/inc/ButtonBaseKeyProcess.h`），确认键盘激活
**住哪一层**、**怎么触发**：

| 事实 | WinUI 做法 |
|---|---|
| 键盘激活住哪 | **控件自身**（`ButtonBase`），不在焦点系统。焦点系统只把 KeyDown/Up 路由给当前聚焦控件 |
| 触发键 | `IsPress` = Space / Enter（+ GamepadA）；本仓先做键盘的两个 |
| 触发时机 | 默认 `ClickMode.Release`——**抬起**才触发。Enter 故意走抬起：keydown 是同步的，处理函数里弹对话框会重入；keyup 是异步的，安全 |
| 与指针的关系 | 键盘按下 = `IsPressed`（等价指针按下），抬起 = `OnClick()`。**两路完全同构** |
| 防误发 | 一个闩 `m_bIsSpaceOrEnterKeyDown`；抬起时 `if (!m_bIsPointerLeftButtonDown)` 才触发 |
| 失焦 | `IsEnabled` 挡全部；`OnLostFocus` 清闩 |

**一个必须做的架构翻译**：WinUI 的控件是有状态 C++ 对象，`m_bIsSpaceOrEnterKeyDown`
是成员。我们的控件是**每帧重造的无状态 `.zypui` 模板**——没有 Button 实例能持有闩。
所以闩必须放进一个**跨帧核心状态机**，与 `FocusState` / `InputRouter` 同级。

**顺着 `pressed` 通道往下追**，确认「复用 pressed」是否成立：

- 模板 `press` 内置词 → `Host::holds(key)` → `Gallery.holds` 读 `self.pressed`
  （来自 `InputState.pressed` ← 宿主 `sync_input` 填 `router.pressed()`）。
- gallery 点击 = `last_pressed=Some → pressed=None` 重建。

所以「键盘激活复用 pressed」= 让 `ActivationState` 的闩并进 `pressed`，应用零改动，
且 Space 按住期间模板自动出按下态——与 WinUI「两路同构」一致。

## 排除的假设

- **「键盘激活住焦点系统里」**——WinUI 明确否掉：焦点系统只路由 KeyDown/Up，
  「Space/Enter 意味着点击」是 `ButtonBase` 自己的语义。若并进 `FocusState`，
  `FocusState` 就变成「既要管 Tab 又要管激活」，且 TextBox（也 focusable）打空格
  会误触发。聚焦回答「Tab 该不该停这」，激活回答「停这时空格/回车意味着什么」，
  两个状态机。
- **「ACTIVATABLE 并进 FOCUSABLE」**——TextBox 同样可聚焦，但它的 Space 是插入
  空格不是提交。合并会让焦点一落到输入框就误触发——焦点系统建好之后**最先**踩的坑。
  所以分开两个属性。
- **「键盘激活独立产出一条 Click 事件」**——语义更干净（键盘按下不显示按下态），
  但要在 `InputState` 里新开键盘 Click 通道，且 Space 按住不会有按下视觉（与 WinUI
  不同）。用户拍板**复用 pressed 通道**：键盘与指针一条道，应用零改动，Space 按住
  有按下态，最贴 WinUI 同构。
- **「键盘激活攒到下一帧 `sync_input`」**——`inject_click` 特意把 down/up 分两帧
  证明 pressed 过渡依赖帧边界。键盘激活必须**当帧**写闩（像 Tab / 指针），否则
  Space 按下那帧模板读不到按下态。
- **「焦点环走样式表匹配 FOCUSED 属性」**——`.zstyle` 选择器只认类型名 + 分类名，
  不匹配属性。焦点环必须靠模板发 `focused` 分类名（经新的 `focused` 内置词 →
  `Host::is_focused`），`.zstyle` 才能按分类名给 `@focus_ring` 换色。

## 结论

**core 新建 `input/activation.rs`，与 `FocusState` 平级。**

| 件 | 内容 |
|---|---|
| `ActivationState` | 跨帧持有 `held: Option<ElementKey>`（被键盘按住的节点） |
| `key_down(root, focused, ctx)` | 仅当焦点节点 `ACTIVATABLE` 且非 `DISABLED` 时闩住；自动重复不重进 |
| `key_up()` | 清闩（激活键抬起即退出键盘按下态） |
| `clear()` | 窗口失焦时清闩（与 WinUI `OnLostFocus` 清闩一致） |
| `on_key(input, root, focused, ctx)` | 按激活键边沿分派 down/up；非激活键返回 `None` |

**props 新增 `ACTIVATABLE`**（与 `FOCUSABLE` 刻意分开）。`style.rs` 加
`is_activatable` / `is_disabled`。`Key` 加 `Space` 变体（原落 `Key::Other`，激活是
热路径判断不该退化成比串），翻译器 `NamedKey::Space → Key::Space`。

**三宿主接线**（D3D11 / D3D12 / Vulkan 同构）：各持 `ActivationState`；键盘边沿
**当帧**喂 `on_key`；`sync_input` 填 `pressed: router.pressed().or(activation.held())`
（指针优先，防误发）；窗口失焦清闩。

**zypui / zypuic**：`focusable` / `activatable` 进 `BARE_ATTRS` 白名单 + 映射到
`props::FOCUSABLE` / `ACTIVATABLE`；`focused` 进 `BUILTINS` / `CALLS` / `RESERVED`；
`Host` trait 加 `is_focused`（带默认 `false`，实现者不必全改）。

**模板 + 样式**：`Button` 发 `focusable="true" activatable="true"` + `focused` 类；
`TextField` 移除旧的 `focused` 实参（改由框架焦点系统提供）并补 `focusable="true"`
（它可聚焦但**不** activatable——Space 打空格，正好验证两个属性分开的必要性）；
`.zstyle` 加 `Control.button.focused` 焦点环（`@focus_ring` + 2px）。

**关键决策**：

1. **`ACTIVATABLE` 与 `FOCUSABLE` 分开**——TextBox 聚焦但 Space 不点击。
2. **键盘激活复用 `pressed` 通道**——应用零改动，Space 按住有按下态，最贴 WinUI
   「两路同构」。
3. **键盘激活当帧写闩**——与 Tab / 指针同理，用户按空格期望立刻看到按下态。
4. **`focused` 内置词经 `Host::is_focused`**——`.zstyle` 只认分类名，焦点环靠它扇出。

## 顺带修的既有问题

- **`arrange_flow`（`layout.rs:1016`）8 参数触发 `too_many_arguments`** —— 在
  `4c6ea8d` 引入、baseline 声称 clippy 绿但实测红。8 个参数是布局一趟的**全部输入**，
  按本仓已有惯例（`paint.rs` / `tessellate.rs` 的 `#[allow(clippy::too_many_arguments)]`
  + WHY 注释）放过。

## 门禁

- `cargo test -p zephyra-core --lib input::activation` —— **13 条**：Space/Enter 是
  激活键 / 初始无按下 / 焦点在 ACTIVATABLE 上按下闩住 / 自动重复不重进 / 抬起清闩 /
  不可激活节点不闩 / 无焦点不闩 / 禁用节点不闩 / 失焦清闩 / `on_key` 边沿分派 /
  非激活键返回 None / Enter 同构 / 无树按下不闩。
- `cargo test -p zephyra-winit-input` —— 39 条（含新 `space_maps_to_a_semantic_key`）。
- 四条门禁 + gallery 整树门禁见提交末尾：`cargo fmt` 干净；`cargo clippy --workspace
  --all-targets -- -D warnings` 0 警告；`cargo test --workspace` 1055 passed / 0 failed
  / 52 ignored；`cargo doc --no-deps --workspace` 无新增警告；gallery gates 41 全绿。
# 2026-10-28-input-focus

> 现象：`TextField` 拿到了键盘事件（上一轮），但没有「哪个控件拿键盘」的路由——
> 焦点系统空白：`props.rs` 41 个属性键里没有 `Focused` / `TabIndex`，`focused` 只是
> 一个字符串分类名，由调用方手填（`text_input.rs` 明写「没有键盘焦点系统，宿主用点击
> 置位」）。于是 Tab 键、焦点圈闭、失焦恢复全都不存在，而 IME 的插入点坐标也只能
> 用指针位置代理。

## 排查

**焦点需要的三样东西，一样都没有**：

| 缺什么 | 现状 |
|---|---|
| `Focused` 属性 | `props.rs` 无此键；`.focused` 是样式分类名，样式层读不到「谁有焦点」 |
| Tab 顺序 | 无 `TabIndex`；无任何「从树里找出可聚焦节点」的逻辑 |
| 状态机 | 无「谁持有焦点」的跨帧事实；`host.rs` 的 `window_event` 没有 Tab 分支 |

**一个必须想清楚的概念切分**：`INTERACTIVE`（可被鼠标点到）与「可聚焦」（该进
Tab 顺序）**不是一回事**。按钮可交互但 Tab 不该停在按钮上（应该跳到下一个文本框）；
文本框可聚焦，但命中测试靠它自己的 `INTERACTIVE` 子节点。若把两者合并成一个属性，
要么按钮进了 Tab 顺序、要么文本框点不到。

## 排除的假设

- **「焦点状态塞进 InputRouter」**——`router.rs` 只管指针（hover/press），而
  Tab 是键盘事件，跟指针移动无关。塞进去会让一个「只关心指针」的模块开始维护
  另一份与指针正交的状态，revision 语义也乱了（焦点变化不该触发 hover 重绘判断）。
- **「焦点状态放 fx 层」**——fx 有 `InputState` 每帧 push，但它依赖 core，
  「哪个控件拿键盘」这种框架语义不该由效果器层决定；而且 `cpu_only::CpuHost`
  不开窗口，焦点状态机放这里会污染纯逻辑路径。
- **「Tab 顺序用显式 tabIndex 属性」**——更接近 HTML，但 `.zypui`/`zstylec` 都要
  加属性映射，且绝大多数控件用默认 0 就够（视觉顺序 = 键盘顺序）。深度优先遍历
  零新增语法，只在「少数要重排」时才需要显式值——那时候再加不迟。
- **「FOCUSED 用 `InputState.focused` 字段」**——不用扫全树 diff，但样式系统读不到：
  `.zstyle` 无法按焦点给控件换边框，必须源码判断 + 分类名。而「焦点环颜色是设计
  令牌」（`focus_ring` / `focus_inner` 都在 `Theme::role` 里），只有写成属性，
  样式表才能引用它们。
- **「FOCUSED 走 `set_styled_prop`」**——`apply` 每帧会 `retain_styled_props`
  拆掉「本轮没人声明的样式值」；焦点是状态机写的、不是样式表声明的，写进 Styled
  层会被当垃圾拆掉。所以走 **`set_prop`（Local 层）**，`apply` 不动它。
  代价：local prop 不会自动清，状态机必须显式把 `false` 写回旧节点。

## 结论

**core 新建 `input/focus.rs`，与 `InputRouter` 平级。**

| 件 | 内容 |
|---|---|
| `FocusState` | 跨帧持有 `focused: Option<ElementKey>` + `written`（上一次写过的节点）+ `revision` |
| `collect_focusable` | 深度优先遍历树，收集 `FOCUSABLE=true` 的节点 key（文档序） |
| `focus(root, target)` | 手动设焦点（鼠标点击）；目标不可聚焦/不存在 → 清空 |
| `tab(root, shift)` | Tab 移动，末端圈闭；无焦点时 Tab 聚焦第一个、Shift+Tab 最后一个 |
| `clear(root)` | 窗口失焦时清空，写回 `false` |

**props 新增两个键**：`FOCUSABLE`（进 Tab 顺序）与 `FOCUSED`（当前有焦点）。
`Node` 上（`style.rs` 的 `impl Node`）新增 `is_focusable` / `is_focused` 读取器。

**三个关键决策**：

1. **FOCUSED 走 Local 层属性**（`set_prop`），让 `.zstyle` 能按它引用
   `focus_ring` / `focus_inner` 令牌 —— 焦点环颜色进样式表、不进源码。
2. **Tab 顺序 = 深度优先文档序**，控件视觉顺序即键盘顺序，零新增语法。
3. **`write_state` 先清旧、再设新** —— local prop 不会自动清，漏清旧节点就会
   「焦点移走了还在高亮」。

## 已知限制（本轮不做，记账）

- **宿主尚未接入** `FocusState` —— 三个 `host.rs` 还没持有它，也还没在
  `window_event` 里消费 Tab / 点击。控件层也还没消费 `FOCUSED`。本轮只做
  状态机 + 属性 + 测试。
- `FOCUSABLE` 的 `.zstyle` / `zstylec` 属性映射**还没加**（props_map 无门禁，
  见 AGENTS 第 3 条）。控件层声明 FOCUSABLE 时走 core 的 `props::FOCUSABLE`，
  样式表路径是下一步。
- 折叠节点不参与焦点序（与 `hit_test` 同一判据）。

## 门禁

- `cargo test -p zephyra-core --lib input::focus` —— **20 条**：深度优先收集顺序 /
  不可聚焦排除 / 初始 Tab 聚焦首尾 / Tab 前进·循环·反向·绕回 / 移动后旧节点被清 /
  手动聚焦到可聚焦·不可聚焦·不存在 / 无焦点树 Tab 不做事 / 单节点树圈闭 / clear
  清空并写回 false / clear 后重新 Tab 从头 / 子节点 clear 后整树清零 / revision
  变化递增·无变化不动。
- 四条门禁 + gallery 整树门禁见提交末尾。
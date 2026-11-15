# 2026-11-14 切换按钮组：三个以上互斥状态用的一排按钮

> 现象（需求）：控件库缺一个**切换按钮** —— 一排按钮，**至多一颗选中**，
> 点一颗其余自动取消。蓝本是 RinUI 的 `ToggleButton` 文档
> （`https://ui.rinlit.cn/zh/BasicInput/ToggleButton.html`）。
>
> ⚠ **本条记的是走过的弯路**：第一版把「切换按钮」做成了**一颗独立的小开关**
> （每颗各管各的、能同时有多颗选中），被用户一句话纠正：
>
> > 这个是用作有 3 个或以上状态时使用的，这些状态应该是互斥的，
> > 不然我为什么不直接用开关
>
> 三个关键词决定了整个设计：**3 个以上**、**互斥**、**不然为什么不直接用开关**。

## 设计决策（先定，再写码）

| 问题 | 结论 |
|---|---|
| 两个状态怎么办 | 用**拨杆**（`ToggleSwitch`）。拨杆天生只有「关 / 开」两格，而且拖得动、看得见行程 —— 两个状态还去摆一排按钮，是把拨杆做得更好的事做得更差 |
| 三个以上状态怎么办 | 一排**按钮**，至多一颗选中。拨杆装不下第三格，而「三选一」又不该长成复选框（一排能同时勾上的框表达的是「多选」） |
| 「互斥」规则写在哪 | **不重写**。`RadioGroup`（`ZephyraUI/src/toggle.rs`）里已经有一份权威实现（至多一颗选中、选中项不能取消、禁用的项选不上）。切换按钮组换的是**长相**，不是**规则** |
| 要不要再写一个 `ToggleButtonGroup` 模型 | **不要**。同规则的第二份模型 = 两份真相（两个应用各写一遍、各错一遍）。组由 `RadioGroup` 驱动，模板只声明**结构** |
| 选中态长什么样 | **反色药丸**（底 `@content` + 字 `@surface`）—— 第二轮改成选项卡，见下面「第二轮」 |
| 选中态有没有动画 | **没有**。开关的动画是 iOS LiquidToggle 的 1:1 复刻（有原版参数），这里没有原版 ⇒ 加动画就是自创 |
| 放哪一族 | **开关族**（参考页第三族）。这一族是「选一个状态」的控件：拨杆 / 单选钮 / 切换按钮组。放按钮族是按**长相**分族，放开关族是按**语义**分族 —— 而用户是按语义找的（他要的是「3+ 状态怎么选」） |
| 无障碍报什么角色 | **Button 控制类型 + Toggle 图案**。UIA 没有 ToggleButton 这个 ControlType —— 与开关报 CheckBox + Toggle 图案同理 |

## 落地

| 层 | 内容 |
|---|---|
| `ZephyraUI/ui/controls.zypui` | 新组件 `ToggleButton`（单颗的结构）+ `ToggleButtonGroup`（一排，`for` 遍历 `items`） |
| `ZephyraUI/controls-button.zstyle` | `.toggle-button` 族（普通底 / 强调底 / 悬停 / 按下 / 焦点环 / 禁用）+ `box.toggle-button-group` |
| `ZephyraUI/src/i18n.rs` | `KEY_PARAMS` 加 `("ToggleButton", "label_key")`（组转发给单颗，单颗内部 `tr()`） |
| `zephyra-uia` | `toggle-button` 分类名 ⇒ **Button 角色 + Toggle 图案**（`rules.rs` 的 `is_toggle_button`） |
| `gallery` | `Gallery::toggle_group`（`RadioGroup`）+ `toggle_group_items` 绑定 + `tbtn:` 点击分派 + 开关族里的示例行 + 三语言文案 |

## 踩到的坑

### 1. ⚠ 一组按钮必须是**三颗以上**才有意义 —— 两颗就该用拨杆

第一版做的是「一颗一颗的独立小开关」，摆出来才发现：两态的东西摆在按钮族里，
用户的第一反应是「那我用拨杆不更好」。**控件的名字（切换按钮）只有在「多个
互斥状态」这个语境里才成立** —— 单颗的它没有存在理由。

⇒ 所以「切换按钮」在本仓的落点是**组**，不是单颗。单颗（`ToggleButton`）只是
组里的一个零件，**不单独出现在 gallery 里**（理由同上：两态用开关）。

### 2. ⚠ 互斥规则**不能**重写一遍

「至多一颗选中、选中项不能取消、禁用的项选不上」这套规则在 `RadioGroup` 里
已经有一份权威实现。再写一个 `ToggleButtonGroup` 模型就是**两份真相** ——
两个应用各写一遍、各错一遍，而那种不一致在画面上完全看不出来（都能点、都会变）。

⇒ 组由 `RadioGroup` 驱动：`RadioOption` 已经有 `key` / `label_key` / `disabled`，
`RadioGroup::is_selected(key)` 给出选中态 —— 模板要的四个字段一个不缺。

### 3. ⚠ `.checked`（3 个分类名）会把 `.focused`（2 个）的**焦点环吃掉**

`control.button.focused` 给的是 `border-color: @focus_ring; border-width: 2px`，
而 `control.button.toggle-button` 给的是 `border-color: @control_border` ——
后者特异性更高 ⇒ **键盘用户看不见焦点停在哪**。

修法：显式写 `control.button.toggle-button.focused`（3 个，压得住）与
`.toggle-button.checked.focused`（4 个，压得住 `.checked`）。
同类地，`.checked` 与本组的 `.hovered` / `.pressed` **同**是 3 个 ⇒ 靠文件次序
取胜太脆，显式补 `.checked.hovered` / `.checked.pressed`（各 4 个）。

### 4. ⚠ 选中型控件点「当前选中项」时状态**本来就不变** —— 门禁会假阴性

`every_interactive_key_is_wired_or_whitelisted` 的判据是「点击前后树的形状指纹
变了」。而选中型控件点当前选中项 ⇒ 不变（**正确行为**，不是空壳）。

⇒ `perturb()` 里必须把切换按钮组的选中项**挪到另一个**，否则那颗按钮在每个
扰动世界里都点不动，门禁会误报成「空壳按钮」。与 radio 同一款理由。

### 5. 顺手修掉一条**先前就在**的 clippy 红

`zephyra-core/src/text/mod.rs` 有一处重复的 `#[cfg(test)]`（上一个 sweep 提交
`91f5432` 带进来的），`cargo clippy --workspace --all-targets -- -D warnings`
直接报 `duplicated attribute`。一行删掉。

### 6. 着色器资产的 mtime 守卫误报（先前就在）

`shader_assets_are_in_sync` 是 **mtime** 守卫：`.bin` 必须比 `.hlsl` 新。上一个
sweep 提交改了 `zephyra.hlsl` ⇒ git checkout 重写了它（mtime 变新），而那两个
**内容没变**的 `.bin`（`zephyra.vs.bin` / `zephyra.PSComposite.ps.bin`）blob
没变 ⇒ git 没动它们 ⇒ mtime 留在旧值 ⇒ 守卫误报。

用 `fxc` 重新编译比对过：四个陈旧 `.bin`（两个 crate 各两个）与当前 `.hlsl`
的产物**逐字节相同** ⇒ 资产是同步的，只刷新了 mtime（内容零改动）。

## 排除的假设

- ❌ **「切换按钮 = 一颗独立的两态按钮」** —— 用户原话：「不然我为什么不直接用
  开关」。两个状态是拨杆的活。
- ❌ **「用 `CheckState` 表达状态」** —— `CheckState` 有三个值，半选对切换按钮
  没有意义（它没有「组」，「选了一部分」这个派生态不存在）⇒ 用 `bool` 让类型
  把非法状态删掉。
- ❌ **「为切换按钮组单写一个模型」** —— 规则与 `RadioGroup` 一模一样，两份实现
  = 两份真相。
- ❌ **「`checked` 用分类词（`""` / `"checked"`）而不是布尔传给模板」** —— 本仓
  两态控件（`PillButton` / `RadioButton` / `ToggleSwitch`）一律传**布尔**，
  三态的 `CheckBox` 才传分类词（布尔装不下第三个值）。跟着既有的分界走，
  不新造第三种约定。
- ❌ **「UIA 报 `Role::CheckBox`」** —— UIA 的 ControlType 里没有 ToggleButton；
  报 CheckBox 会让屏读器念「复选框」。正确形态是 **Button 控制类型 + Toggle
  图案**（与开关报 CheckBox + Toggle 图案同构）。
- ❌ **「给选中态加一段动画」** —— 开关的曲线是 iOS LiquidToggle 的**复刻**（原版
  参数逐条对齐），这里没有原版 ⇒ 加动画就是自创。

## 结论

「切换按钮」在本仓落成**一组互斥的按钮**（`ToggleButtonGroup`），与拨杆开关
（`ToggleSwitch`）分工明确：**两个状态用拨杆，三个以上用按钮组**。

它换的是**长相**不是**规则** —— 互斥规则仍由 `RadioGroup` 一处承担。

## 第二轮：长相改成开关轨道 + 滑动玻璃药丸

### 现象

第一版做成了「三颗独立发光的按钮」（各自有底、各自发光、像三个孤立的开关），
被用户一句纠正：

> 我想要的不是这样的，我希望有一个容器把三个按钮包含在里面，
> 这个容器就和开关按钮的轨道一样，点击后方块平移切换，
> 液态玻璃材质也和开关一样，你理解错了

蓝本改回开关家族的语言：**容器 `toggle-track` = 开关轨道**（胶囊底，半透明），
`toggle-block` = 一颗液态玻璃药丸在各选项槽位间平移（`block_x = 选中下标 × 槽宽`，
宿主逐帧补间，250 ms 快入，与拨杆把手同款手感）。药丸用 `material: glass`
（真实斯涅尔折射，和开关把手同一个 `Material::Glass` 词汇表），白描边
`#ffffff66` 钉轮廓。

### 排查

1. **模板结构**：`ToggleButtonGroup` 从 `for` 遍历单颗 `ToggleButton` 改成
   `toggle-track` 容器包着 `toggle-slot`（可点、`toggle-button` 分类名供 UIA）
   + 脱流的 `control.toggle-block`（`position: parent` + `translate-x`）。
2. **`position: parent` 的 `left`/`top` 是窗口绝对坐标**：`manual_rect` 里
   `x = coord − size.width × ax`，**不加**父内容区原点（`ref_origin` 只在未显式
   写 left/top 时用）。写了 `left: 0` ⇒ 药丸钉在窗口 (0,0) 看不见 ⇒
   改为**不写 left/top**，`anchor: top-left` 默认贴父内容区左上角，再由
   `translate-x` 平移到选中槽位。
3. **`text.toggle-label.checked` 匹配不到**：模板只给 `control` 加 `checked`，
   `text` 没带 `checked` 分类名 ⇒ 选中项的字回退成 `text.toggle-label` 的
   `@content_secondary`（灰），选中态「反色」失效 ⇒ 改成跟 `Segmented` 一样：
   `text` 也带 `{e.id == sel ? 'checked'}` 分类名。
4. **药块盖住文字**：`block` 是 `toggle-track` 的**最后一个子**，画在槽上面，
   把选中项的字盖住 ⇒ 调成**先画 `block`，后画 `for` 里的槽**（子树序即画序）。

### 排除的假设

- ❌ **「`left: 0`/`top: 0` 是相对父容器偏移」** → `manual_rect` 显式 left/top 走
  绝对窗口坐标（与 `position: root` 的 `Point::new(0,0)` 分支并列），`ref_origin`
  只兜底未显式写时 → 改为不写 left/top。
- ❌ **「选中态只给 `control` 加 `checked` 分类名」** → `text.toggle-label.checked`
  永远匹配不到 ⇒ 选中项字色不反色 ⇒ `text` 也要带 `checked`（与 `Segmented`
  的 `seg-label.selected` 同一手法）。
- ❌ **「`box.toggle-block` 也能当脱流子项」** → `box` 作为 manual 子项在布局
  趟里不占主轴尺寸，且画序在槽后 ⇒ 改成 `control.toggle-block` 并前置到 `for`
  之前。

### 结论

轨道 `264×32`（3 × 槽宽 `88`），高 `32`px，圆角 `16`px（胶囊）；药丸 `88×32`，
圆角 `16`px，`translate-x = block_x`（选中下标 × 88px），宿主 250 ms 快入补间；
`material: glass` 真实折射 + `#ffffff66` 白描边；选中项字 `@surface`（反色），
未选中 `@content_secondary`（次级灰）。画面上：一颗液态玻璃药丸在三个槽间
平移，选中槽的字白、其余灰 —— 与开关的「轨道 + 把手」同一套视觉词汇。

## 门禁

| 门禁 | 钉住什么 |
|---|---|
| `gallery/tests/gates.rs::the_toggle_button_is_exposed_with_the_right_role_and_state`（新增） | 每颗的 `checked` 分类名与模型一致 + **恰好一颗选中** + 点另一颗 ⇒ 原来那颗自动取消（互斥）+ 无障碍是 **Button 角色 + Toggle 状态** + 可访问名来自 `label_key` 通道 |
| `gallery/tests/gates.rs::every_interactive_key_is_wired_or_whitelisted` | 点每一颗真的翻面（不是空壳按钮）；`perturb()` 里挪选中项，避免选中型控件的假阴性 |
| `gallery/tests/gates.rs::every_class_name_the_source_emits_exists_in_the_stylesheet` | `toggle-button` / `toggle-button-group` / `checked` / **`toggle-label`** 这些分类名在样式表里都有规则（拼错是静默失效） |
| `gallery/tests/gates.rs::every_label_key_exists_in_every_language` | 三个选项的标签 key 三语言齐全 |
| `zephyra-uia` 单测 `a_toggle_button_is_a_button_with_a_toggle_pattern`（新增） | 切换按钮 = Button + Toggle 图案；**普通按钮没有** Toggle 图案 |

验收：`cargo fmt --all -- --check` 干净、`cargo clippy --workspace --all-targets
-- -D warnings` 0 警告、`cargo test --workspace` 全绿、`cargo doc --no-deps
--workspace` 无新增警告。

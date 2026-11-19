# 玻璃材质观感对齐原版 + 文本框光标溢出

> 2026-11-17 · `gallery` / `ZephyraUI`
> 排查手段：ZephyraUI MCP（`skills/zephyra-mcp`）读布局 + 真 GPU 截图；对照 `E:\AndroidLiquidGlass`

## 现象

用户反馈两处，经 MCP 实测复现：

1. **文本输入**：单行 `TextField` 的光标是一条 36px 高的白竖线，起点在文字顶端
   （y=212，含 padding），而输入框本身只有 36px 高（y=202..238）⇒ 光标一路画到
   y=247，在下方「全选 / 复制 / 粘贴」按钮上留一道白线（截图裁切区
   x=446，白像素 y=212..247）。
2. **玻璃材质**：材质页默认跑 `GlassMode::Cheap`（`Filter::PixelOffset`），不采样背景
   ⇒ 背景直直透过来，看起来像一块淡白贴片；切到 `Physical` 又是整圈彩虹色散
   （R/B 相对 G 的均匀偏移）—— 两者都与 `E:\AndroidLiquidGlass` 的原版观感差得远。

## 排查

### 光标

`<box class="cursor">` 在 `controls-text-input.zstyle` 里写的是 `height: 36px`，
而脱流锚点是**内容原点**（已含 padding）。高度必须等于**文本行高**（`field-text`
text-size 15px ⇒ 行高 18px），不能等于输入框高度。改成 18px 后光标落在 y=212..230，
完全在框内（截图复核：按钮上那道白线消失）。

### 玻璃

原版 `LiquidButton` 的配方是 `vibrancy() + blur(2dp) + lens(12dp, 24dp)`，核心是
`RoundedRectRefractionWithDispersionShaderString`：`circleMap` 位移剖面 + `gradSdRoundedRect`
法线 + **角部** 7 通道分色（强度 `(cx·cy)/(hx·hy)`，只在四角明显）。

本仓**早就把这条 shader 逐字转译**成了 `Filter::Lens` / `PSLens`（D3D11 / D3D12 HLSL
与 Vulkan WGSL 各一份），但只用在液态开关把手上，没接到玻璃材质页 —— 页上两条路分别是
`PixelOffset`（无背景折射）与 `Refraction`（斯涅尔 + 均匀色散）。

⇒ 新增 `GlassMode::Original`（用 `Filter::Lens`，两个参数 `refraction_height` / `refraction_amount`
由 Bezel / Thickness 档位驱动，按 1/2 缩放落到原版 `lens(12,24)` 量级），**设为默认**；
廉价路与斯涅尔路都完整保留，一键可切。面板在透镜 / 廉价两模式下都只画 Amount / Bezel
两个旋钮（透镜的分色是 shader 按几何写死的，没有旋钮 —— 不摆「点了不动」的摆设）。

## 排除的假设

❌「玻璃按钮要改成真折射才能像原版」—— 材质页三块玻璃改完即可复现原版观感；
通用按钮走廉价路是明确的预算取舍（见 `2026-11-svg-style-glass.md`），本轮不动。

❌「色散要更浓才像玻璃」—— 恰恰相反：原版色散只在角部明显（`(cx·cy)/(hx·hy)`），
`Physical` 那条路的整圈彩虹才是「太浓」的来源；用 `Filter::Lens` 而不是调大
`Refraction.dispersion` 才对。

## 结论

- 光标高度 = 文本行高 18px（不是控件高 36px）。
- 玻璃默认 = 原版透镜（`Filter::Lens`）；面板显示 `Amount` / `Bezel`；按钮 modes 文案
  增加「原版液态玻璃 / Liquid glass (original) / オリジナルのリキッドガラス」。
- 门禁新增 `original_glass_mode_moves_refraction_params`（默认 = Original、两参数接线），
  改 `cheap_glass_mode_moves_amount_and_bezel` 显式切到 Cheap。

## 第二轮（同日）：文本输入与像素偏移奶白边

用户在实机上又报三处，仍用 MCP 逐条复现修复：

### 1. 输入文字不显示

MCP 点击文本框 → `press_key("a")` → 布局树里 `field-text` 还是 placeholder。根因：
`InputState.focused` 是 `FocusState::is_visible()` 过滤后的值（focus-visible 语义：鼠标点击
设焦点但**不画焦点环**），而 gallery 的 `on_input` 用 `self.focused` 决定文本事件送给谁 ⇒
**鼠标点进输入框后的按键全被丢掉**。

⇒ `InputState` 新增 `active_focus`（行为焦点，不过滤可见性），三宿主从
`self.focus.focused()` 填充；gallery 改用 `state.active_focus` 分发文本事件。
MCP 实测连打 `a/b/c` → 文本依次变成 `hia` / `hiab` / `hiabc`。`focused` 仍只管画焦点环，两者不再混用。

### 2. 输入法候选窗位置不对

三宿主 `WEvent::Ime` 用 `self.pointer_pos`（最近鼠标位置）当插入点 ⇒ 候选窗跟着鼠标跑。
⇒ 新增 `ime_caret()`：从布局树上按 `focus.focused()` 找到焦点节点，再找其 `cursor`
子节点（模板里 `translate-x` 按字符下标算），取矩形下沿作为插入点；找不到才回退鼠标位置。

⚠ **光把坐标塞进 `Ime::Start{x,y}` 给应用不够** —— 真机截图显示候选窗仍在**屏幕左下角**。
winit 不知道插入点，`Window::set_ime_cursor_area(PhysicalPosition(caret), PhysicalSize(2,18))`
必须显式调用，OS 才会把候选窗贴过去。三后端在每个 IME 事件里都调一次（组字全过程跟随
光标移动）。截图复核：候选窗贴到文本框下方。

### 3. 像素偏移玻璃一圈奶白

`a_glow` 用带宽 24px / 峰值 0.42：220×120 玻璃块上是一圈 ~22px 宽、42% 白的**奶白带**，
像磨砂瓷砖；原版 `lens()` 没有这层大面积内辉光。
⇒ 带宽封顶 10px、峰值降到 0.16、峰值贴边（`gw = short_half*0.05 + amt*0.3`，
`gsigma = gw*0.4`，`gpos = gw*0.15`）。两份 `.hlsl` 同步改 + **25 个 entry 全部 fxc 重编**
（工具脚本写在临时目录，用 `10.0.26100.0/x64/fxc.exe`）。MCP 截图复核：奶白带消失，
背景清晰透出、只剩一条贴边亮线。

## 第三轮（11-18）：光标竖线不跟随文本

用户截图：输入 `b23342` 后竖线仍贴在输入框最左边。根因是 `text_field_cursor` 绑定把
**字节下标直接当像素** `translate-x`（代码注释自己承认这个 shortcut）—— 6 个字符只把
光标挪 6px，看起来没动。

⇒ `Gallery::view(ctx)` 用 `ctx.engine` 的文本引擎按**实际字形步进**测前缀宽度
（`TextEngineExt::advance_width`，style size=15 与 `.zstyle` 一致），结果存进
`Cell<f32>`，`binding()` 读像素值而非字节数；选区两端同样处理。ASCII / CJK / IME
组字自动对齐。MCP 截图复核：输入 `abc` 后竖线正好落在 `c` 后面 ✅

⇒ 同时修 `ime_caret()`：布局树的 `rect()` **不含绘制期 `translate-x`**，候选窗锚点
必须加上 `cursor.translate_x(ctx)`，否则跟随文本的是画面上的竖线、而 IME 仍钉在输入框左边。

## 第四轮（11-18）：文本编辑能力补全（对齐 Chromium）

用户截图：长文本溢出输入框；拖文字不能选、不能在文字间插入。照 Chromium
`SelectionController` / `RenderText::FindCursorPosition` / `SelectWord` 补全：

- **字符边界表**：`view(ctx)` 用 `engine.advance(ch, style)` 逐字累加，缓存
  `(字节, 累计px)` 表；`on_input` 拿它做像素→字节**反查**（字形中点之前归左边界）。
  没有引擎时回退每字节 1px。
- **单击定位**：`Down` 落在框内 → `set_cursor(byte)`（清选区）。
- **拖选**：`Down` 记锚点、`Move` → `set_selection(anchor..byte)`、`Up` 收尾。
- **双击选词 / 三击全选**：连击计数走 `dt_ms` 累加（500ms / 24px slop，同 Chromium），
  2 击按字符类（alnum/_ 成词，CJK/标点各自成词）扩到词边界，3 击全选。
- **溢出裁剪 + 滚动**：`.text-field` 加 `overflow: hidden`（core 的 `PushClip/PopClip`
  既有机制）；`TextField` 模板加 `scroll_x` 参数、text 节点 `translate-x`；
  `view()` 按光标位置算 `scroll_x` 保持光标可见，光标/选区绑定值扣掉滚动量。
- 4 条新单测在 `gallery/tests/text_editing_tests.rs`：单击定位、拖选、双击/三击、
  定位后插入。

MCP 截图复核：`abcde` 双击整词高亮；36 字符长文本裁在框内、自动滚到末尾，光标可见。

## 门禁

- MCP 读回：材质页 `lens:3` 趟（改前 `refract:0/lens:0`）；截图见背景被掰弯 + 角部淡分色；
  文本页面截图确认光标不再越界、输入文字实时显示；像素偏移页截图确认奶白带消失。
- `cargo test --workspace`：**1718 passed / 0 failed**。
- fmt 干净、clippy 0 警告、doc 退出 0（5 条 d3d12 既有链接警告，非本轮新增）。
- 两份 `.hlsl` SHA256 逐字节相同；25 个 `.ps.bin` / `.vs.bin` 全部重编并同步到两个 crate。

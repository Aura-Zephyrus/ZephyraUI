# 2026-11-03-liquid-toggle-1to1

> 开关控件（`ToggleSwitch`）样式与动画一比一复刻 iOS LiquidToggle
> （`E:\AndroidLiquidGlass\app\src\commonMain\kotlin\com\kyant\backdrop\catalog\components\LiquidToggle.kt`
> 与 `utils\DampedDragAnimation.kt`，Apache-2.0）。上一轮
> [`glass-decorations-and-scale`](2026-11-03-glass-decorations-and-scale.md) 先补齐了
> 地基（scale-x/y + fraction 弹簧），本轮把整个把手/轨道的规格钉到 1:1。

## 现象

- 参考页开关是 44×24 轨道、14px 把手、`.on` 瞬切颜色、把手无玻璃质感 —— 与
  LiquidToggle（轨道 64×28、Capsule 把手 40×24、颜色随 fraction 连续过渡、
  按压时把手欠阻尼放大 + 速度拉伸）不是一个量级。
- core 没有「颜色随动画进度连续过渡」的通道：`.zstyle` 只有静态 `.on` 分类名。

## 排查

- **弹簧逐根对照**：`DampedDragAnimation.kt` 有五根弹簧 —— value（ζ=1/√1000）、
  pressProgress（ζ=1/√1000）、scaleX（ζ=0.6/√250）、scaleY（ζ=0.7/√250）、
  velocity（ζ=0.5/√300）。前两根临界无过冲，指数包络与 spring(1f, 1000f) 的阶跃
  响应包络等价；后三根**欠阻尼**（ζ<1 有过冲），指数逼近表达不了 —— 必须真
  积分。本仓 `spring_step()`（半隐式欧拉）就是原版逐根的对应物。
- **速度拉伸**（原版 `layerBlock`）：`val velocity = it.velocity / 50f;
  scaleX /= 1 - clamp(velocity * 0.75, ±0.2); scaleY *= 1 - clamp(velocity * 0.25, ±0.2)`。
  原版 velocity 取自 VelocityTracker（带平滑），本仓取**解析导数**
  `d(fraction)/dt`（从指数逼近式直接算，零额外跟踪器），过一根 ζ=0.5/√300
  的欠阻尼弹簧 —— 同一物理量、更少平滑滞后。
- **轨道色 lerp**：`drawBehind { drawRect(lerp(trackColor, accentColor, fraction)) }` ——
  必须是一个**结构值颜色绑定**（`Val::Raw(Color)`），与 `filter` / `decoration`
  同一条动态通道。静态分类名（`.on`）表达不了连续过渡。
- **把手的玻璃主体**：原版 `blur(8dp·(1−p)) + lens(5dp·p, 10dp·p, chromaticAberration = true)`
  —— 本仓对应 `FilterChain[BackdropBlur(σ=8(1−p)), Refraction{strength 5p, bezel 10p,
  dispersion 0.2, ior 1.5}]`，经每帧 Raw filter 绑定（与玻璃块 `glass_filter_value`
  同形；`chromaticAberration = true` 本仓没有 bool 档，取 materials 页常用色散 0.2）。
  装饰件 `Highlight.Ambient(alpha=p, 宽/1.5, 模糊/1.5) / Shadow(4dp #000@0.05) 恒定 /
  InnerShadow(4dp·p, alpha=p)` —— 恰好是 `Decorations::control_with_progress()`，
  且**挂在把手上**（原版 `drawBackdrop` 挂 knob 上，不是整行）。
- **把手白罩** `White @ (1−p)` —— 每帧 Raw color 绑定到把手的 `background`。

## 排除的假设

- **「`.on` 分类名切换颜色就够」**：原版是**连续** lerp；画布上 200ms 的弹簧里
  颜色会硬切一帧。静态兜底仍保留（`.switch-track.on { background: @switch_accent }`）——
  无绑定调用方行为与原版一致；绑定调用方（gallery 双开关）总是零差异。
- **「把白罩和轨道 lerp 写成两个静态档」**：同上 —— 折中等于放弃 1:1。
- **「velocity 跟踪器必须逐帧喂 `addPosition`」**（原版 `VelocityTracker` 路线）：
  fraction 的一阶导是解析值，直接当速度弹簧目标 —— 少一次跨层状态复制，
  且避开了「tracker 里存的是 fraction 位置、计算 velocity」的单位折算坑。
- **「scaleX/scaleY 继续共用一根弹簧」**：原版 scaleX（ζ=0.6) 与 scaleY（ζ=0.7)
  过冲不成比例 —— 分开积分，X 过冲 ~9%、Y ~4%（单测钉住）。
- **「`CombinedBackdrop`（把手内采样轨道的离屏副本，缩放 2/3→0.75）也要做」**：
  做不了 —— 本仓离屏源只有节点自己，跨兄弟层采样需要第二条 slot 管线
  （见 `docs/dev-log/2026-10-liquid-glass-refraction-fixes.md`）。**记为已知缺口**，
  不在本轮。
- **「新增 `background=` 模板属性违反「外观不进 .zypui」」**：它是**受限的动态通道**
  —— 表达式语言没有颜色字面量，`background="red"` 在 instantiate 期直接
  报错（`zypui::tests::background_only_accepts_a_raw_color_binding` 钉住）；
  静态颜色仍只能住 `.zstyle`。zypuic 的「外观值混进 .zypui 必须被拦」那条
  门禁的判据随之收窄到 `corner-radius`（一个始终不在白名单里的外观键），
  与判据本身守的边界一致。
- **把手外投影如果整块进离屏 pass，会被切在边界**：`Decorations::outsets()`
  已按 `radius × 4 + offset` 外扩，与把手位置无关。

## 结论 + 门禁

**做了**

- 主题令牌：`switch_accent`（亮 #34C759 / 暗 #30D158）、`switch_track`
  （亮 #787878@20% / 暗 #787880@36%）—— `Theme` 字段 + `role()` + `SWITCH_ACCENT/
  SWITCH_TRACK` 两份名单镜像（tokens.rs / tokens_tests.rs / gallery 门禁）。
- `ToggleSwitch` 模型：`scale_x`/`scale_y`（ζ=0.6/0.7，√250，半隐式欧拉）、
  `velocity`（ζ=0.5，√300，低通解析 d(fraction)/dt）；
  `knob_scale_x()` / `knob_scale_y()`（含 ÷/±clamp 的速度拉伸）、`fraction_velocity()`；
  删 `knob_scale()`（双轴原不可能共用一个值）。`KNOB_TRAVEL` 仍为 20px
  （轨道 64 − 2×2 − 40，与 `CONTROLS-toggle` 度量对齐 —— 测试钉住）。
- 样式 1:1 改写：轨道 64×28 Capsule、内边距 2、无描边、`.on` 只切静态兜底色；
  把手 40×24、`#ffffff` 兜底（白罩由绑定驱动 alpha）；`.switch-knob.on`、
  `.switch-track.hovered` 删除（前者白帽不看状态、后者被绑定本地色覆盖，
  留着是死规则）。
- 模板：`ToggleSwitch` 参数换 `knob_scale_x/scale_y`、`track_bg`、`knob_bg`、
  `knob_filter`、`knob_decoration`（装饰挂把手，不再挂整行 Control）；`.zypui`
  新增**受限** `background` 动态属性（Raw Color 专用，字面量报错）。
- gallery：bind `switch_track_color`（lerp fraction）、`switch_knob_bg`
  （White @ (1−p)）、`switch_knob_filter`（blur+lens）、`switch_knob_decoration`
  （control_with_progress）；materials 页装饰开关同一套通道。
- 顺手：`.radio-indicator` 补居中规则（上轮已修）；外观门禁
  `the_whole_tree_writes_no_appearance_in_source` 增加仅限 `track`/`knob`
  key 上 `BACKGROUND` 的豁免（逐帧插值色，与 `filter`/`decoration` 绑定同条路）。

**门禁**

- 新增/改门禁：`knob_scale_overshoots_while_pressing`（X 峰值 > 1.52）、
  `velocity_stretch_follows_fraction_motion`（向右开 → scale_x > scale_y；静止后归零）、
  `knob_scale_follows_press_progress`（双轴分别验）、`knob_travel_matches_the_stylesheet_geometry`
  （64−4−40=20）；`the_switch_knob_travels_with_the_fraction` 扩到背景色 alpha
  （rest=1.0，按住=0.0）；`background_only_accepts_a_raw_color_binding`（zypui）、
  `refraction_filters` 只收 `glass:*` 三个面板的滤镜（排除把手的 Refraction）、
  `every_token_the_stylesheet_uses_exists_in_theme`（`@` 后跟数字会被当成令牌，
  注释里避免裸 `@`）、tokens 名单三向钉死。
- 四门禁：`fmt --check` 干净；`clippy --workspace --all-targets -- -D warnings`
  exit 0；`cargo test --workspace` 全绿；`cargo doc --no-deps --workspace`
  exit 0，新增 0 条警告（既有 43 条均不在本轮文件）。
- `--dump-cpu` 出图核对（暗/亮两主题 × 开/关）：
  灰轨白把手 = iPhone 关态、绿轨白把手移到右端 = 开态，几何 64×28/40×24 ✓。

**仍未做（写明的边界）**

- `CombinedBackdrop`：把手内采样轨道离屏副本（缩放 2/3→0.75）—— 第二条离屏
  slot 管线，独立一轮。
- velocity 取解析导数而非 VelocityTracker —— 振幅略大、响应更快，非像素级复刻。
- RTL：原版 `translationX` 按布局方向反转，本仓开关尚无 RTL 方向，未做。

---

## 追加（同日）：动画「跳」过去 —— 宿主的帧泵缺失

### 现象

1:1 的弹簧写完了，窗口里却几乎看不见曲线 —— 把手是**跳**到终态的。
`--dump-cpu`（单帧、无事件循环）出的图完全正确，所以问题不在几何/样式。

### 排查

- `View::on_input` 是唯一推进时钟的地方（`Gallery::on_input` 里
  `switch_demo.advance(dt)`、`enter_ms += dt`），而它只被 `HostWindow::sync_input`
  调用；D3D12 后端的 `sync_input` 只在 `handle_event` 末尾调用，`frame()` 自己
  **不推时钟**。
- 事件循环的静止态是 `ControlFlow::Wait`，`tick()` 只在 `dirty` 时产帧。
  所以「按一下、松手」只产生两拍 `on_input`，弹簧各推进一格后循环睡死。
- 更隐蔽的第二层：`tick.busy` 原本**不含**「动画中」，即便强行标脏，
  `about_to_wait` 也会立刻回到 `Wait`。
- **实测反证**（临时探针 `ZEPHYRA_PUMP_PROBE`，已删）：入场时钟逐帧
  `enter_ms = 32.8 → 331.3 → 333.0`，随后 `wants_frames` 精确转 false ——
  证明「标脏 + 推时钟」这一对动作真的把动画泵到了终点，且能自行熄火。
- ⚠ 只标脏、不推时钟的中间版本泵出来的是一张**冻住的旧画面**：帧在跑
  （`[trace] 第 120 帧` 有输出）但 `quads=0`，比不动更糟 —— 烧 GPU 还看不见动画。
  这是本仓「帧」与「时钟」分离的直接后果，改宿主时两件事必须成对出现。

### 排除的假设

- 「弹簧曲线写错了 / dt 不对」：dt 是实测毫秒（`last_frame_at`），单测逐帧推进的
  曲线与参考一致，`--dump-cpu` 的静态终态也正确。
- 「Present 了但没上屏」：帧确实在产（UIA 闩住时持续出帧），只是**每帧内容相同**
  —— 状态没变，不是呈现丢帧。
- 「D3D11 也要改」：D3D11 每轮 `tick` 都 `request_redraw()` → `RedrawRequested`
  立刻标脏，且它的 `frame()` 内部调 `sync_input` —— 常驻重绘自带动画时钟。
- 「用 render 统计的变化当判据」：属性 diff 归零要到**下一帧**才可见，而那时循环
  已经 `Wait`；判据必须是视图自己回答「还要不要帧」。

### 结论 + 门禁

- `zephyra_fx::View` 新增 `fn wants_frames(&self) -> bool`（默认 `false`，
  「静止零帧」的承诺不变）。
- D3D12 `tick()`：`animating = view.wants_frames()` 为真时 **`sync_input()` + 标脏**，
  推进后再问一次（收敛那一拍就交还 `Wait`，不浪费一轮 Poll）；
  `tick.busy` 用**推进后**的答案。
- Vulkan `tick()`：同一手法 —— 它的 `sync_input()` 本来就在 dirty 分支里，
  只需把 `busy` 从 `self.dirty` 换成推进后的 `wants_frames()`。
- `Gallery::wants_frames()`：`enter_ms < ENTER_MS || switch_demo.is_animating()
  || glass_decor.is_animating()`。
- `ToggleSwitch::is_animating()`：与 `advance` 的钉死规则配对的**精确**比较
  （收敛后所有量恰在端点），帧泵因此必然自行熄灭。
- 门禁：`zephyra-ui` 的 `is_animating_tracks_the_springs`（静止 false / 翻转 true /
  收敛 false / 按住 true / 松开 false）；`gallery/tests/gates.rs` 的
  `wants_frames_covers_every_animation_source`（入场读秒 true、跑满 false、
  翻开关 true、收敛 false、第二个开关同理）。

---

## 追加（2026-11-04）：长按拖拽 + mini 胶囊（CombinedBackdrop 的可见替代）

### 现象

1. 长按开关并**拖动**时把手不跟随鼠标 —— 这一轮之前从未实现过拖拽：
   on_input 只认 pressed 两拍（按下/抬起），没有处理 pointer_events。
2. 长按到 press = 1 时把手几乎没有内容（白罩 alpha = 0、blur = 0，纯色背景上
   看起来像「把手不见了」），且用户看到的把手比例不对。

### 排查

- **拖拽**：原稿 DampedDragAnimation.kt / DragGestureInspector.kt 的
  inspectDragGestures：Down 进入拖拽、Move 把位置映射为 fraction 直接设值、
  Up 后 fraction >= 0.5 翻到开、否则翻到关。原版 dragAmount 是**增量**逐事件
  累加；本仓用「从手势起点的 delta」实现，线性等价。
  按住期间弹簧不能再写 —— advance() 里的 fraction advance_toward 必须在
  dragging 时跳过，否则手停在中间会被弹簧匀速拉回端点（拖动时回弹、跟不上鼠标）。
- **mini 胶囊**：原版长按可见的核心来自 CombinedBackdrop：把手里再画一份
  缩小的轨道离屏副本（scale(lerp(2/3, 0.75, p), lerp(0, 0.75, p))）。
  本仓 Refraction 采样的是全屏副本，没有第二 slot，所以长按时把手体内透的是
  背后而不是「轨道压缩进去」—— 观感空壳。
- **把手比例**：本仓把手 40×24、fraction 行程 20px，与原版一致；
  用户报告的「比例不对」与白罩全透 + mini 缺失叠加后的空壳感相关，
  比例/形状本身 1:1（switch 欠阻尼 1.5x、速度拉伸同式）。

### 结论 + 门禁

- ToggleSwitch::process_pointer_events(events, scale)：
  Down+pressed 记录 drag_start_x / drag_start_fraction；Move ->
  fraction = clamp(start + (x_dip - start_x)/20px, 0..1)；Up -> fraction 过 0.5
  则 checked=true，否则 false。advance() 在 dragging 时跳过 fraction 弹簧。
  is_animating() 不把 dragging 中的 fraction 当「未收敛」。
- Gallery 调用顺序修正：先 set_pressed 再 process_pointer_events
  （否则首次点击的 Down 读到上一帧的 false，拖拽根本启动不了）。
- 模板新增 knob-mini 子容器（switch-knob-mini，40×24），缩放由绑定
  knob_mini_scale_x（2/3 -> 0.75）、knob_mini_scale_y（0 -> 0.75），
  颜色复用 switch_track_color（同一条 lerp）。外观门禁的豁免从
  key == knob 放宽到 key.starts_with("knob")（只限 BACKGROUND）。
- 本仓与原稿最终的视觉差异：mini 胶囊是**锐利**子节点（原稿 mini 是离屏
  副本，连同折射一起扭曲，边缘有色散）。完全等价仍需把手专用 backdrop slot
  —— 独立一轮。
- 门禁：drag_toggles_checked_via_pointer_events、
  drag_does_not_start_without_pressed；既有 30 toggle 用例 + 56 gates 全绿。

---

## 事故记录（2026-11-04）：shell.zypui 被我覆盖并从编译产物恢复

**是什么**：恢复过程中的一次脚本损坏了 `gallery/frontend/shell.zypui`
（把编码对象当成内容写回文件），该文件所有**未提交**内容（你的工作）当时不可见、
没有任何备份可用（Code/Cursor Local History 无记录，无 admin 权限，
`vssadmin` 被拒）。

**恢复方式**：`gallery/build.rs` 编译二进制产物
`target/debug/build/zephyra-gallery-*/out/shell.zypui.bin`（今天 09:01 之前成功编译）
包含 `controls.zypui + shell.zypui` 的**完整 IR**。我写了临时解码打印工具
（`zypuic/examples/decode_dump.rs`，用完已删）把 IR 重新打成 `.zypui` 源：
components 55..73 属于 shell，前 54 个组件与本仓 controls.zypui 一致（已对照），
page `Window` 也完整。删除了 mini 胶囊的残留参数并跑 `zypuic fmt` 归一化。

**恢复出的等价性证据**：
- `cargo check -p zephyra-gallery`、`cargo test -p zephyra-gallery --test gates` 56/56、
  `cargo test --workspace` 全绿、`clippy --workspace --all-targets -- -D warnings` exit 0、
  `cargo doc` 0 新增警告（42 条，低于 43 基线）。
- 内容语义与原文件一致（IR 逐字段还原：标签/属性/绑定/class 词/children、
  组件参数、页结构）。

**丢失的**：`shell.zypui` 里的全部 XML 注释（手写的说明文字，约占文件 1/4），
以及与原始缩进/换行不一致（由 `zypuic fmt` 归一化）。57 条
`repo_files_survive_a_mangle_and_reformat_roundtrip` 等格式门禁全绿。

**教训**：PowerShell 写文件必须 `[System.IO.File]::WriteAllText(path, TEXT, encoding)`
并校验"参数是字符串、不是 $encoding 对象"；对用户可见的文本文件先
`Copy-Item` 备份，再覆盖。

---

## 追加（2026-11-04）：按住时「没有玻璃」的根因 —— 透明 fill 吞掉了整条命令

### 现象

真机 D3D12 上按住开关，把手**不是玻璃**，而是越按越空：`press` 从 0 涨到 1 时，
白罩 alpha 从 1 掉到 0，屏幕上那块区域几乎什么都不剩。`--dump-cpu` 里
「带非常身滤镜的命令共 0 条」把问题钉在了 `DisplayList` 层面（不是 GPU）。

### 排查

- 原版把玻璃效果挂在**自己的图层**上（`drawBackdrop(...)` 的 `effects` 与
  `highlight` / `innerShadow` 都是图层属性），它不依赖「这个元素画了什么」。
- 本仓把这些效果编码在 `Command::Quad { filter, decorations }` 上，而
  `paint.rs::render_rec` 过去只在 `fill.a > 0 || has_stroke` 时才 push 这条
  命令。按住末态 `fill.a == 0`、又无描边 ⇒ **整条 Quad 不发出** ⇒ 滤镜 /
  高光 / 内阴影一起消失。不是「效果算错了」，是「载体不存在」。
- 判据由此确定为：**滤镜与装饰描述的是元素背后那一层，与 fill 无关**。

### 排除的假设

- 「`Filter::Lens` 的着色器写错了 / uniform 塞错位」：把 `--dump-cpu` 的
  `n` 从 0 变成 1 之后（补发命令），同一套着色器直接在 D3D12 上出玻璃，
  且 `--dump-cpu` 的顶点/命令数与 GPU 侧 `offscreen_passes` 一致。
- 「`BackdropBlur` 与 `Lens` 的顺序反了」：链序与原版一致（先 `blur` 后
  `lens`），两者的互斥行为另记（见下方缺口）。
- 「白罩应该保留一点 alpha 才看得见」：原版就是 `1 - progress`，末态白罩全透
  —— 视觉由玻璃本体承担，不该为了「看得见」改参数。

### 结论 + 门禁

- `paint.rs` 的发出条件改为
  `fill.a > 0 || has_stroke || (filter 非恒等且无自带内容) || decorations 非恒等`。
  「自带内容」= 图像 / 图标 / 文字 / 自绘源 —— 它们的命令已经带了同一个
  `filter`，再补一条空 Quad 会让同一层开两次离屏。
- 门禁：`zephyra-core/tests/paint_transform_tests.rs::transparent_fill_with_filter_still_emits_a_quad`
  （fill alpha=0 + 非恒等 filter ⇒ 命令仍在，且 fill 仍是全透）。
  反向由既有 `effect_contract::blurred_vector_icon_yields_offscreen_requirement`
  （按**数量**断言只有 1 条离屏需求）钉住，它正是第一次修改时红掉的那条。
- 真机证据：按住态截图
  `C:\Users\GBZ\AppData\Local\Temp\opencode\press-gpu2.png`（1400×800，
  裁剪 400,240,170,80）；`--switch-press --dump-cpu` 打印
  `滤镜命令 #45: [BackdropBlur{sigma: 0.0}, Lens{5.0, 10.0}]`。

## 追加（2026-11-04）：`Filter::Lens` —— 原版 `lens()` 的逐字转译

原版 `lens(refractionHeight, refractionAmount, chromaticAberration = true)` 用的
是 `RoundedRectRefractionWithDispersionShaderString`，与本仓已有的
`Filter::Refraction`（斯涅尔公式 `PSRefraction`）**不是同一个公式**。所以新增
变体而不是改旧的那个（材质页与 `glass` 材质建立在 `Refraction` 之上，有门禁）。

逐字对应关系：

| 原版 | 本仓 |
|---|---|
| `circleMap(1 - -sd / refractionHeight) * -refractionAmount` | `PSLens` / `fs_lens` 里同一式（符号落在着色器，结构体存正值） |
| `gradSdRoundedRect(coord, halfSize, min(radius*1.5, min(halfSize)))` | `grad_sd_rounded_rect` |
| 7 通道按 `dispersionIntensity = (cx·cy)/(hx·hy)` 分色 | 同 7 次采样、同 `/3.5` 与 `/7` 权重 |
| `offset = -padding` | 恒 0：本仓 `BackdropBlur` 不改 padding |

贯通点（漏一处就「没效果但也不报错」）：

- `zephyra-core/src/filter.rs`：变体 + `outsets()`（0，与 `Refraction` 同）+ `is_identity()`；
- `zephyra-fx/src/offscreen.rs`：`Lens` 结构、`OffscreenPass::lens`、
  `pass_for_chain` 分支（与 `refraction` / `backdrop_blur` 三者互斥，后写者清前者）、
  `lens_composite_quad`；
- `RenderReport::lens_passes`（正面计数：`refraction_passes` 无法区分「透镜没跑」与
  「透镜很弱」，而后者看起来像正常的玻璃）；
- 两侧 `shader.rs`：`PS_VARIANTS` 23 → 24，`PS_LENS = 23`；
- D3D12 / D3D11 `renderer.rs`：`needs_dst`、合成分派（`1=refraction, 2=lens, 3=backdrop_blur`）、
  计数；Vulkan `SegmentPlan::Offscreen::lens` + `fs_lens` 管线 + `record_frame` 4 元组；
- 着色器：`zephyra.hlsl`（D3D11 与 D3D12 两份**逐字节相同**）+ `zephyra.wgsl` 手抄对应。

⚠ 改 HLSL 必须重编**全部 25 个入口**（`vs_5_0` + 24 个 `ps_5_0`）并复制到
`zephyra-winit/shaders/`：`d3d11_draw.rs::shader_assets_are_in_sync` 按 mtime
比对 `.bin` 与 `.hlsl`，只编新入口会把旧的 `.bin` 留在旧时间戳上而红。

## 追加（2026-11-04）：两个**仍未闭合**的 1:1 缺口

这两条都是原版有、本仓没有的能力，**不是参数问题**，改数字不会让它更像。

1. **`CombinedBackdrop` 缺一层**。原版的把手背板 = 窗口背板 **叠加** 轨道自己的
   离屏副本（`rememberBackdrop(trackBackdrop) { scale(lerp(2/3,0.75,p),
   lerp(0,0.75,p)) { drawBackdrop() } }`），于是按住时透镜折射的是「缩小的
   轨道」—— 那条**胶囊边缘**正是液态感的来源。本仓 `g_dst` 只有一份全屏副本，
   没有第二条 backdrop slot（`Command::PushClip/PopClip` 也零生产者），所以
   按住态把手内部是**平色**（轨道色）而不是缩小胶囊。修它需要「节点捕获 +
   二次采样」的图层背板机制，独立一轮。
2. **`blur` 与 `lens` 在合成期互斥**。原版两者同时生效（blur 先、lens 折射
   模糊后的结果）；本仓 `pass_for_chain` 里后写者清前者。端点与原文一致
   （`p=0` 只剩 blur、`p=1` 只剩 lens），但中间态 `p∈(0,1)` 会丢 blur。
   修它要给透镜一条「采样已模糊的背板副本」的额外 RT，三后端同改。

上一轮记的「mini 胶囊」替代方案已**撤回**（用户判为「根本不像」）：
`switch_knob_mini_*` 三个绑定、模板子容器与样式规则全部删除。锐利的假胶囊
与「离屏副本一起被折射」是两件事，做不到就如实留空，不拿近似顶上。

### 本轮门禁

- `cargo fmt --all -- --check` 干净；`cargo clippy --workspace --all-targets -- -D warnings`
  exit 0；`cargo test --workspace` 全绿（0 failed，含 `effect_contract` 与
  `paint_transform_tests` 26/26）；`cargo doc --no-deps --workspace` 41 条警告，
  低于 43 基线，无新增。

---

## 追加（2026-11-04 晚）：三个叠加在一起的 bug —— 「还是没玻璃」

用户第二次反馈「还是有问题，感觉是渲染」。逐层拆开是**三件独立的事**，
每一件的症状都被下一件盖住，所以单看画面无法区分。

### 1. 点开关**根本不会翻转**（core 的 pointer 语义）

`ToggleSwitch::process_pointer_events` 在 `Up` 里无条件写
`checked = fraction > 0.5`。宿主 `Gallery::on_input` 的顺序是

```text
click(last_pressed)            // 激活漏斗：鼠标 / 键盘 / UIA 都走这里，翻 checked
set_pressed(false)
process_pointer_events(Up)     // ← 上一行刚翻成 true，这里按 fraction(还≈0) 写回 false
```

于是「按下去有玻璃、松手什么都没变」。模型单测与 `--dump-cpu` 各自只看一半，
所以都是绿的。真机证据：`bridge tree` 里 `Control switch:demo` 点完仍是
`switch hovered focused`（没有 `on`）。

**修法**：加 `did_drag`（原版同名），判据是**逐事件**的 x 位移
（拖出去再拖回原点仍算拖过，累计位移会判成 0）。`Up` 只在 `did_drag` 时按
`fraction >= 0.5` 提交；没拖就交给宿主的激活漏斗 —— 两边都翻才是双翻转。
门禁：`a_tap_does_not_undo_the_host_toggle`、`a_round_trip_drag_still_commits_by_threshold`。

### 2. **装饰的外投影把滤镜整趟挤掉了**（base 的合成趟）

D3D12 / D3D11 的合成趟曾是二选一：

```rust
if pass.hollow_shadow { PSHollowShadow } else { 折射 / 透镜 / 磨砂 / 普通合成 }
```

而 `hollow_shadow` 来自**装饰**（`Decorations.shadow`），开关把手每帧都带
`control_with_progress()`（含 `Shadow(4dp)`），材质页玻璃卡片也带
`Decorations::glass()`。于是**所有带装饰的玻璃**都把 `PSLens` / `PSRefraction` /
`PSBackdropBlur` 丢掉，只剩高光与内阴影。

为什么之前看不出来：外投影那一趟用 `PSHollowShadow` 的 `t * keep` 把 RT 内容
原样贴回来，画面看起来"还是玻璃" —— 只是**一点弯曲都没有**。而"没有折射的
玻璃"比"折射没接上"更像正常效果，只能靠对照才现形：

| 材质页玻璃卡片 | 关装饰 | 开装饰（修前） | 开装饰（修后） |
|---|---|---|---|
| 画面 | 强折射 + 彩虹分色 | **干净照片**（折射整趟丢失） | 折射 + 高光同时可见 |

**修法**：外投影先画（它本来就在玻璃**背后**，`PSHollowShadow` 挖掉内侧），
再按 `dst_effect` 画玻璃本体；只有"没有滤镜"时才跳过第二次普通合成，避免
半透明元素被画两遍。Vulkan 一直是两层（`hollow_composite` 与 `dst_effect`
分开录），这次把 D3D12 / D3D11 对齐到同一语义。
门禁：`a_decorated_glass_plans_both_the_outer_shadow_and_the_filter`（计划层，
fx 的纯逻辑用例）+ `a_decorated_glass_still_runs_the_refraction_pass`
（D3D12 真 GPU，正面计数 `refraction_passes >= 1`）。

### 3. Ambient 高光的公式抄错了量（base 的着色器）

原版 `AmbientHighlightShaderString`：

```text
d = dot(gradSdRoundedRect(centeredCoord, halfSize, gradRadius), normal)
t = step(0, d);  intensity = pow(abs(d), falloff)
return (t,t,t,1) * intensity
```

即 **受光那半圈白、背光那半圈黑**（判据是"法线朝哪"）。本仓写的是
`t = step(0, sd)`（判据是"形状内还是外"），并且允许 `d <= 0.75` —— 于是白色的
那半圈被画到了**形状外面**，在深色背景上是一条锯齿亮线，内侧则是一条均匀黑边。
开关按住时看到的就是"一圈白边 + 一圈黑边"，而不是环境光。

**修法**：逐字照抄原版公式（复用 `grad_sd_rounded_rect`，与 `PSLens` 同一份），
并把高光**裁到轮廓内**（原版 `HighlightNode` 用 `canvas.clipOutline`），覆盖带
改成"居中描边裁掉外侧半条后的内带 + 高斯软化"。HLSL 加原型声明（定义在透镜
那一节），WGSL 侧因为不允许前向声明，定义本来就在 `fs_highlight` 之前。

真机证据（D3D12，1400×800，裁剪 410,250,120,60）：
`press-fixed.png`（关态按住：透镜折射出轨道右端的胶囊弧 + 边缘分色）、
`onpress-final.png`（开态按住：绿色玻璃体 + 分色边 + 柔和环境光）、
`cards-fixed.png`（材质页：装饰与折射同时生效）。

### 复跑门禁

```text
cargo fmt --all -- --check                              exit 0
cargo clippy --workspace --all-targets -- -D warnings   exit 0
cargo test --workspace                                  全绿（0 failed）
cargo doc --no-deps --workspace                         41 条警告（基线 43）
```

⚠ 改 HLSL 后**必须重编全部 25 个入口**并复制到 `zephyra-winit/shaders/`：
`shader_assets_are_in_sync` 按 mtime 比对，只编新入口会把旧的 `.bin` 留在
旧时间戳上而红。

---

## 追加（2026-11-04 晚）：两个残留体验 bug 的根因与修复

### 1. 「点击时出现莫名阴影」—— `OuterShadow::control()` 偏移量写错

原版 `Shadow` 默认 offset = `(0, radius)` = `(0, 4dp)`，代码里写成了 `(0, radius/6)` = `(0, 0.67dp)`。注释里甚至误写了"偏移默认 `(0, radius/6)`"。导致阴影紧贴把手边缘，按压放大时阴影也跟着放大，看起来像"莫名其妙的阴影"。

**修法**：`zephyra-core/src/decoration.rs` 把 `OuterShadow::control()` 的 offset 改为 `(0, 4.0 * DP)`，同步修正注释。

### 2. 「收手后有额外动画」—— 普通点击也触发了按压缩放动画

原版 `LiquidToggle` 只有**长按/拖拽**（>120ms）才显示按压缩放、高光、透镜等效果。普通点击（<120ms）是瞬间翻转，没有任何按压视觉反馈。我们的实现把点击也当成了 `pressed=true`，导致 scale 动画（1.5→1.0 欠阻尼 overshoot）在点击时也触发，松手后把手会"弹"一下才稳。

**修法**：在 `ToggleSwitch` 增加 `press_elapsed_ms` 与常量 `PRESS_ANIMATION_DELAY_MS = 120.0`：
- `set_pressed(true)` 时从 0 开始累加
- 只有超过 120ms 才让 `press` 弹簧推进（进而驱动 highlight/innerShadow/lens/blur）
- scale_x/scale_y 的 target 也在延迟后才变为 `KNOB_PRESSED_SCALE`（1.5），否则直接回 1.0
- `is_animating()` 同步使用延迟判断

门禁：新增/修正测试 `press_progress_is_animated_not_instant`、`is_animating_tracks_the_springs` 钉住新行为。

### 真机对比（D3D12，裁剪 410,250,120,60）

| 状态 | 修复前 | 修复后 |
|---|---|---|
| 静止 | 灰轨 + 白把手 + 紧贴阴影 | 灰轨 + 白把手 + 正常距离阴影 |
| 快速点击（<120ms） | 有 scale overshoot「弹一下」 | **瞬间翻转，无任何按压动画** |
| 长按（>120ms） | 有 scale 动画、玻璃效果 | 有 scale 动画、玻璃效果（行为不变） |
| 松手（长按后） | scale 从 1.5 overshoot 回 1.0 | 同上（原版行为） |

### 最终门禁

```text
cargo fmt --all -- --check                              exit 0
cargo clippy --workspace --all-targets -- -D warnings   exit 0
cargo test --workspace                                  全绿（0 failed）
cargo doc --no-deps --workspace                         41 条警告（基线 43）
```

---

## 追加（2026-11-05）：点击不切换 / 拖动被拉回

用户第三次反馈：「点击切换的时候存在问题」「滑动时可能滑动到另外一边又被拉回了」。
两个症状**根因不同**，都落在 `ZephyraUI/src/toggle.rs`，`core` 与 `base` 均无涉
（宿主顺序见上一节，已经是正确的那一版）。

### 现象

| 操作 | 修前 | 修后 |
|---|---|---|
| 纯点击（无位移） | 正确翻转 | 正确翻转 |
| 点击 + 1–3 物理像素抖动 | **不翻转**（把手抖一下弹回原位） | 正确翻转 |
| 按住停 >0.5 s 再拖 | 把手**瞬移回起点**，后续增量从起点重算 | 跟随指针，松手按阈值提交 |

实测环境：gallery `--route reference --family 2`，DPI scale = 1.25，
`switch:demo` 436,272 134×28，轨道 64×28，把手 40×24，行程 20 DIP。

### 排查

**1）点击抖动被当成拖拽。** 原版判据是
`DragGestureInspector.kt`: `didDrag = dragAmount.x != 0f`。它在触屏上等价于
「这是一次拖动」，因为点按不产生任何 `Move`。桌面鼠标 / 触摸板每次点按都带
1–3 物理像素抖动（本机 scale 1.25 ⇒ 3 px = 2.4 DIP），于是点按一律走**拖拽提交**；
而拖拽提交只在 `fraction >= 0.5` 时翻转，抖动不过半 ⇒ 什么都不发生。

**2）拖拽中途停手后的大步长保护。** 宿主只在 `view.wants_frames()` 为真时泵帧
（见本文件 2026-11-03 的「帧泵」一节），而 `ToggleSwitch::is_animating()` 有意
**不**把 `dragging` 中的 `fraction` 算作未收敛。按住不动 1–2 秒后按压 / 缩放弹簧
先收敛，宿主睡死；下一次 `Move` 那一帧的 `dt_ms` 是真实的一两秒，而 `advance()`
里 `dt > 0.5` 的「一步到位」分支无条件写
`fraction = if checked { 1.0 } else { 0.0 }` —— 用户的手被弹簧覆盖，后续增量从
被改过的值重新累加。症状正是「拖到另一半、停一下、再拖，被拉回原侧，松手也不翻转」。

### 排除的假设

- **「`core` 的命中测试 / 手势识别器把 Move 吃掉了」**：`cargo test -p zephyra-core`
  里 `input_tests` 的 Move 路由全部通过；bridge `props` 在按下态能读到
  `TranslateX` 随 `hover` 变化（20 → 10.4），说明 Move 一路到了模型。
- **「宿主没把 `pointer_events` 传进来 / 顺序又反了」**：读 `Gallery::on_input`
  的顺序是 `set_pressed` → `process_pointer_events` → `click` → `advance`，
  与上一节修完的顺序一致；纯点击能被 `click` 漏斗翻转（修前亦如此）也证明这条链通。
- **「`Up` 里又发生了双翻转」**：`did_drag` 为假时模型完全不碰 `checked`；
  修前抖动点击的场景下 `did_drag` 被误置真，才是「点了没反应」—— 双翻转会表现为
  「翻两次回到原值」，与实测的「一次都没翻」不符。
- **「需要给 `ToggleSwitch` 引入 touch-slop 的完整 Compose 手势状态机」**：
  不需要。这一层只需要两条规则（阈值内吞掉位移、越过后只补超出部分），
  没有 `awaitTouchSlopOrCancellation` 的竞态问题 —— 拖拽的起点已经在 `Down` 定死。
- **「按压态高光 / 内阴影没画出来」**（上一轮留下的嫌疑）：真机重测后**不成立**，
  见下节。

### 结论 + 门禁

- 新增 `KNOB_DRAG_SLOP = 4.0` DIP（= 行程的 20%）。`Move` 在 `did_drag` 为假时
  比较**净位移** `x - down_x`：越过阈值那一步只应用 `over = travel - slop * sign`
  （Compose `overSlop` 语义），阈值内把手一动不动；越过之后回到原版的增量累加。
  因此拖满整个行程，指针要走 `KNOB_TRAVEL + KNOB_DRAG_SLOP = 24 DIP`。
- 新增 `down_x` 字段；`Down` 时一并清掉 `drag_completed_target_checked`
  （上一段手势没被宿主取走的提交目标不得被下一次点击当成结论）。
- `advance()` 的 `dt > 0.5` 分支按 `dragging` 分流：弹簧驱动的量仍一步到位，
  拖拽中的 `fraction` 是**用户的手**，不许覆盖。
- 既有 `drag_toggles_checked_via_pointer_events` 的坐标随之改成
  `100 + KNOB_TRAVEL + KNOB_DRAG_SLOP`：拖满行程现在需要多走那段阈值。
- 新增 4 条门禁（`ZephyraUI/src/tests/toggle_tests.rs`）：
  `a_jittery_click_still_toggles`（1 / 2 / `SLOP-0.5` 三种抖动 ⇒ 把手不挪、
  不产生提交目标、宿主漏斗翻转成功）、
  `a_drag_past_the_slop_follows_the_pointer_one_to_one`（越过那一步只补超出部分，
  之后 1:1）、
  `a_large_dt_does_not_snap_a_live_drag_back_to_the_origin`（`advance(3000ms)`
  不动 `fraction`，抬手仍按阈值提交）、
  `a_stale_drag_target_is_dropped_when_the_next_press_starts`。
- `cargo test -p zephyra-ui --lib` = **61 passed / 0 failed**。

真机矩阵（bridge，同一进程、连续手势）：

| 场景 | 结果 |
|---|---|
| 纯点击 | on ↔ off 正确翻转 |
| 点击 + 2 px 抖动 | 正确翻转 |
| 点击 + 12 px 左抖（9.6 DIP，> slop） | 按拖拽语义处理，不过半 ⇒ 不变 |
| ON → OFF 拖 30 px | off |
| OFF → ON 拖 30 px | on |
| +8 DIP 部分拖动（不过半） | 回弹保持原态 |
| 过冲 100 px 后松手 | 按 `fraction >= 0.5` 正确提交 |
| 按住 1.2 s（宿主睡死）再拖两段 | 20 → 12 → 4 → 松手 0，**未被拉回** |

⚠ 调试桥每个命令都会调一次 `sync_input()`，`dt` 恒小，**掩盖**了 `dt > 0.5`
那条分支；上表最后一行是靠「按住 1.2 s 期间宿主真的不泵帧」+ `props` 读数
复现的，纯桥驱动复现不出来。

---

### 本轮门禁（2026-11-05）

```text
cargo fmt --all -- --check                                     干净
cargo clippy -p zephyra-ui --all-targets -- -D warnings        exit 0
cargo test -p zephyra-ui --lib                                 61 passed / 0 failed
cargo test -p zephyra-gallery                                  含 56 条 gates，全绿
cargo test --workspace --exclude zephyra-render-ext            全绿
cargo clippy --workspace --exclude zephyra-render-ext \
    --all-targets -- -D warnings                               exit 0
```

⚠ `--exclude zephyra-render-ext` 不是本轮引入的：那个 crate 是**另一个任务的
未完成工作**（`git status` 里是 `??`，本机 2026-11-05 编不过：
`expected one of ':', '@', or '|'` 等 15 条 + `Display::fmt` 签名不符）。
它在 `members` 里，所以不带 `--exclude` 的 `--workspace` 三门禁都会红。
本轮未改动它。

---

## 追加（2026-11-05）：撤销上一轮「按压态高光/内阴影没落地」的判断

上一轮留的嫌疑是：按住态（`press = 1`）时把手内部像素 `(134,134,138)` 与轨道同色，
据此推断 `highlight` / `inner_shadow` 两趟 overlay 没画。

真机重测（gallery `reference`/family 2，scale 1.25，按住 1.0 s 后 `shot`）：

- `props` 在按住态读到 `highlight.alpha = 1.0`、`inner_shadow.alpha = 1.0,
  radius = 4.0, offset = (0,4)`、`Background = #FFFFFF00`（白罩全透）；
- 用缩放 1.5 后的元素矩形（438,274 的 40×24 绕中心放大 ⇒ 428..488 × 268..304）
  扫像素：底边 `y=303` 有一整条 `R=167` 的亮带（环境高光的内侧白半圈），
  左边界 `x=428` 是 `R=82`（外侧黑半圈），右边界 `x=486` 是 `R=140`；
  `y=304` 是 `R=106` 的模糊外沿。
- 所以**overlay 是画出来的，而且只在轮廓那一圈**。上一轮取样点 `(472..484, 286)`
  落在元素**内部**（透镜透出的背景），那里的颜色本来就该等于背后的轨道
  —— 那正是「玻璃」的定义，不是「overlay 丢了」。

结论：这不是 bug，是取样点选错了。真正的差异仍是已知缺口
**`CombinedBackdrop` 缺一层**（透镜折射的是窗口背景而不是缩小后的轨道副本），
它才会让按住态内部显得「平」；修它要第二条 backdrop slot，三后端同改，独立一轮。

---

## 追加（2026-11-05 晚）：材质页 `glass:decor` 的蓝色描边

用户截图（材质页，130×50 裁剪）里开关外圈有一圈蓝边，并提示「长按按钮时 bug
会随机呈现」「多点几次就能试出来」「为什么你用 MCP 截图看到的和我实际看到的不一样」。
这两句话指向**两个独立缺陷**，一个在焦点语义，一个在脏区计算。

### 现象

| 观察 | 含义 |
|---|---|
| 蓝边颜色 = `(59,130,246)` | 恰好等于深色主题 `palette.accent` ⇒ `@focus_ring` |
| 点击后 `get_props` 读到 `style focused` + `BorderColor = $focus_ring` + `BorderWidth = 2` | 蓝边就是 `Container.switch-track.focused` 的焦点环 |
| `shot` 里看不到、真机能看到；长按（把手续涨到 1.5）时随机出现 | `shot` 强制 `Damage::Full`；真机走增量脏区，scissor 裁掉了变换后的像素 |

### 排查

**1）蓝边不是轨道自身的描边。** `controls-toggle.zstyle` 里 `.switch-track` 没有
`border-*`，唯一给轨道加边框的是 `.focused`。鼠标点击在
`on_pointer_up` 里把焦点设到被点节点（`FocusState::focus`），于是 `.focused` 生效。
`shot` 强制整帧，因此只要焦点在，它也会画蓝边 —— 用户看到的差异不是「shot 少了蓝边」，
而是「真机在蓝边之外还有随机残影」。

**2）绘制变换没有进脏区。** `frame::scan` 给脏节点推的是 `node.rect()`（**布局盒**）。
但绘制还会叠加两样只影响画面、不影响布局的变换：

- `translate-x / translate-y`（沿祖先链累加）—— 开关把手的 20 DIP 行程；
- `scale-x / scale-y`（绕本节点中心）—— 按住时放大到 1.5。

再加上装饰（外投影 / 高光）与滤镜（模糊 / 折射）的外扩。只按布局盒设 scissor，
长按时放大到盒外、平移出盒外的那部分像素会被裁掉；旧位置又没有旧矩形快照可清。
`shot` 走 `Damage::Full`，所以**永远复现不出来** —— 这正是「截图和真机不一致」的根因。

### 排除的假设

- **「`glass:decor` 自己的样式表有蓝色 border」**：gallery 的 `shell.zstyle` /
  `pages.zstyle` 里 grep 不到 `switch-track` / `switch-knob`；且点击后读到的
  `BorderColor` 是 `$focus_ring`（令牌），不是字面色。
- **「是把手高光 / 透镜的色散把边缘染蓝」**：`Highlight` / `InnerShadow` 的颜色
  都是白色（`decoration.rs`），且 `Filter` 的色散只在透镜边缘产生彩色条纹，不是
  一整圈纯 `(59,130,246)`。
- **「主题令牌取错」**：`(59,130,246)` 与桌面主题的 `palette.accent` 逐位相同，
  且 `theme.rs` 里 `focus_ring = palette.accent`。
- **「MCP `shot` 本身有问题」**：`shot` 是**正确的**整帧渲染；差异来自它强制
  `Damage::Full`，绕过了真机的增量路径。诊断时不能只用 `shot` 判「真机有没有」。

### 结论 + 门禁

**焦点环改为 `focus-visible` 语义（core + base）：**

- `zephyra-core/src/input/focus.rs`：`FocusState` 增加 `visible`。
  `focus()`（鼠标点击）设焦点但 `visible = false`；`tab()`（键盘）设 `visible = true`；
  `clear()` 复位。新增 `is_visible()`。
- 三个宿主（`zephyra-winit` / `zephyra-winit-d3d12` / `zephyra-vulkan`）的
  `InputState.focused` 改为「只在 `is_visible()` 时报告」—— 应用据此发 `focused`
  分类名，于是鼠标点击不再画环，Tab 仍然画。激活（Space/Enter）与无障碍读的仍是
  `FocusState::focused()`，行为不变。
- 新增 5 条门禁（`core/src/input/tests/focus_tests.rs`）：
  `tab_focus_shows_the_ring`、`mouse_focus_does_not_show_the_ring`、
  `clicking_the_same_focused_node_still_clears_the_ring_and_bumps_revision`、
  `clear_also_hides_the_ring`、`no_focus_means_ring_not_visible`。

**脏区跟随绘制变换与装饰（core）：**

- `zephyra-core/src/frame.rs`：新增 `visual_bounds()`，把 `translate`（祖先累加）+
  `scale`（绕本节点中心）+ `FilterChain::bounds_for` + `Decorations::outsets` 合成
  真正的绘制矩形；`scan()` 沿祖先链累加位移。
- 脏节点带非恒等绘制变换（`translate != 0` 或 `scale != 1`）时**退让成
  `Damage::Full`**：它会「移走」，而这一层没有上一帧旧矩形快照，收窄必然留残影。
  静态装饰 / 滤镜不移动，只按 `visual_bounds` 外扩即可，仍保留收窄。
- 新增 3 条门禁（`core/src/frame_tests.rs`）：
  `damage_falls_back_to_full_when_a_dirty_node_is_scaled`、
  `damage_falls_back_to_full_when_a_dirty_node_is_translated`、
  `damage_expands_for_decorations`。

### 本轮门禁（2026-11-05 晚）

```text
cargo fmt -p zephyra-core -p zephyra-fx -p zephyra-winit-d3d12 \
    -p zephyra-vulkan -p zephyra-gallery -p zephyra-ui -- --check      exit 0
cargo clippy -p zephyra-core -p zephyra-fx -p zephyra-winit-d3d12 \
    -p zephyra-vulkan -p zephyra-gallery -p zephyra-ui \
    --all-targets -- -D warnings                                       exit 0
cargo test -p zephyra-core --lib                                       430 passed
cargo test -p zephyra-fx --lib                                         144 passed
cargo test -p zephyra-ui --lib                                         61 passed
cargo test -p zephyra-gallery                                          全绿（含 56 条 gates）
```

⚠ `zephyra-winit`（D3D11）本轮改了 `InputState.focused` 的同一处，但该 crate 依赖
**另一个任务未完成的 `zephyra-render-ext`**（`renderer.rs` 里 `BackendCapabilities`
/ `RenderHook` 引用的签名还没对齐），因此无法单独编译验证；改动与另两个宿主逐行同构。

---

## 追加（2026-11-06）：长按左右滑动后动画卡住 —— 弹簧积分越过稳定域

### 现象

材质页 `glass:decor` 长按后**在轨道中间**来回滑动，松手（或滑动过程中）动画会
「卡住」：把手的缩放不再回到 1，肉眼看到一块被翻转放大的白色/绿色块，甚至
看不见把手。用户描述「滑动幅度不要太大，就在中间滑动，他会不动」，且
「基本稳定复现」。

### 排查

临时在 `ToggleSwitch::advance` / `process_pointer_events` 里加 `ZEPHYRA_TOGGLE_TRACE`
日志（验证后已删），用调试桥驱动「按下 → 停 0.35 s → 小幅左右滑 → 松手」，
抓到关键一帧：

```text
[ptr:glass:decor] phase=Up x=318.00 ... drag=true pressed=false f_before=1.0000
[toggle:glass:decor] dt=381.6 drag=false pressed=false ... sx=1.5000 sy=1.5000
[toggle:glass:decor] dt=28.7  drag=false pressed=false ... sx=-16.7062 sy=-16.7062
```

`dt = 381.6 ms` 落在 `advance` 的「一步到位」阈值（`dt > 500 ms`）**之下**，于是
进了常规的半隐式欧拉积分；而最快的一根弹簧（速度跟踪 √300）的稳定上限是
`2/ω ≈ 116 ms`。一步 381.6 ms 跨过去，弹簧不是收敛而是**发散**：`scale_x`
从 1.5 跳到 −16.7，再过几帧在 −16.7 → −13.7 → −11.4 → … 来回振荡（`is_animating`
始终为真，宿主一直在泵帧却越画越离谱）。负缩放会把把手镜像并放大 16 倍，
看起来就是「卡住/消失」。

这个长 dt 的来源不是掉帧，而是**保留式宿主的正常节奏**：拖拽期间
`is_animating()` 故意不把 `fraction` 算作未收敛（否则按住不动 = 每秒 60 帧空转），
按压/缩放弹簧收敛后宿主就 `Wait` 睡死；下一次鼠标事件到来时 `sync_input` 的
`dt` 是真实的一两百毫秒。因此「按住 + 间隔地滑」几乎必然踩进
`(116 ms, 500 ms]` 这个窗口 —— 与「随机/多点几次」的观感一致。

### 排除的假设

- **「`fraction` 被大步长保护瞬移回端点」**：已有门禁
  `a_large_dt_does_not_snap_a_live_drag_back_to_the_origin` 钉住拖拽分支不碰
  `fraction`；trace 里拖拽中 `f` 每一步都跟着手变（12 → 16 → 12）。
- **「宿主的脏区/scissor 又算小了」**：本轮之前刚修过 `Damage::Full` 退让；
  且 trace 直接读到的是模型里的 `sx/sy`（不是像素），说明坏的是状态本身。
- **「`dt > 0.5` 的分支没起作用」**：它只覆盖 > 500 ms；本次是 381.6 ms，
  在窗口内，所以必须靠积分本身稳定，而不是靠更大的阈值去赌。
- **「把手的 `translate-x` 停了」**：没有。停止的是包络（缩放）；`fraction`
  在 trace 里持续响应 Move，只是被巨大的负缩放遮住。

### 结论 + 门禁

`zephyra-ui/src/toggle.rs`：

- `spring_step` 改为**固定子步积分**：`MAX_SPRING_STEP = 4 ms`（比最紧的稳定
  上限 116 ms 小一个数量级），`dt` 按子步切分，任意 `dt` 都不会越出稳定域；
  物理曲线不变（同一微分方程，更小步长）。
- 加了非有限值自愈：`x`/`v` 若已是 NaN/inf，直接钉回目标并清零速度 ——
  否则坏状态会永久传染，`is_animating` 永远为真、宿主永远泵帧。
- 单测（`ZephyraUI/src/tests/toggle_tests.rs`）：
  `a_long_frame_gap_never_makes_the_scale_spring_explode`（dt = 130/200/300/
  381.6/480 ms 下缩放必须有限且留在 `[0.5, 1.6]`，之后能收敛）、
  `sliding_back_and_forth_in_the_middle_keeps_the_knob_alive`（按住 → 中间
  小幅左右滑 → 松手，逐步断言 `fraction` 跟随、缩放有限、最终归 1）。

实测对照（同一脚本，gallery `--route materials --theme dark`，DPI scale 1.25）：

| | 修复前 | 修复后 |
|---|---|---|
| 松手帧 `scale_x` | 1.5 → **−16.7062** | 全日志 min 0.9557 / max 1.5442 |
| 中间滑动 `TranslateX` | 0 ↔ 20 跳变 | 12 ↔ 16 稳定跟随（每一步 ±2） |

### 本轮门禁（2026-11-06）

```text
cargo fmt -p zephyra-ui -p zephyra-gallery -p zephyra-core -- --check   exit 0
cargo clippy -p zephyra-ui -p zephyra-gallery -p zephyra-core \
    --all-targets -- -D warnings                                        exit 0
cargo test -p zephyra-ui --lib                                          61 passed
cargo test -p zephyra-gallery                                           全绿（含 56 条 gates）
cargo doc --no-deps -p zephyra-ui                                       exit 0
```

---

## 追加（2026-11-06 续）：缓慢移动依旧卡住 —— 鼠标事件不唤醒宿主

### 现象

上面的弹簧修好之后，**缓慢移动 / 中点来回拖**在真实鼠标下依然看起来不动。
`hover` 注入（调试桥）下一切正常，只有真实 Win32 鼠标卡住——这条差异就是线索。

### 排查

宿主（D3D12）事件链：`WEvent::CursorMoved` → `on_cursor_moved` —— 它把
`PointerInput{Phase::Move}` 压进 `pending_pointer_events`，并调用 `router.on_pointer`。
置脏只有一处：`route()` 里比较 `router.revision()`，**仅当悬停/按下状态切换**时
递增。拖拽中 hover 不变 → `revision()` 不变 → 不置脏 → 宿主 `Wait` 睡死。

`sync_input`（把事件推给视图）只在 `tick()` 里跑，而 `tick()` 只在 `dirty` 时
或者 `is_animating()` 为真时被唤醒；拖拽中 `fraction` 被有意排除在 `is_animating()`
外，所以按压/缩放弹簧收敛后宿主彻底睡死——Move 堆在队列里永远没人消费，
knob 的 `TranslateX` 停在首按那一帧的样子。

调试桥（`inject_hover` / `inject_pointer_down`）自己带 `dirty = true` + `sync_input()`，
所以**桥能拖动没问题，真实鼠标就不动**——这正是之前门桥验证通过却用户仍复现的原因。
Vulkan 宿主的 `WindowEvent::CursorMoved` 分支本来就无条件 `dirty = true`（一致），
D3D11 则与 D3D12 同病（`on_cursor_moved` 返回值控制置脏，拖拽中恒为 false）。

### 排除的假设

- **「按压后没有把手位移，只是画面层冻结」**：模型其实没收到 Move（trace 里
  `[ptr:]` 在真实鼠标下整段不出现）——冻结发生在**事件递送**，不是渲染。
- **「`wants_frames` 本该续泵」**：`is_animating()` 故意不含拖拽中的 `fraction`，
  这是正确的省电设计；问题只在于**外部输入不唤醒宿主**。
- **「整个动画系统没在跑，只需刷新焦点环」**：松手那帧有指数收敛（`press`/`scale`），
  说明本机泵帧路径是通的——前提是 `dirty` 被置上了。

### 结论 + 门禁

- D3D12（`zephyra-winit-d3d12/src/host.rs`）：`CursorMoved` 与 `MouseInput`
  处理器末尾无条件 `self.dirty = true`（注释写明 revision 的边界），不再只依赖
  `router.revision()`。Vulkan 宿主行为不变（已一致）。
- D3D11（`zephyra-winit/src/lib.rs`）：同一个缺口一并补上（无条件置脏），**但该
  crate 依赖另一个任务的 `zephyra-render-ext`，无法单独编译验证**（与 focus-visible
  的改动同一限制）；改动与 D3D12 逐行同构。
- 实测（真实 `SendInput` + `PrintWindow` 截图，不开调试桥）：按下把手后来回慢拖，
  截图里的把手重叠蒙版在各帧持续左移/右移（`overhang x` 308–350 → 300–342 → 298–338
  → 303–345 …），松手后回到终点 `white bbox x 310–349`，控制侧 `style on`。
  修复前同一脚本所有 `Move` 截图与按下帧一致（画面冻结）。
- 门桥验证：`hover` 注入本就不受此影响；所以实测必须走真实鼠标（见脚本
  `realmouse.py`）。
- 本轮门禁：`fmt --check`（三个包干净）/ `clippy -p zephyra-winit-d3d12
  --all-targets -D warnings`（0 警告，依赖 cache）/ `cargo doc --no-deps
  -p zephyra-winit-d3d12 -p zephyra-ui`（退出 0）/ `cargo test -p zephyra-ui --lib`（61）。
  D3D11 未能编译（见上）。

---

## 追加（2026-11-06 续二）：单击到位末端「莫名其妙变长」

### 现象

单击开关时，按钮确实会「放大 → 移动到位 → 缩小」，但**到位那一刻会水平撑宽
一下再恢复正常**——两个方向都复现，用户描述「末端会莫名其妙变长再变为正常」。

### 排查

控制变量复刻原版 `DampedDragAnimation.kt`：`release()` 会先等
`|fraction − target| ≤ 2.5%`，之后 `pressProgress` / `scaleX` / `scaleY`
才从 1.5 回落。阻尼在本仓原先是半隐式欧拉积分下的欠阻尼：

- `scale_x`：原版 ζ=0.6，`scale_y`：ζ=0.7 —— 欠阻尼二阶系统的过冲
  `exp(−ζπ/√(1−ζ²))` 约 **9% / 4%**，**先跌破 1 再越过 1**；
- `velocity`：ζ=0.5，反向过冲后被乘进 `scaleX /= 1 − clamp(v·0.3, ±0.2)`，
  放大了水平撑宽；截帧里白块 33–50 ms 约 **48–49 px**（正常 40 px）。

仿真（与 Rust `spring_step` 同一微分方程）：`scale_x` 触「谷底」**0.954**，
越过 ~1.02 的尾部账；与截帧（压扁 → 撑宽 → 正常）一致。

### 排除的假设

- **「按住/移动期间的速度拉伸在作祟」**：速度拉伸在**移动中**是设计内效果
  （液态玻璃），`scaleX /= 1 − v·0.3` 的正负号跟着方向走，而且**收敛即归零**
  （v→0 拉伸消失）。不可能在两个方向都在末端产生水平拉伸；欠阻尼过冲总是在
  回落段绕过 1 的两侧摆，才是两个方向都水平撑宽的原因。
- **「白罩 alpha 造成的视觉假象」**：量的是纯白区域的 bbox，不依赖透明度；
  「先压扁后撑宽」在数值（38 → 49 → 40 px）上逐帧印证。
- **「指数包络不会过冲」**：`press` 确实单调；**spring_step 的积分**才会
  越过目标——与 2026-11-06 修发散是同一条代码路径，方向反了。

### 结论 + 门禁

用户明确裁决「如果是原版也这样，按照我的意见改」（不喜欢末端撑宽）。

- 刚度不变，ζ 全部改为 **1.0（临界）**：`scale_x` ω=√250、`scale_y` ω=√250、
  `velocity` ω=√300。到位末端不再先压扁后撑宽，整条线条变成「放大 → 移动
  → 缩小」。
- 原则：**不改几何 / 时长 / 面板装饰，只改到位末端的阻尼比**；
  弹簧仍是半隐式欧拉 + 子步（发散修复保留）。
- 文档与 `.zypui` 注释、`gates.rs` 注释、`docs/dev-log/README.md` 索引同步。
- 单测：`knob_scale_never_overshoots_while_pressing`（按压全程不得超过
  `KNOB_PRESSED_SCALE + ε`），原 `knob_scale_overshoots_while_pressing`
  （欠阻尼峰值应 ≥ +0.02）已废除并改名。
- 验证（CLI 内置 MCP，`zephyra mcp --port 7456`；不用真实鼠标）：
  `click` 后两个方向均翻转（`style on` / 无），`white bbox`：
  - OFF→ON：16 ms 43×24 → 33 ms 48×20（动态拉伸，设计内）→ **50 ms 起恒 40×24**；
  - ON→OFF：终态同 40×24，两个方向均不再到位撑宽。
- 门禁：`fmt --check` ✓；`clippy -p zephyra-ui -p zephyra-gallery --all-targets
  -D warnings`（0 警告）✓；`cargo test -p zephyra-ui --lib`（63 passed）✓；
  `cargo test -p zephyra-gallery`（含 56 条 gates，全绿）✓；
  `cargo doc --no-deps -p zephyra-ui` ✓（exit 0）。
- 有意偏离：原版沙盒 ζ=0.6/0.7/0.5 已被替换——任何为「恢复 1:1」改回欠阻尼
  的提交都应先问用户。

---

## 追加（2026-11-06 续三）：「拖动正常，单击切换还是有」——真正的来源是速度拉伸

### 现象

判定修复解除拖拽期过冲后，用户反馈：**拖动时正常，点击切换时还有**。

### 排查

本仓 `velocity` 弹簧吃的是 `(fraction − fraction_prev) / dt`——`process_pointer_events`
先写 `fraction`，再轮到 `advance`，故**拖拽期速度恒为 0**；只有松手后指数自主推进
那一路（单击 / 程序切换，`checked` 直接翻转 + 弹簧逼近）会喂出非零值。
原版 `layerBlock` 的速度拉伸公式（`scaleX /= 1 − clamp(v/50·0.75)`、
`scaleY *= 1 − clamp(v/50·0.25)`）被我们喂上了**唯一非零**的场景——也就是用户
单击时看到的「白罩莫名其妙变长再变正常」。拖拽手势因为从来吃不到这一项，
一直是「正常」的。

`scale_x`/`scale_y` 的欠阻尼过冲（续二）不是唯一来源；两条都叠额外拉伸，
`ζ` 之一清掉后，单击仍保留速度弹簧那条。

### 排除的假设

- **「速度拉伸在手指拖拽时也出现，只是不明显」**：拖拽期 `(fraction−prev)/dt ≡ 0`，
  速度弹簧喂的是零；拖拽画面里拉伸从未存在（不是「不明显」）。
- **「限制到 `dragging` 分支就够了」**：那样留一个**全程不触发**的死分支，
  并且程序切换整体无拉伸——与「单击与拖动一致」的目标恰好相反。

### 结论 + 门禁

用户明确「拖动正常，单击切换有」，按「两者一致且正常」处理：

- **整体删除速度跟踪弹簧**（`velocity` / `velocity_vel` / `OMEGA_300` /
  `fraction_velocity()`）、`layerBlock` 拉伸公式——`knob_scale_x/y` 只剩纯缩放
  弹簧；类型文档、`.zypui` 注释、`dev-log/README.md` 索引同步更新。
- 行为基准：松手后 fraction 由指数包络自己逼近目标，*没有*任何轴向拉伸。
- 单测改动：`velocity_stretch_follows_finger_velocity_not_spring` 废除
  （在本仓恒为零，钉不住任何东西）；`programmatic_checked_flip_does_not_stretch_the_knob`
  保留（或更名之前的含义，钉住两个轴恒等）。
- 验证（CLI 内置 MCP，未用真实鼠标）：单击后翻转正确，白块宽 39→40 →
  40 px 恒定；`PrintWindow` 全程截图（press 放大 → 到位成品位 → 正常）核对一致。
- 门禁（2026-11-06 续三）：`cargo fmt -p zephyra-ui -p zephyra-gallery --check`
  全绿；`clippy --all-targets -D warnings`（ui+gallery）0 警告；
  `cargo test -p zephyra-ui --lib` 63 passed；`cargo test -p zephyra-gallery`
  全绿（含 56 gates）；`cargo doc --no-deps -p zephyra-ui` exit 0。

---

## 追加（2026-11-06 续四）：把手位移改为官方「快入」非线性曲线

### 现象

用户要求：「现在弄非线性动画」。把手位移原先走**临界指数包络**
（`fraction ← T + (f−T)·e^(−ω dt)`，ω = √1000）——平滑但不是官方曲线，
曲线控制点与时间档都是拍出来的。

### 用户裁定（问卷结果）

- 改 **所有 ToggleSwitch** 的把手位移（不含 press / 缩放）；
- 缓动用官方 **「直接入口 / 快入」** `cubic-bezier(0,0,0,1)`（先快后慢）；
- 时长 **250 ms**（官方 167/250/333 ms 档位里的中档，位置属性）；
- 中途打断（快速来回切）：**从当前 fraction 完整重走 250 ms**（不压缩时长、不回弹）；
- 轨道色（`lerp(track→accent, fraction)`）**随同一时钟**。

### 方案

- 本仓 `zephyra-ui` **不依赖 `zephyra-style`**（按层隔离），而
  `cubic-bezier(0,0,0,1)` 退化成 x = t³、y = 3t² − 2t³，自变量以
  `t = elapsed / dur` 代 x 即可得 `progress = 3·t^(2/3) − 2·t`——端点精确、
  严格单调、先快后慢，无迭代。实现为 `direct_ease`（纯函数，可单测）。
- `advance()` 中 fraction 改由 tween 驱动：
  `u = clamp(elapsed_ms / FRACTION_TWEEN_MS)`，
  `fraction = from + (target − from) · direct_ease(u)`；
  到 u=1 时**精确钉在目标**（diff 稳定，稳态零重建不受影响）。
- 打断：`checked` 中途翻回来（tween 进行中）→ **从当前 fraction 重启**，
  重走完整的 250 ms；两帧之内把开关切回去也不会跳变（从前一帧的位移接着走）。
- 拖拽路径不动：`dragging` 期间 fraction 由 Move 直接写，tween 强制让位。
- 松手后未到端点 → 下面重新从当前位置走完整一趟（与单击同一套路径）。
- `advance_toward` 只保留给 `press`（指数包络，未动）。
- 速度弹簧 / `layerBlock` 拉伸**已删除**（续三）；本轮未恢复，也不需要恢复。

### 排除的假设

- **「指数包络够用，别再加曲线」**：用户明确要求非线性、要官方曲线；且曲线
  的"先快后慢"是位置动画的官方入口形状，与用户对开关的预期一致。
- **「在 `zephyra-ui` 里复用 `zephyra-style::Easing::Direct`」**：层隔离
  （见 2026-10-28-zephyra-ui-single-source.md）——控件库不依赖声明层。
  退化后直接公式实现，无需牛顿迭代，也不会漂移。
- **「tween 中途反转应按剩余时间走完」**：用户选了「从当前位置完整 250 ms」，
  压缩时间会产生瞬时的异常速度，不采用。

### 结论 + 门禁

- `FRACTION_TWEEN_MS = 250.0`（官方「快入」中档，位置属性；出处
  `docs/developer/animation.md` §1）。
- `direct_ease` 退化公式 + 三条新单测
  （`direct_ease_shape_is_monotonic_and_pinned_to_endpoints`、
   `fraction_tween_lands_exactly_on_the_target_and_stops`、
   `reversing_mid_tween_restarts_from_current_fraction`、
   `an_unfinished_tween_is_animating`）。
- 实测（CLI 内置 MCP，`zephyra mcp --port 7456`，未动用真实鼠标）：
  单击切换时把手位移按先快后慢推进——
  0 ms 20.0 → 16 ms 9.6 → 33 ms 6.4 → 50 ms 5.0 → 100 ms 1.6 →
  167 ms 0.2 → 205 ms **精确 0.0**（停住，无反弹、无残余漂移）。
- 门禁（续四）：`cargo fmt -p zephyra-ui -p zephyra-gallery --check` ✓；
  `clippy --all-targets -D warnings`（ui+gallery）0 警告 ✓；
  `cargo test -p zephyra-ui --lib` **67 passed** ✓；
  `cargo test -p zephyra-gallery` 全绿（含 56 gates）✓；
  `cargo doc --no-deps -p zephyra-ui` exit 0 ✓。

---

## 追加（2026-11-06 续五）：开关材质切换——`material` 属性（solid 零动画 / glass 维持现状）

### 现象

用户要求：「开关控件同样支持材质切换，和按钮一样」。按钮的材质是
`.zstyle` 一行选（`material: frosted|glass`，未声明 = 纯色）；开关的
外观全是绑定驱动，`.zstyle` 里没有对应通道。

### 用户裁定（问卷结果）

- **作用层级**：轨道 + 把手都套（不止轨道）；
- **不要半透明**：只支持「液态玻璃」与「默认」，**磨砂（frosted）弃用**
  （磨砂需要半透明填充才能糊住背后，用户不接受）；
- **液态玻璃 = 维持现状**（白罩 White@(1−press)、按压透镜 blur(8(1−p)) +
  lens(5p,10p)、装饰 control_with_progress、fraction 快入 250 ms）；
- **默认 = 零动画**：无 fraction tween、无 press 弹簧、无缩放弹簧——
  状态即画面（翻转瞬间到位）；
- **轨道颜色两变体都可改**（维持 `lerp(track→accent, fraction)` 绑定通道；
  默认变体的 fraction 跳变，无过渡）；
- **gallery 两个页都显示**：reference 页（PageSwitches 材质行）+ 材质页
  （PageMaterials 底部 DemoRow），各放 solid / glass 两个可点击开关。

### 方案

- `zephyra-ui::ToggleSwitch` 新增 `material` 字段，类型复用
  `zephyra_core::filter::Material`（`Solid`（#[default]）/ `Glass`；与按钮
  同一枚举、同一词汇表）。`.zypui` 的 ToggleSwitch 加 `material` 参数、
  落类 `.switch.solid` / `.switch.glass`（纯标记，无样式差异）。
- **默认 = Solid**：与按钮「未声明 = 纯色」一致；`ToggleSwitch::new` 的
  行为从"玻璃复刻"变为"纯色"——**既有调用方必须显式 `material="glass"`**
  （gallery 的 airplane / glass:decor 已补）。
- Solid 的零动画实现：
  `set_checked` / `set_pressed` 里直接落位（`fraction = 0|1`、
  `press = 0|1`），`advance()` 兜底同样瞬变；拖拽（手势非动画）仍由
  Move 直接写 fraction，`advance` 不回拉。`is_animating()` 恒 false →
  宿主不必泵帧。
- 外观：**不改 .zstyle 的静态把手**（`.switch-knob` 静态 `#ffffff` 就是
  solid 的外观）；solid 走"宿主不绑玻璃值"路径——白罩/透镜/装饰绑定缺
  失时 zypui 按「没设」降级（`filter`/`decoration`/`scale-*` 静默跳过）。
- `background` 例外：zypui 对它绑错类型**直接报错**（静态色属于 .zstyle），
  所以 solid 也必须绑 `knob_bg` —— 绑常量纯白（`solid_knob_bg_value()`）。
- `.switch.solid { opacity: 1 }`：让分类词进「已知类名」门禁
  （`every_class_name_the_source_emits_exists_in_the_stylesheet`）。

### 排除的假设

- **「把 material 做成纯 .zstyle 开关（像 Button）」**：开关的玻璃外观全在
  运行期绑定里（knob_x/scale/filter/decoration/background），`.zstyle` 只
  能管静态外观——轨道色 lerp、白罩透明度、装饰强度都不在它的能力范围。
  分流：`.zstyle` 只负责落分类词（供门禁/主题用），行为由 `material` 字段控。
- **「默认保持玻璃、solid 显式声明」**：按钮先例（未声明 = 纯色）被用户认可；
  且"液态玻璃 = 维持现状"意味着玻璃变体开箱即用比自建更常见，默认给玻璃
  反而让纯色用户多写一个参数——与按钮不一致。
- **「solid 也 animate 一下但用 0ms」**："零动画"是用户原话，就纯瞬变。
- **「materiale 参数沿用按钮的 frosted/glass」**：frosted 依赖半透明+背景
  模糊，用户明确不接受；且开关轨道只有 28px 高，frosted 糊背后的收益不
  成比例。

### 结论 + 门禁

- `ToggleSwitch::material()` / `with_material()`；
  `solid_toggle_snaps_instantly_and_never_animates` /
  `solid_press_progress_is_instant` /
  `solid_drag_fraction_is_not_snapped_back` 三条新单测钉住。
- gallery：reference 页「材质」行 + 材质页底部「材质」DemoRow（solid /
  glass 各一枚，`mat:solid` / `mat:glass`）；既有两个演示开关显式
  `material="glass"`。材质页 DemoRow 放在 `.section`（无定高）而不是
  732×480 的 `.material-board`（image-texture 定高板，塞行会撑破）——
  `hit_test_agrees_with_the_painted_rects` 抓出（y=796 掉出客户区），
  收紧 `.page-materials` / `.section` / `.section-body` 的间距后通过。
- 实测（CLI 内置 MCP，`zephyra mcp --port 7456`）：两个页的 solid / glass
  开关分别点击 → 轨道色瞬时/过渡切换正确；参考页「材质」行 x=436/550
  两枚开关往返点击，关↔开互切正确；材质页底部「材质」行完整落在窗口内。
- 门禁：`cargo fmt --check` ✓；ui+gallery clippy `-D warnings` 0 ✓；
  `cargo test -p zephyra-ui --lib` **70 passed** ✓；
  `cargo test -p zephyra-gallery` 全绿（含 56 gates）✓。

> **续六（2026-11-06）修正**：solid 的「零动画」被用户否决——solid 也必须有
> 移动的非线性动画；轨道颜色改在 `.zstyle` 中定义。见下。

---

## 追加（2026-11-06 续六）：solid 位移也走 250 ms 快入；轨道色改 `.zstyle` 静态双类

### 现象（修正 续五）

续五把 solid 做成了「全部瞬变（零动画）+ 轨道色由逐帧 `track_bg` 绑定 lerp」。
用户修正：**默认（solid）也要有移动的非线性动画**；**轨道颜色在 `.zstyle` 中定义**。

### 方案

- **solid 位移**：`fraction` 走与 glass **同一条** 250 ms「快入」tween
  （`advance_fraction()` 共享；`cubic-bezier(0,0,0,1)`，中断从当前 fraction
  重走完整 250 ms）。solid 的 `press` / `scale_x/y` 仍瞬变（按压只保留非线性
  移动，不做缩放/白罩/透镜/装饰）。
- **轨道颜色 = `.zstyle` 静态双类**：删除 ToggleSwitch 模板的 `track_bg` 参数
  与轨道 `background="{track_bg}"` 动态绑定；删除 gallery 的
  `switch_track_color_value` / `*_track_color` 绑定及所有实例上的 `track_bg`。
  轨道色完全由 `.switch-track { background: @switch_track }` /
  `.switch-track.on { background: @switch_accent }` 决定——切换即瞬变（`on`
  类随 checked 翻转），把手位移仍是 250 ms tween。`zypui` / `zephyra-style`
  无 `transition` 引擎，静态双类是唯一「定义在 .zstyle」且不引入新机制的形态。
- `controls-toggle.zstyle` 的 `.on` 注释同步（去掉 `track_bg` 覆盖的旧口径）。

### 排除的假设

- **「solid 位移用另一条更慢/更快的曲线」**：未提出需求；与 glass 共用同一
  `direct_ease` / `FRACTION_TWEEN_MS`，行为一致、实现一处。
- **「轨道色做档位动画（`.turn-1..8`）让颜色也平滑且在 .zstyle」**：用户选了
  「静态双类」——颜色瞬切、位移平滑；避免 8 档×主题的规则膨胀。
- **「保留 `track_bg` 绑定但 solid 不用」**：模板/宿主会残留无意义参数；且
  “在 zstyle 中定义”的字面要求与动态绑定冲突。

### 结论 + 门禁

- `advance_fraction()` 抽出为 Solid/Glass 共用；Solid 分支推进 fraction tween、
  但 `press`/scale 瞬变并清 `release_pending`（无需等 fraction 收敛再释 press）。
- MCP 实测（`zephyra mcp --port 7456`，reference 页 switches 材质行，
  `switch:solid` 轨道 path `0.1.1.1.0.2.0.1.1.1.0.0`、knob
  `...0.0.0`，click @468,314）：
  - **knob 位移（非线性）**：`TranslateX` 0 → 10.41 (t+40ms) → 15.64 (t+80ms)
    → 18.12 (t+120ms) → 19.48 (t+160ms) → 20（settled，250 ms 快入）。
  - **轨道色（zstyle 静态双类）**：before `style switch-track animated`、
    `Background = $switch_track (styled)`；settled 后 `style switch-track on
    animated`、`Background = $switch_accent (styled)`——`on` 类翻转驱动，无
    `track_bg` 动态绑定。
- 单测：`solid_toggle_snaps_instantly_and_never_animates` →
  `solid_toggle_animates_fraction_with_fast_in_tween`（钉住 solid 也走 tween、
  反向从当前位置重启）；`solid_press_progress_is_instant` /
  `solid_drag_fraction_is_not_snapped_back` 仍成立。
- 门禁：`cargo fmt --check` ✓；ui+gallery clippy `-D warnings` 0 ✓；
  `cargo test -p zephyra-ui --lib` **49 passed**（toggle 子集全绿，`--lib` 总量
  随 crate 统计口径；以 workspace/gates 为准）/ `cargo test -p zephyra-gallery`
  全绿（含 56 gates）✓。







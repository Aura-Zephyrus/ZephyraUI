# 窗口透明模式 —— 逐像素 alpha 进合成器（2026-11）

> 本篇是**事实层**：标签、路线、约束。决策过程（含「排除的假设」）在
> `docs/dev-log/2026-11-01-window-transparent.md`；DWM backdrop（Mica 一类）的
> 移除理由在 [01-window-attributes.md](01-window-attributes.md) §4。

## 1. 定义与入口

- 配置：`ZephyraUI.json` 的 `window.transparent`（`zephyra-app-config` 的
  [`WindowConfig::transparent`]）。未写默认 `false`。
- 运行时覆盖：`ZEPHYRA_TRANSPARENT=1|0`（含 `ZEPHYRA_FRAMELESS=1|0` 的同款 cwd
  语义），是临时钉住/真机验收用的开关；永久定义在配置与 per-window 两层。
- per-window：`WindowOpts { transparent: Option<bool> }`（创建时显式给定优先）。
  这是「不同窗口可以有不同定义」的入口——透明只能由**创建时**决定，因为窗口的
  alpha 语义一旦交给合成器就固定了。
- 隐含 **transparent ⇒ frameless**：系统标题栏是实色矩形，叠在 alpha 窗口上语义
  不成立。三个后端都用
  `with_decorations(!(frameless || transparent))` 落实，不改配置文件的值。

## 2. 各后端的路线

| 后端 | 窗口属性 | alpha 通路 | clear 色 |
|---|---|---|---|
| `zephyra-winit`（D3D11） | winit `with_transparent`（DWM blur-behind 空区域） | **必须 DComp**：普通 HWND swapchain 实测拒收 `DXGI_ALPHA_MODE_PREMULTIPLIED`（一律 `DXGI_ERROR_INVALID_CALL`），只有 `CreateSwapChainForComposition` + premultiplied 能把 alpha 交给合成器。transparent=true 时无视 `ZEPHYRA_DCOMP=0` 直接走 DComp，且**建链失败即启动失败**（没有「静默退回不透明」这个选项） | 透明时 `theme.surface.with_alpha(0.0)` |
| `zephyra-winit-d3d12` | 同上 | **恒走 DComp**：本后端的唯一呈现路径就是 composition swapchain（premultiplied），所以透明只差窗口属性与 clear | 同上 |
| `zephyra-vulkan` | 同上（X11 换 ARGB visual） | composite alpha 探测恒首选 `PRE_MULTIPLIED`（回退 `OPAQUE`/`INHERIT`）；交换链 clear 恒为 `(0,0,0,0)`，底色由内容 quad 补，所以本侧**只有**窗口属性 | 本就透明，未改 |

内容侧（`zephyra-fx` 的顶点流、D3D11/D3D12 的 blend 状态、CPU 光栅）**本来就全部
按 premultiplied 走**，本轮零改动；`clear_color_is_premultiplied*` 两条既有测试
钉的就是这套语义。

## 3. 与既有机制的相容性

- **flip model「不画不 present」**：D3D12 的 `should_draw_frame` / DComp 提交
  节奏不受影响——透明改的是后缓冲里像素的 alpha 值，不改「哪帧该送」。
- **MSAA**：alpha 与 RGB 同受多重采样与 resolve 管理，无需特殊处理；
  抓帧（`ZEPHYRA_FRAME_DUMP`）读回的是 RGBA，透明窗口的后缓冲含 alpha 信息。
- **`ZEPHYRA_DCOMP`**：语义改为「不透明窗口要不要走 DComp」（诊断/对照用）。
  透明窗口不再受它约束——DComp 是透明的唯一通路，不是可选风味。
- **外观门禁**：透明是窗口属性不是控件外观，不进 `.zstyle`；`gates.rs` 的
  树级扫描不涉及。

## 4. 应用侧约束（写透明应用时必读）

1. **窗口没画到的像素才是透明的**。clear 是 alpha=0，而内容按 premultiplied 叠加
   ——应用根背景画成不透明色，窗口就是不透明的。透明模式对「根留空/留透明」的
   要求与 DWM 材质时代相同（语义现已从材质换成窗口属性）。
2. **标题栏自绘**：transparent⇒frameless，拖动/关窗按钮由应用负责
   （gallery 的 caption-bar 是现成参照）。
3. **半透明令牌**：`@surface_alt_translucent` 之类的 alpha<1 令牌是透明画面的
   原材料；剪裁/圆角沿用的都是既有机制。

## 5. TODO：多窗口的 JSON 定义（本轮未做）

- 现状：仓库不支持多窗口（`live_resize.rs` 的记账是单窗口假设；三个后端各自
  只 `create_window` 一次）。
- 计划形状：`ZephyraUI.json` 增加 `windows` 节，按**命名**定义窗口状态；
  新建窗口按 key 选定义，**缺省回退主配置**（`window` 节）。
  [`WindowOpts`] 就是这条路径的接入点——命名定义携带的字段与它一一对应，
  回退语义即 `WindowOpts.transparent: None`。
- 未做的理由：多窗口牵动 live_resize 的 `HashMap<isize, …>` 改造、事件路由与
  每窗口一份的后端资源表，超出「透明」一条线的验收范围。

[`WindowConfig::transparent`]: ../../../zephyra/zephyra-base/zephyra-app-config/src/lib.rs
[`WindowOpts`]: ../../../zephyra/zephyra-base/zephyra-app-config/src/lib.rs

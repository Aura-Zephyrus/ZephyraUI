# 2026-11-01-window-transparent

> 现象（需求侧）：需要「窗口透明模式」——窗口背后透出来。实现过程中撞出一个
> 既有功能的冲突并整条移除：DWM 背景材质（Mica / Acrylic）与逐像素 alpha 是
> 两套互相排挤的合成路线。

## 排查

### 1. 真正缺的是什么

内容侧早就全是 premultiplied（`zephyra-fx` 顶点流、D3D11/D3D12 blend 状态、
CPU 光栅，`clear_color_is_premultiplied*` 两条测试钉着），所以透明不缺混合语义，
缺的是「窗口层让 alpha 进合成器」。逐后端核：

| 后端 | 缺口 |
|---|---|
| `zephyra-winit-d3d12` | 呈现**恒**走 DComp composition swapchain（premultiplied），只差 winit 窗口属性 + 清屏色 |
| `zephyra-winit`（D3D11） | 普通 HWND swapchain **拒收** `DXGI_ALPHA_MODE_PREMULTIPLIED`（`surface.rs` 建链处实测一律 `DXGI_ERROR_INVALID_CALL`），必须走 DComp |
| `zephyra-vulkan` | composite alpha 探测已首选 `PRE_MULTIPLIED`，交换链 clear 本就 `(0,0,0,0)`，只差窗口属性 |

### 2. 和 Mica 的冲突（为什么它被整条删掉）

`DWMWA_SYSTEMBACKDROP_TYPE` 那条路线（`BackdropKind` / `ApplyOutcome.backdrop` /
gallery 的 `.mica` 分类名）与逐像素 alpha 不能共存：backdrop 一旦设上，窗口走
opaque 合成，DWM 自己画材质；而透明要的是**窗口自己**带 alpha 参与合成。
两者都要「背后透出来」，语义重叠、实现互斥。用户裁定：删 Mica，留透明。

删除范围（不是改默认值，是删类型）：`zephyra-platform` 的 `BackdropKind` /
`WindowChrome.backdrop` / `ApplyOutcome.backdrop` / `Capabilities.backdrop`；
`zephyra-platform-win` 的 `dwm_backdrop` / `DwmAttr::SystemBackdropType` /
`DWM_SYSTEMBACKDROP_TYPE` 的 `AsDwmValue`；`zephyra-platform-linux` 与 `stub` 的
对应位；D3D12 / Vulkan 宿主里的 `backdrop_active` 字段与「材质生效才透明」的
clear 分支；gallery 的 `mica` 状态 / `@mica` 绑定 / `Container.titlebar.mica`
规则 / 两条门禁。**保留**的是窗口内磨砂（`Filter::BackdropBlur` /
`Material::Frosted` / `PSBackdropBlur`）与 D3D12 折射链的「backdrop 副本」——
那是应用内合成，和 DWM 材质同名不同物。

### 3. 排除的假设

- **「Mica 与 DWM spinner 路径本就该共存，问题只是 clear 色」** ❌ —— 曾经真这么
  写过（`backdrop_active` 只影响 clear）。实测口径修正：材质生效时窗口仍是 opaque
  合成，clear 成 alpha=0 只会让**材质区域**露出来，不会让**应用像素**参与 alpha
  合成；所以它给不了「半透明控件叠在桌面上」这种效果，那正是透明要的。
- **「D3D11 可以在普通 HWND swapchain 上直接开 alpha」** ❌ —— `surface.rs` 既有
  实测注释：带 `PREMULTIPLIED` 一律 `DXGI_ERROR_INVALID_CALL`。所以透明在 D3D11
  必须强制 DComp，不能做成"看 ZEPHYRA_DCOMP 心情"。
- **「winit 的 `with_transparent` 在 Windows 是 `WS_EX_LAYERED`」** ❌ —— 抄
  `winit 0.30.13` 源码确认：`attributes.transparent` 走
  `DwmEnableBlurBehindWindow` 空区域；带 `no_redirection_bitmap` 时才不走这条。
  探针测试按这个形态复刻。
- **「Vulkan 侧要补清屏色」** ❌ —— `begin_main_pass` 的 clear 恒
  `(0,0,0,0)`（`renderer.rs:1394-1397`），底色由内容 quad 补；透明在那边只差窗口属性。
- **「多窗口定义这轮顺便定 JSON 形状」** ❌ —— 仓库目前不支持多窗口
  （`live_resize.rs` 单窗口假设、三个后端各自 `create_window` 一次）；只把形状
  写进 TODO（`docs/developer/win/07-window-transparent.md` §5），per-window 的
  真实入口先落在 `WindowOpts`。

## 结论

- **入口三层**：`window.transparent`（主窗口默认，serde default false）→
  `WindowOpts { transparent: Option<bool> }`（per-window 覆盖，创建时定）→
  `ZEPHYRA_TRANSPARENT=1|0`（运行时诊断覆盖，与 `ZEPHYRA_FRAMELESS` 同款）。
  **transparent ⇒ frameless**（系统标题栏是实色矩形，与 alpha 窗口语义冲突），
  三个后端统一 `with_decorations(!(frameless || transparent))`。
- **各后端**：D3D12 恒 DComp（无改动）；D3D11 `dcomp_enabled() || transparent`，
  且 `use_dcomp && transparent` 时建链失败**直接 panic**（退普通链 = 静默变不透明，
  属于改定义）；Vulkan 只加窗口属性。
- **清屏色**：透明窗口 `theme.surface.with_alpha(0.0)`（D3D11 / D3D12 两处，
  各自抽成纯函数 `clear_color` 做门禁；Vulkan 本就透明）。
- **顺带清掉三处对已删功能的引用**：gallery ComboBox 演示选项
  `["Mica","Acrylic","Tabbed","None"]` → `["Solid","Frosted","Glass","None"]`
  （取本仓应用内的材质名）；`theme.rs` 里 `surface_alt_translucent` 的注释
  「让系统背景材质（Mica/Acrylic）透出来」改指窗口透明模式——该令牌**保留**
  （控件层按钮 hover 面在用），只是理由换了。

## 门禁

| 位置 | 钉住的命题 |
|---|---|
| `zephyra-app-config` 8 条 | `window.transparent` 读写 / 缺省 false / `ZEPHYRA_TRANSPARENT` 双向覆盖 / `WindowOpts` 显式值覆盖主窗口默认 |
| `zephyra-dcomp --test smoke::premultiplied_chain_survives_an_alpha_window` | 真机（真窗口 + 真 GPU）：winit 同款 alpha 窗口状态下，premultiplied composition swapchain + visual + commit + present 整链不报错 |
| `zephyra-winit --lib` 3 条 / `zephyra-winit-d3d12 --test transparent_clear_gate` 4 条 | 清屏判据纯函数：透明 ⇒ alpha=0（含「半透明底色也被压到 0」）、不透明 ⇒ 逐字节不变、幂等；两个后端各一份实现、各一份门禁 |
| `zephyra-dcomp --lib` 既有 | composition 交换链 `AlphaMode == PREMULTIPLIED` |
| `zephyra-winit` / `-d3d12` 既有 `clear_color_is_premultiplied*` | clear 进 premultiplied 空间的换算 |
| gallery `the_translucent_surface_token_is_actually_translucent` | 半透明令牌不许被改成实色（Mica 门禁改名后的留存，控件 hover 面仍用它） |
| `zephyra-cli` 28 条 | 生成的 `ZephyraUI.json` 形状（含 `transparent`） |

## 验收（实测）

- `cargo test --workspace`：87 套件，1185 passed / 0 failed / 53 ignored（本轮新增
  app-config 2 条、winit `clear_color` 3 条、d3d12 `transparent_clear_gate` 4 条、
  dcomp 透明探针 1 条）。
- `cargo clippy --workspace --all-targets -- -D warnings`：0 警告。
- `cargo fmt --all -- --check`：干净。
- `cargo doc --no-deps --workspace`：不新增警告（`zephyra-gallery` 的
  `RawUi` 未解析链接在改动前就存在，已用 `git stash` 对照确认）。
- 真窗口对照（D3D11 后端，`examples/demo`，本机真 GPU）：

  | 环境 | 打出 | 含义 |
  |---|---|---|
  | `ZEPHYRA_TRANSPARENT=1` | `窗口装饰: frameless` + `呈现路径: DirectComposition（透明窗口）` | 透明 ⇒ frameless，且**无视**未设的 `ZEPHYRA_DCOMP` 强制 DComp |
  | `ZEPHYRA_TRANSPARENT=0` | `窗口装饰: system` + `呈现路径: DXGI swapchain` | 对照：不透明路径一个字节没变 |

  两轮都正常出帧（`ZEPHYRA_FRAME_DUMP` 读回：800×600，约 620 种颜色）。
  D3D12 后端跑 `ZEPHYRA_TRANSPARENT=1` 同样打 `frameless（透明窗口）` +
  `DirectComposition` 且出帧正常。
- **像素层 alpha 的诚实边界**：demo 与 gallery 的内容都铺满客户区（读回采样点
  `y=2/40/300` 全 `A=255`），所以「未绘制像素是否真透」在两个现成应用上**看不见**
  ——由清屏判据门禁（4 + 3 条）与 dcomp 真机探针顶上。自绘透明应用（根留空）
  的验收条件写在 `docs/developer/win/07-window-transparent.md` §4。
- 转义提醒：`README.md` 与 `gallery` 的若干文件在 git 里是 CRLF，**不要**用脚本
  整文件重写（本轮 `zephyra-app-config/src/lib.rs` 被编辑工具写成 LF 一次，
  已手工恢复 CRLF，diff 从 307 行回到 77 行）。

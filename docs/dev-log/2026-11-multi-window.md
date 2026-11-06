# 多窗口：从「一个进程一个窗口」到 `WindowSet`

日期：2026-11。范围：`zephyra-window`（新 crate）、`zephyra-app-config`、
`zephyra-platform-win` 的实时 resize 钩子、D3D12 / D3D11 后端。

## 现象

本轮之前，三个后端各有一个 `HostWindow<V>`，它以
`winit::application::ApplicationHandler` 的身份**独占**整个事件循环：

- `window_event(_, _id, event)` 的 `_id` 被忽略 —— 多窗口下所有事件都会落到同一个
  窗口上；
- `CloseRequested` 无条件 `event_loop.exit()` —— 关一个工具窗口会退出整个进程；
- `about_to_wait` 自己 `set_control_flow` —— N 个窗口会互相覆盖；
- `zephyra-platform-win::live_resize` 用**进程级** `static HOOK` 与
  `IN_HOOK` —— 第二个窗口装钩子会把第一个顶掉，且一个窗口的重入标记会挡掉另一个
  窗口的 `WM_SIZE`；
- `Gpu` 是 `HostWindow` 的字段 —— 想开第二个窗口就得再建一个设备。

## 排查

要「多窗口」必须先回答三个问题，且它们互不依赖：

1. **谁协调事件**：`WindowSet`（每线程一个），按 `WindowId` 分发；owned 窗口
   级联关闭；最后窗口按 `ShutdownMode` 决定是否退出。
2. **谁共享 GPU**：进程级 `OnceLock<Arc<Mutex<Shared>>>`。D3D12 的 `Shared` 含
   `gpu` + `renderer` + `engine` + `textures`；D3D11 的 `SharedGpu` 只含设备与
   立即上下文（理由见下）。
3. **每个窗口私有什么**：交换链 / DComp visual / `FrameCache` / `View` / 输入与
   焦点状态，以及 D3D12 的 `WindowTargets`（几何缓冲、离屏 RT、目标副本、钩子）。

## 排除的假设

- **「把 `Renderer` 整个留在每个窗口里就行」**：D3D12 的 `srv_heap` 只有 12 个槽，
  0 号恒给图集。两个 `Renderer` 各有自己的图集与 12 槽堆，功能上能跑，但
  `TextureTable` 也要每窗口各注册一份 —— 应用侧每开一个窗口就得重传所有图像纹理。
  所以 D3D12 拆成「共享 `Renderer` + 每窗口 `WindowTargets`」。
- **「D3D11 也照 D3D12 拆 `Renderer`」**：D3D11 的管线状态由驱动隐式缓存，且
  `TextureTable` 直接持有 `ID3D11ShaderResourceView`。共享 `Renderer` 会把两份图集
  挤进同一张 SRV 表，`ID3D11DeviceContext` 也不是线程安全的。所以 D3D11 只共享
  `Gpu`，`Renderer` 仍每窗口一份。**两个后端的共享边界不同，这是有意的**，
  不是漏改。
- **「`live_resize` 按 HWND 存表要加锁会变慢」**：拖动时的 `WM_SIZE` 是热路径，
  但窗口数量是个位数，线性查表比 `HashMap` 更省；实测 6 条钩子测试全绿
  （`hooks_are_per_window` / `reentry_flag_is_per_window` 直接钉住这两条）。
- **「注释说 `props_map.rs` 那条对齐测试会红」**：与本次无关，但顺着这条查时发现
  它并不存在（AGENTS 已记账）。
- **`cargo check` 说 `Fresh` 就是没改**：本轮实测踩到一次 —— 改完 `renderer.rs`
  后 `cargo check -p zephyra-winit-d3d12` 在 0.15s 内 `Finished`，而文件 mtime
  比 fingerprint 记录的时间更新。删掉
  `target/debug/.fingerprint/zephyra-winit-d3d12-*` 后立刻重编并报出真实错误。
  **判据不是「Finished」，而是「这次我的文件被 `Checking` 了」**。

## 结论 + 门禁

已落地：

| 东西 | 位置 | 门禁 |
|---|---|---|
| `WindowSet` / `WindowHandle` / `WindowSpec` / `Backend` / `WindowHost` | `zephyra-base/zephyra-window` | `src/tests.rs` 12 条（级联关闭、句柄失效、命令队列、主题广播、key 不复用、spec 分层） |
| 命名窗口定义 + 未写字段回退 | `zephyra-app-config` | `src/tests.rs` 5 条新增（继承 / 覆盖 / 未知名字回退 / transparent⇒frameless / main 等价） |
| 按 HWND 的实时 resize 钩子 | `zephyra-platform-win/src/live_resize.rs` | 6 条（`hooks_are_per_window`、`reentry_flag_is_per_window` 等） |
| D3D12「共享 `Renderer` + 每窗口 `WindowTargets`」 | `renderer.rs` / `host.rs` | 既有 `d3d12_draw` / `swapchain_draw` / `glyph_draw` / `command_validation` 全绿 |
| D3D12 `D3d12Backend` + `WindowHost` | `host.rs` | 编译期契约 + gallery `--multi-window` |
| D3D11 `D3d11Backend` + `WindowHost`（共享设备） | `zephyra-winit/src/lib.rs` | 既有测试全绿 |
| Vulkan 拆分：进程级 `Device` + 每窗口 `Surface` | `zephyra-vulkan/src/surface.rs` | 渲染钩子实测（Wayland，`#[ignore]`）+ 编译期契约 |
| Vulkan `VulkanBackend` + `WindowHost` | `zephyra-vulkan/src/host.rs` | `tests/backend_contract.rs` 2 条（`Backend` 上界、`Arc<Mutex<Device>>: Send+Sync`）+ gallery `--multi-window` |

`WindowSet` 的线程模型：它含 `Rc`，因此 `!Send` / `!Sync`（编译器强制）。
跨线程用法是在**另一个线程**再建一个 `WindowSet`（自己的 `EventLoop`）；GPU 由
`OnceLock<Arc<Mutex<...>>>` 串行化。⚠ winit 0.30 在 Linux（X11/Wayland）上只允许
主线程拥有事件循环，跨线程用法在 Linux 上会 panic —— 这是 winit 的限制，不是本层的
取舍。

设备丢失恢复：`Shared.generation` 在设备重建时 +1。本窗口重建后其它窗口在下一帧
看到代次不一致，各自 `rebuild_chain`（只重建本窗口的交换链 / 目标，复用新设备）。

**Vulkan 拆分后的共享边界**（与两个 Windows 后端各不相同的第三个版本）：

| 层 | 共享（`OnceLock<Arc<Mutex<Device>>>`） | 每窗口 |
|---|---|---|
| Vulkan | instance / 逻辑设备 / 队列 / 命令池 / MSAA 采样数 | `Surface`（surface + swapchain + format + extent）、`Renderer`、帧缓冲、`FrameCache` |

`Renderer` 每窗口一份的理由与 D3D11 同款：管线把 swapchain 的 `VkFormat`
烤进 `VkRenderPass`，而格式跟着 `Surface` 走；MSAA 目标、离屏 RT、顶点缓冲
也都与窗口尺寸绑定。

**尚未完成**（明确记账，不假装已做）：

- **gallery 的 Inspector 是同一个 `Gallery` 视图的第二个实例**（共享主题意图格），
  不是真正精简的设置页。它验证了动态建窗 / owned 级联 / 共享纹理表，但视觉上不是
  一个「设置面板」。

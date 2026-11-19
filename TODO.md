# TODO — ZephyraUI

> 按轨道分组。✅ 表示已完成（保留最近几条做上下文），其余按优先级排序。
> 每项做完后在 dev-log（`docs/dev-log/`）记账，这里的条目只留结论与指针。

## 进行中（收尾与待验收）

### important：调整base与core架构，组件化管理

### 1. Vulkan：渲染源钩子与命令校验（实现完成，待物理 GPU 复核）

- 已完成 `SegmentPlan::Custom` 保序、`Renderer::register_hook` /
  `HostWindow::register_hook`，record 阶段在主 render pass 中调用钩子。
  `HookCtx` 提供活命令缓冲、ash 设备函数表和目标尺寸；v1 不允许切换
  render pass 或 attachment。
- WSLg 集成门禁通过：llvmpipe（Mesa 25.2.8，LLVM 20.1.2）上注册钩子
  涂红并读回恰好 `7×9 = 63` 个红像素；收窄至 `1×1` 的 scissor 后仍覆盖
  bounds；未注册钩子读回全黑且 `unhandled_custom == 1`。
- 同一门禁启用 Khronos validation layer，反向对照确认 debug-utils 消息
  能被 callback 收集并抽干；正常注册/未注册帧均断言零 validation ERROR。
  反向对照是合成 debug-utils 消息，不代表实际非法 Vulkan 命令测试。
- **仍待物理 GPU 复核**：当前 WSL Vulkan 设备是软件 llvmpipe，不是物理
  GPU。本机有 Intel UHD 630，但 `vulkaninfo.exe --summary` 在枚举过程中以
  `0xC0000005` 崩溃，尚不能据此声称 Vulkan 驱动验收通过。该项仅是驱动覆盖
  待办，不阻塞代码与 WSLg 门禁完成。

### 2. 多窗口 + Vulkan `Backend`（实现完成，待 Linux 运行验收）

- 已落地 `WindowSet` / `Backend` / `WindowHost`（`zephyra-window`），D3D12、
  D3D11、Vulkan 三个后端全部接入；Vulkan 完成「进程级 `Device` + 每窗口
  `Surface`」拆分，`VulkanBackend` 与 gallery `--multi-window` 已接
  （`linux-vk` 与 Windows d3d12 两条路共用同一套自举代码）。
- 门禁：`zephyra-window` 单测、`zephyra-vulkan/tests/backend_contract.rs`
  2 条（`Backend` 上界、`Arc<Mutex<Device>>: Send+Sync`）、全 workspace
  `1206 passed / 0 failed / 53 ignored`、clippy 0 警告。
- **仍待 Linux 运行验收**：本机只装了 `x86_64-pc-windows-msvc`，Wayland
  surface / swapchain、共享设备多窗口、`--multi-window` 只做了编译期与
  人工核对，没有实跑。在 WSL2 上跑：
  `cargo run --manifest-path ../zephyra-gallery/Cargo.toml -p zephyra-gallery --no-default-features --features linux-vk -- --multi-window`
  以及 `cargo run --manifest-path ../zephyra-gallery/Cargo.toml -p zephyra-gallery --bin gallery --no-default-features --features linux-vk`。
  验收点：两个窗口各自 present、主窗口关闭时 Inspector 级联关闭、resize
  后交换链重建、`ZEPHYRA_FRAME_DUMP` 仍能读回。

## 后续（已排期未动工）

- **gallery Inspector 换成真正的 `.zypui` 设置页**：现在的 Inspector 是
  `Gallery` 视图的第二个实例（共享主题意图格），只验证了动态建窗 / owned
  级联 / 共享纹理表，视觉与交互上都不是一个「设置面板」。目标是写一份独立的
  `.zypui` 设置页（主题、frameless / transparent 等运行期可改的窗口选项），
  由自己的 `View` 驱动，而不是复用 Gallery。
- **离屏自绘支持（v2）**：带非恒等滤镜的 Custom 进离屏段时，几何展开层
  画不出宿主内容（现在诚实计数 `unhandled_custom`）。v2 让离屏 pass
  也能调钩子画进 RT。依赖：三个后端钩子先全部落地。
- **自定义布局策略拓展接口（第二缺口）**：Measure/Arrange trait 化，
  允许宿主注入非流式布局算法。与渲染源同一「核心只搬运、宿主解释」
  哲学，设计前先盘点 layout 趟的缓存写回形状（node 枢纽）。

## 已完成（近期，详见 dev-log）

- ✅ 2026-11 多窗口与三后端 `Backend`：`WindowSet` 按 `WindowId` 分发、owned
  窗口级联关闭、`ShutdownMode` 决定最后窗口退出；D3D12 拆「共享 `Renderer`
  + 每窗口 `WindowTargets`」，D3D11 只共享设备，Vulkan 拆「进程级 `Device`
  + 每窗口 `Surface`」并补上 `VulkanBackend`。gallery `--multi-window` 在
  Windows d3d12 与 Linux Vulkan 两条路上共用自举代码。细节见
  [多窗口](docs/dev-log/2026-11-multi-window.md)。
- ✅ 2026-10-26 MCP 输入/文本布局/D3D12 截图与 `.zypui` 渲染源：D3D11 /
  D3D12 真机 MCP 闭环（键盘、滚动、点击后布局变化、截图）；TCP 大截图
  非阻塞续传；`get_layout` 提供文本布局树；`.zypui` 只声明 kind，payload
  由 Host 注入并校验。细节见 [MCP 输入、布局与 D3D12 截图](docs/dev-log/2026-10-26-mcp-input-layout-d3d12-capture.md)。
- ✅ 2026-10-26 MCP 按住拖动、性能分析与资源遥测：按住左键支持折线/直线/easing
  连续路径；D3D11 / D3D12 实机 MCP 逐帧报告与资源读数通过；文本布局的绝对坐标
  和 `get_props` 路径也经真实点击闭环验收。细节见
  [MCP 拖动、性能分析与资源遥测](docs/dev-log/2026-10-26-mcp-clicks-resources.md)。
- ✅ 2026-10-26 MCP 可靠性加固：传输失败丢连接重连重试（应用重启不再废会话）、
  10 秒读超时（冻结应用不挂死 MCP 进程）、`tools/call` 独立工作线程 +
  `notifications/cancelled`（排队跳过/在途中止）、`analyze_performance` 默认只回
  汇总（`include_rows` 取全量）、工具 annotations、`ping` 工具、截图绝对路径回执、
  桥 `on_bridge_disconnect` 断连补抬指针。细节见
  [MCP 重连与超时](docs/dev-log/2026-10-26-mcp-reconnect-timeout.md)。
- ✅ 2026-10-25 外部渲染源拓展接口：`Command::Custom` + `RenderSource`
  （ptr_eq 判等保 diff 静止）+ fx `Segment::Custom` 保序（段数不变量
  2f+2c+1）+ D3D11/D3D12 `RenderHook` 注册表 + 两侧像素级端到端验收
  （D3D11 `ClearView` / D3D12 `ClearRenderTargetView` pRects）。
- ✅ 2026-10-25 core 结构审计与 API 收口；`.zypui` 声明式结构全量落地。
- ✅ 2026-10-25 SoftLight Pegtop 修复 + 压力门禁；D3D 调试层命令校验
  门禁 26 条（8 个存量真 bug）。

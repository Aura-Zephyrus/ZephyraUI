# 渲染后端能力缺口与扩展接口盘点

> 盘点范围：D3D11（`zephyra-winit`）、D3D12（`zephyra-winit-d3d12`）和 Vulkan（`zephyra-vulkan`）宿主的触控输入、帧输出/诊断，以及 `Command::Custom` 扩展接口。
>
> 这是代码与现有实测记录的能力盘点，不代表以下缺口已经修复。需物理设备验证的部分单独标明，不把 Linux 软件 Vulkan/WSLg 验证等同于物理 GPU 验证。

## 结论摘要

1. **触控尚未接入任何宿主。** 三个 `window_event` 都处理 `CursorMoved` 和左键 `MouseInput`，但没有处理 `WindowEvent::Touch`。核心有 `PointerSource::Touch` 和触摸手势逻辑，宿主到核心的输入链却没有把触点送进去。Windows 的触摸不会可靠地转成鼠标事件，不能把鼠标路径当触控支持。
2. **输入能力不一致。** ~~D3D11、D3D12 有键盘和滚轮事件；Vulkan `InputState` 同步时固定提供空键盘事件与空滚轮。~~ **已于 2026-10-28 收口**：三个宿主共用 `zephyra-winit-input` 的同一份翻译，Vulkan 补上了键盘、滚轮与 IME 分支。见 [2026-10-28-input-keyboard-ime](../dev-log/2026-10-28-input-keyboard-ime.md)。
3. **帧输出与截图能力不一致。** D3D11、D3D12 都有调试桥的按需截图；Vulkan 只有首次上屏时可选的 `ZEPHYRA_FRAME_DUMP` 诊断读回，没有 MCP `BridgeHost` 截图路径。
4. **自绘扩展已接入三后端，但不是跨后端的统一 GPU 插件 API。** `Command::Custom` 与 payload/kind 身份跨后端共用；`RenderHook`、GPU 上下文和资源管理按后端分别实现。自绘源的非默认滤镜没有可用的离屏执行，Vulkan hook 也不能更换 render pass 或 attachment。
5. **文档存在过时或不完整的能力描述。** `migration-seam.md` 描述了窗口系统替换时应接入的 Touch 事件，并不代表现有宿主已经接通；触控尺寸文档里“只在触摸路径放大”目前没有可用的宿主触摸路径。D3D12 宿主关于截图未接线的注释则与当前实现及 2026-10-26 真机验收记录相矛盾。

## 1. 触控与输入

| 能力 | D3D11 | D3D12 | Vulkan | 证据与影响 |
|---|---|---|---|---|
| 鼠标移动 / 左键按下抬起 | 已接 | 已接 | 已接 | 三宿主用 `CursorMoved` / `MouseInput`，以 `PointerSource::Mouse` 路由 |
| 原生触点 Down / Move / Up | **未接** | **未接** | **未接** | 三个宿主均无 `WindowEvent::Touch` 分支；触摸屏上无法保证产生鼠标事件 |
| 键盘逻辑键事件 | 已接 | 已接 | 已接（2026-10-28） | 三宿主都过 `zephyra_winit_input::Translator`，产出 core 的 `TextEvent::Key`（语义键 + 键名 + 重复 + 修饰键） |
| 可打印字符 | 已接 | 已接 | 已接（2026-10-28） | 同一翻译器产出 `TextEvent::Char`；Ctrl/Alt/Super 与控制的字符被挡掉 |
| IME（组字 / 提交 / 取消） | 已接 | 已接 | 已接（2026-10-28） | 同一翻译器产出 `TextEvent::Ime`；`Ime::Start` 的插入点暂用最近指针位置代理（焦点系统未建） |
| 滚轮 / 触控板滚动 | 已接 | 已接 | 已接（2026-10-28） | Vulkan 补上了 `MouseWheel` 分支，`scroll` 不再写死 `None` |
| 多指身份到应用输入状态 | **未接** | **未接** | **未接** | core `PointerEvent` 有 id/source，但 fx 的 `PointerInput` 只有坐标和相位；当前宿主队列无法将 id、触点来源交给 `View::on_input` |

核心与宿主的边界需要特别注意：

- core 的 `PointerSource::{Touch, Pen, Mouse}`、固定容量指针表和手势识别证明**核心有部分触摸建模能力**，不能据此声称宿主支持触摸。
- `InputState.pointer_events` 文档当前称其为“鼠标移动与按键边沿”，确实只有 `{x, y, phase}`；完整触控/笔输入需要先定跨后端公共事件契约，至少表达 pointer id、source、phase 和位置。
- [响应式与触控规范](../developer/responsive-and-touch.md) 中触控路径放大目标尺寸的描述是**未兑现的设计口径**。当前宿主没有触摸 source，所以该尺寸策略不会被触摸输入路径触发。
- Gallery 的 `touch_target()` 目前只有定义、没有调用点；它只按 scale 换算目标尺寸，不会识别触摸设备，也没有实际放大 Gallery 中的热区。

### 建议的补齐判据

- 在三宿主接入同一语义的触点生命周期；包括取消/丢失触点的收尾，避免 pointer capture 残留。
- 触点坐标应与命中测试的客户区物理像素坐标系一致；DPI/缩放换算只能有一个明确责任层。
- `InputState` 向应用暴露稳定的 id/source，而不是把触点伪装成鼠标。触摸手势与鼠标合成行为分别验证，避免一次原生触摸被重复路由。
- 将键盘、滚轮、触控的能力差异列为 backend capability 或补齐 Vulkan；当前只在 Vulkan 支持矩阵中标明空实现，容易导致同一应用在 Linux 上静默失去操作。
- 加宿主无关的事件转换测试，以及 Windows 真触摸设备/注入层和 Linux 可用触摸设备上的端到端验收。纯 core 手势测试不能代替宿主事件接线测试。

## 2. 帧呈现、读回与诊断输出

| 能力 | D3D11 | D3D12 | Vulkan | 当前口径 |
|---|---|---|---|---|
| 窗口呈现 | DXGI HWND 交换链；可选 DComp 路径 | D3D12 交换链 + DirectComposition | Vulkan surface / swapchain | 三套宿主分别负责 GPU 命令和上屏，不共享 GPU 实现 |
| 调试桥布局/交互 | 已接 | 已接 | **未接** | D3D11 / D3D12 实机 MCP 闭环有记录；Vulkan 宿主没有持有/轮询 `DebugBridge` |
| 按需 MCP BMP 截图 | 已接 | 已接 | **未接** | D3D11、D3D12 的 `BridgeHost::screenshot` 强制绘制、读回并返回 BMP 像素 |
| 环境变量帧 dump | 已接 | **未接** | 已接 | D3D11 / Vulkan 首帧 dump 是诊断输出，不是交互式截图 API；D3D12 使用独立的按需读回路径 |
| GPU 执行时间戳 | 未提供 | 未提供 | 未提供 | MCP 帧 profile 记录 CPU 帧阶段耗时，不是 GPU timestamp |

已确认的事实与缺口：

- D3D12 `HostWindow::frame_inner(true)` 为截图强制完整帧绘制，`render_pass` 在 resolve 后复制到 readback heap，再经 `BridgeHost::screenshot` 返回。D3D12 MCP 真机 BMP 验收记录为 **1100×700、2,310,054 字节**。源码中声称该后缓冲读回“尚未接线”的模块注释已过时，不能作为当前能力依据。
- Vulkan 的 `ZEPHYRA_FRAME_DUMP` 在首次 `present` 后读回当前交换链图并写 BMP/统计颜色数。它不能由 MCP 客户端按需触发，也没有面向应用的通用截图回调；按需截图与持续诊断 dump 是两种不同接口。
- Vulkan 已有 surface transfer-source 能力检查和图像读回实现，但“底层有读回函数”不等于“宿主有调试桥截图功能”。
- 三后端都遵守交换链重绘约束：不能靠跨帧保留后缓冲做局部重绘；`Unchanged` 必须跳过绘制与 present。该差异不是触控问题，但属于应用观察输出时应保持的帧契约。
- [可移植性现状](../developer/portability.md) 已记录 Vulkan 的高级混合模式仍有降级：未接通的 blend 会回退到 `SrcOver` 并计入 `unimplemented_blends`。这是屏幕输出语义差异，应用不能仅凭“命令已回放”判断效果已一致。
- Vulkan 当前物理 GPU 驱动复核仍待完成；现有 WSLg/软件 Vulkan 像素读回记录是有效门禁，但不覆盖物理驱动行为。

### 建议的补齐判据

- 为 Vulkan 提供按需读回 API，并明确 CPU/GPU 同步、截图尺寸/像素格式、失败错误与 MCP 暴露策略；不要把首次 dump 改造成隐式常驻开销。
- 对三后端采用一致的 BMP/颜色空间定义，并用相同 `DisplayList` 与 CPU 参考图做输出对照；分别覆盖 resize、MSAA 和设备不支持读回的错误路径。
- 输出能力与性能能力分开命名：CPU 阶段计时不可称为 GPU 耗时；当前缺 GPU timestamp 时应保持 unavailable，而非输出伪造的近似值。
- 为 Vulkan 高级 blend 降级保持可观测计数并补齐物理 GPU 验收；不能只看错误日志或 draw call 数。

## 3. 外部渲染源与扩展接口

### 现有接口已经提供什么

- `Command::Custom { bounds, source, filter, blend }` 是 DisplayList 中唯一明确的开放命令位。`RenderSource` 携带字符串 `kind` 与 `Arc<dyn Any + Send + Sync>` payload；fx 负责排序保序，后端按 kind 调用已注册钩子。
- D3D11、D3D12、Vulkan 均有各自的 `RenderHook` 与 `register_hook`；已有未注册时跳过并增加 `unhandled_custom` 计数的路径。D3D11、D3D12 有像素级真 GPU 测试；Vulkan 有 validation 与读回测试，历史验收使用 WSLg/软件 Vulkan。
- 外部渲染源可保持与相邻命令的 DisplayList 顺序；这不是在窗口绘制完成后追加 overlay 的独立通道。

### 目前明确的限制

| 限制 | 具体影响 |
|---|---|
| Hook trait 与 `HookCtx` 按后端各自定义 | 应用共享 `RenderSource` / `Command::Custom` 数据，但 D3D11、D3D12、Vulkan 的实际 GPU hook 代码不能原样复用；这是因为上下文和录制模型不同，不是类型遗漏 |
| 没有统一生命周期/资源接口 | trait 只有 `draw`；设备/管线/纹理创建、resize、设备丢失与恢复、销毁由后端适配代码自行管理。若扩展持有 GPU 资源，需自行对齐宿主设备与同步生命周期 |
| 自绘源的非恒等滤镜没有可用实现 | Custom 命令即使带滤镜，也不会由通用离屏几何回放出自绘内容；会计入未实现项。hook 不能假定 `filter` 自动作用于自己的像素 |
| Vulkan hook 限制在主 render pass / 当前 attachment | 不能从 hook 内切换 render pass 或 attachment；需离屏、resolve、多通道效果时应由后端正式增加能力，不应私自结束宿主 pass |
| GPU 状态由扩展负责 | hook 继承当前后端状态，须设置自身所需状态并遵守文档中的恢复/后续段重绑定契约；不遵守可能污染同帧其余 UI |
| `PushClip` / `PopClip` 目前零生产者 | core 中有裁剪命令定义和消费端，但当前渲染树不会生成它们；不能把“命令存在”理解为应用已能通过普通 UI/API 建立任意裁剪区 |

### 建议的扩展接口路线

1. **先定义跨后端能力声明**：例如 Custom 基本绘制、离屏/滤镜、attachment 切换、截图读回、设备重建；实际未支持时显式报告，不以“已注册 hook”推断所有能力齐全。
2. **维持底层 hook 后端特定**：不要为了让三种 GPU 上下文同型而把 D3D/Vulkan 类型藏进 core。若应用需要一次注册，多加一层应用侧适配/feature-selected factory，而不是让 core 依赖 GPU API。
3. **明确资源生命周期**：补充 prepare/resize/device-lost/device-restored 等宿主通知前，扩展只能自行观察/缓存信息，无法可靠地管理设备重建资源。
4. **继续守住开放位不变量**：未知 `kind` 可见计数、已注册 kind 有像素级验收、Custom 相邻顺序不变；新增字段须由三后端同步消费或显式计入降级报告。

## 源码与验收入口

- [输入事件类型](../../zephyra/zephyra-core/src/input/pointer.rs)
- [应用输入状态与 View](../../zephyra/zephyra-fx/src/lib.rs)
- [DisplayList 自绘命令](../../zephyra/zephyra-core/src/paint.rs)
- [D3D11 宿主](../../zephyra/zephyra-base/zephyra-winit/src/lib.rs)
- [D3D12 宿主](../../zephyra/zephyra-base/zephyra-winit-d3d12/src/host.rs)
- [Vulkan 宿主](../../zephyra/zephyra-base/zephyra-vulkan/src/host.rs)
- [迁移缝清单](../developer/migration-seam.md)
- [MCP 与 D3D12 截图真机记录](../dev-log/2026-10-26-mcp-input-layout-d3d12-capture.md)
- [Vulkan 外部渲染源验收记录](../dev-log/2026-10-25-vulkan-render-hook-validation.md)

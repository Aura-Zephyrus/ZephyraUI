//! 窗口宿主：winit 事件循环 + DComp 视觉树 + D3D12 帧循环。
//!
//! # 与 `zephyra-winit`（D3D11）的关系
//!
//! 两个 crate **互不依赖**，只共同依赖 `zephyra-core`（核心）与 `zephyra-fx`（后端无关
//! 的渲染层）。所以本文件与 D3D11 那份 `HostWindow` 是**两份实现**，不是一份代码两处
//! 条件编译 —— 理由见 `lib.rs`：D3D11 与 D3D12 的失败模式长得几乎一样，混在一个 crate
//! 里时画面一空就分不清是哪个后端的问题。
//!
//! # 字段顺序**不是**随意的（析构约束）
//!
//! Rust 按**声明顺序**析构字段（先用一段小程序实测过：先声明的先 `drop`）。而 DComp
//! 有两条硬约束：
//!
//! 1. `DcompCompositor` 持有 `target` / `root` / `visual`，它必须**比视觉内容**
//!    （也就是交换链）**活得久** —— 否则 visual 还指着已释放的 swapchain，窗口全白；
//! 2. 它又必须**比窗口活得久** —— target 绑在 `HWND` 上，窗口先死就没有「拆除绑定」
//!    这回事了，只是关窗瞬间闪一下黑。
//!
//! 于是顺序必须是「交换链先死 → 合成器 → …… → 窗口最后死」，也就是**反过来声明**：
//! [`Swapchain`] 在最前，[`Window`] 在最后。
//!
//! ⚠ 另一个 crate 曾在注释里把这个结论写反（说「surface 必须声明在 window 之后」），
//! 而那与实测的析构顺序相反。判据不是注释，是上面那两条约束 + 一次实测。

use std::sync::{Arc, Mutex, OnceLock};

use windows::Win32::Foundation::HWND;
use winit::event::WindowEvent as WEvent;
use winit::event::{ElementState, MouseScrollDelta};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use windows::core::Interface;

use zephyra_core::context::Context;
use zephyra_core::diff::DiffStats;
use zephyra_core::frame::{Damage, FrameCache, FrameOutcome};
use zephyra_core::id::ElementKey;
use zephyra_core::input::{InputRouter, PointerPhase, RoutedEvent, RoutedEvents};
use zephyra_core::node::Node;
use zephyra_core::theme::Theme;

use zephyra_fx::{InputState, TextEngine, View};

use zephyra_dcomp::{DcompCompositor, SwapchainDesc, VisualSlot};

use crate::renderer::{RenderCtx, Renderer, TextureTable, WindowTargets};
use crate::surface::Gpu;
use crate::swapchain::Swapchain;
use crate::{Frame, RenderReport};

use zephyra_window::{
    Backend, HostEventOutcome, HostTick, WindowHandle, WindowHost, WindowKey, WindowSpec,
};

/// 窗口 + GPU + 保留树的组合体。
///
/// 保留树（[`Node`]）跨帧存活 —— 这正是保留式相对即时式的收益：状态与纹理得以复用，
/// 只有真正变化的子树被重排/重绘。
///
/// # 字段顺序是**硬约束**，见模块文档
pub struct HostWindow<V: View> {
    /// 交换链（视觉的内容）。**最先析构**。
    chain: Swapchain,
    /// DComp 合成器（持有 target / root / visual）。比 `chain` 活得久，比 `window` 短。
    compositor: Option<DcompCompositor>,
    /// 挂着交换链的那个 visual。单独存一份是为了「交换链重建后重新挂上去」。
    visual: Option<VisualSlot>,
    /// 进程级共享的 GPU 栈（设备、渲染器、文本引擎、纹理表）。
    ///
    /// 多个窗口 / 多个 `WindowSet`（不同线程）共用一个 `Arc<Mutex<Shared>>`，
    /// 所有 GPU 访问都在这把锁里串行 —— D3D12 的命令列表与栅栏是**单份**的。
    shared: Arc<Mutex<Shared>>,
    /// 本窗口私有的渲染目标（几何缓冲、离屏 RT、目标副本、钩子）。
    targets: WindowTargets,
    /// 本窗口的稳定标识（`WindowSet` 分发用）。
    window_key: WindowKey,
    /// 应用侧句柄（命令队列）。单窗口兼容路径下为 `None`。
    handle: Option<WindowHandle>,
    /// 建本窗口交换链时共享栈的代次；与 `shared.generation` 不一致就重建本链。
    seen_generation: u64,
    view: V,
    theme: &'static Theme,
    /// 帧缓存 —— 保留树 + `DisplayList` + 上一帧视口，全在核心里。
    ///
    /// 理由与 `zephyra-winit` 那份**逐字相同**：这三样原来分散在本结构体的
    /// `node` / `list` / `needs_layout` 里，而数据全在 core，于是本 crate 要自己
    /// 编排「要不要重排 / 要不要重绘」—— 那正是 base 层第二条契约禁止的。
    cache: FrameCache,
    router: InputRouter,
    pointer: Option<u32>,
    /// 桥注入的「主指针按住」状态。断连收尾靠它判断要不要补一次抬起
    /// （见 [`BridgeHost::on_bridge_disconnect`]）——`pointer` 是路由层的
    /// 指针 id，与「按钮按没按住」是两回事，不能拿它当判据。
    bridge_pointer_pressed: bool,
    pointer_pos: (f32, f32),
    pending_text_events: Vec<zephyra_core::input::TextEvent>,
    /// winit 按键 → core 文本事件的翻译器（修饰键状态也住在这里）。
    ///
    /// 三后端共用 `zephyra-winit-input` 里的同一份实现 —— 曾经每个宿主各抄一份
    /// `logical_key_name`，而 Vulkan 那份干脆不存在。
    translator: zephyra_winit_input::Translator,
    /// winit 指针事件 → core 指针事件的翻译器（**含单调时钟**）。
    ///
    /// ⚠ 触控这条路**必须**有它：`winit` 的 `Touch` 事件不带时间戳，而静态
    /// 手势（轻点 / 双击 / 长按）是计时的。
    ///
    /// 三后端共用 `zephyra-winit-input` 里的同一份实现 —— 抄三份就意味着
    /// 三份会漂移（键盘那条路已经漂移过一次）。
    pointer_translator: zephyra_winit_input::PointerTranslator,
    /// 焦点状态机：Tab 移动、点击设焦点、失焦清空。跨帧持有。
    focus: zephyra_core::input::FocusState,
    /// 键盘激活状态机：焦点落在 ACTIVATABLE 节点上时 Space/Enter 闩住 pressed。
    activation: zephyra_core::input::ActivationState,
    pending_scroll: Option<zephyra_fx::ScrollInput>,
    pending_pointer_events: Vec<zephyra_fx::PointerInput>,
    /// 待发的**静态手势**（轻点 / 双击 / 长按）。
    ///
    /// 官方触控指南把它们单列一档：它们在**交互完成后**才触发。
    pending_gestures: Vec<zephyra_fx::GestureEvent>,
    /// 待发的**操作增量**（多指平移 / 缩放 / 旋转 + 速度）。
    pending_manipulations: Vec<zephyra_core::input::ManipulationDelta>,
    /// 操作结束时的平移速度（px/秒）—— **惯性的初值**。
    pending_inertia: Option<(f32, f32)>,
    /// 有没有**待处理的事件**（resize / ptr_of / 按键 / 图集变化）。
    ///
    /// ⚠ 这**不是**「内容变了」。内容变没变由 core 判定（`FrameOutcome`）。
    /// 本字段只表示「值不值得跑一帧」—— 因为判断后者必须先跑 `view()`，而那些
    /// 应用代码。
    dirty: bool,
    /// 窗口是否可见（未最小化、未被完全遮挡）。
    ///
    /// `WindowEvent::Occluded` / `WindowEvent::Focused` 维护。不可见时 `tick()`
    /// 直接返回空帧、不 layout、不 paint、不 present。恢复可见时
    /// `cache.invalidate()` 强制下一帧完整重绘。
    window_visible: bool,
    /// 最近一次**输入事件**到达的时刻。
    ///
    /// # 为什么需要它
    ///
    /// 「画面跟手吗」要**测**才知道：从输入到达，到承载它的那一帧就绪，
    /// 中间花了多久。这一段里藏着事件循环的控制流、命中测试、diff、布局、
    /// 绘制 —— 不测就只能猜。
    ///
    /// `None` = 这一帧之前没有输入。
    input_at: Option<std::time::Instant>,
    /// 上一帧测到的「输入 → 帧就绪」耗时（微秒）。
    input_latency_us: Option<u64>,
    /// 上一帧 `render_pass` 的耗时（微秒）—— 临时探针。
    render_submit_us: u64,
    /// resize 的状态（本帧）。`frame()` 据它决定要不要画。
    swap_state: SwapState,
    /// 连续 resize 失败的次数。只给 trace 用：它是「为什么拖动时不动了」的判据。
    resize_failures: u32,
    /// 客户区尺寸**最后一次变化**的时刻 + 当时的尺寸。拖动去抖用。
    ///
    /// # 为什么要有它
    ///
    /// 本机 `ResizeBuffers` 在后缓冲被用过之后就再也调不成功（实测矩阵见
    /// [`Swapchain::resize`] 文档），所以尺寸一变就只能**重建整条交换链**。而拖边时
    /// `WM_SIZE` 每秒来几十次 —— 每次都重建，实测一口气重建 140 次，GPU 直接进
    /// `DEVICE_HUNG`，屏幕上就是「一直闪」。
    ///
    /// 所以这里**不再跟着每一次 `WM_SIZE` 重建**：尺寸还在变的时候只把已有画面
    /// 缩放铺满窗口（见 [`Self::stretch_visual_to_client`]），等它**稳下来**才真的
    /// 重建一次。于是一次拖动 = 1 次重建，而不是 140 次。
    /// 「设备已不可用」是否已经报过一次。见 [`Self::report_device_dead_once`]。
    reported_device_dead: bool,
    /// 上一次**尝试恢复设备**的时刻。`None` = 还没试过（于是第一次就试）。
    ///
    /// 设备重置后要按间隔重建整条 GPU 栈才恢复，见 [`Self::try_recover_device`]。
    last_recovery_attempt: Option<std::time::Instant>,
    /// 本窗口是否透明（逐像素 alpha 进合成器）。
    ///
    /// resolution 在 [`Self::new_lenient_opts`] 里做完：`WindowOpts` 显式给定
    /// 优先，否则读主窗口配置 `window.transparent`。创建时确定，运行期不变。
    transparent: bool,
    /// 帧计数。只给 `trace` 用。
    /// 系统语言偏好（首帧读一次后缓存）。
    ///
    /// 每帧问一次是一次 Win32 调用，而它是**启动时的外部事实**，之后不变
    /// （用户切换系统语言要重启进程才该重新读）。与版本号、系统主题同一手法。
    system_languages: Vec<String>,
    frames: usize,
    /// 上一次 `sync_input` 的时刻，用来算**真实帧间隔**（动画时钟的唯一来源）。
    ///
    /// `None` = 还没有上一帧，于是 dt 为 0。首帧没有"距上一帧多久"可言，
    /// 填 0 而不是填一个猜的值：猜的值会让首帧动画从中间开始。
    last_frame_at: Option<std::time::Instant>,
    /// 上一帧布局用的视口（有效像素）。**只给 trace 用**。
    ///
    /// 存在的理由与 `FrameCache::viewport` 不同：那条是 core 的**判据**，决定要不要
    /// 重排；这一份纯粹是「resize 之后到底发生了什么」的观测点。
    /// 两者的值应当一致 —— 不一致本身就是缺陷（见 `docs/dev-log/task-responsive.md` B2）。
    traced_viewport: Option<(f32, f32)>,
    /// 调试桥（`ZEPHYRA_DEBUG_PORT` 开启时存在）。见 `zephyra_fx::bridge`。
    bridge: Option<zephyra_fx::bridge::DebugBridge>,
    /// 最近一帧的统计（`stats` 命令的答案，`frame()` 末尾刷新）。
    ///
    /// 结构体而不是文本：文本形态由 [`zephyra_fx::bridge::FrameStats`] 的
    /// `Display` 出，JSON 形态由它的 `to_json` 出 —— 两份口径同源。
    last_stats: zephyra_fx::bridge::FrameStats,
    /// **渲染过**的帧数（`wait_frames` 按它算）。
    ///
    /// 为什么数「渲染过」而不是「跑过」：静止的应用不出帧，而排查动画的人
    /// 要等的是「下一帧的画面」—— 跑过但没画不算。
    rendered_frames: u64,
    /// 上一次**呈现**的时刻。
    ///
    /// # 它是「GPU 占用」的上界
    ///
    /// GPU 占用 = 呈现帧数 × 每帧 GPU 时间。而呈现帧数在本后端**不受限** ——
    /// 动画期间事件循环切 `ControlFlow::Poll`，能画多快就画多快。
    ///
    /// 于是「把 GPU 占用压到 X%」这件事**没有上界**：一帧便宜了就画得更勤，
    /// 占用原地弹回原处。**先给呈现率封顶**，后面的每一点优化才兑现成
    /// 「占用下降」而不是「帧率上升」。
    ///
    /// 封顶值取**显示器的刷新率**（不是硬编码 60）：120 Hz 屏上按 60 封顶
    /// 会白白丢掉一半的流畅度。
    last_present_at: Option<std::time::Instant>,
    /// 呈现之间的最小间隔（= 1 / 刷新率）。
    present_interval: std::time::Duration,
    resource_telemetry: Option<zephyra_platform_win::ProcessTelemetry>,
    profiler: zephyra_fx::bridge::PerformanceProfiler,
    /// 无障碍（UIA）接线。
    ///
    /// `None` = 没装（默认）。装了之后每帧发布一份无障碍树快照，并把 UIA 侧
    /// 排队的动作转成「聚焦 + 键盘激活」交给应用。
    ///
    /// 为什么是 `Option`：无障碍是**附加能力** —— 默认不装，屏幕阅读器就看不见
    /// 这个界面，而应用代码一行都不用改。要不要装由应用在 `Bootstrap::resumed`
    /// 里决定。
    uia: Option<zephyra_uia::Bridge>,
    /// UIA 侧刚改过状态 ⇒ 还需要再跑一帧把它画出来。
    ///
    /// # 为什么单独一个字段
    ///
    /// 应用是在 `sync_input` 里改的状态，而那发生在**本帧的树建好之后** ——
    /// 于是新状态要等**下一帧**才画得出来。
    ///
    /// 而本后端静止时一帧都不跑（事件驱动），所以必须显式记着「还欠一帧」，
    /// 否则那个勾不会出现在屏幕上 —— 而应用那边已经改了状态，画面与状态就
    /// 悄悄对不上了。
    uia_redraw: bool,
    /// 窗口。**最后析构**，见模块文档。
    window: Arc<Window>,
}

/// 进程级共享的 GPU 栈。
///
/// # 为什么这些是共享的
///
/// 「一个进程一个设备」不是可选项：D3D12 的设备 / 命令队列 / 栅栏是**单份**的，
/// 每窗口各建一套会让「谁的队列在跑」无法回答，而且驱动对多设备共享纹理的支持很脆。
/// 图集与纹理表、PSO 也一样 —— 它们描述的是*怎么画*，与窗口无关。
///
/// 每窗口自己的东西在 [`HostWindow::targets`]（[`WindowTargets`]）。
///
/// # 锁
///
/// 所有字段都在 `SHARED` 的 `Mutex` 里。一次帧（布局 + 记录 + 提交）在同一把锁内
/// 完成，所以两个窗口不会同时操作命令列表。这也是「跨线程再起一个 `WindowSet`」
/// 能成立的全部原因：锁串行化，而不是每个线程各建一份设备。
pub(crate) struct Shared {
    /// 设备、命令队列、命令列表、栅栏。
    pub(crate) gpu: Gpu,
    /// 管线状态、图集、SRV 描述符堆、纹理表。
    pub(crate) renderer: Renderer,
    /// 文本引擎（字形来源 + 图集位图）。
    ///
    /// `RefCell` 是为了让 core 的 `Context` 保持 `Copy` —— 布局递归要求这一点。
    pub(crate) engine: std::cell::RefCell<TextEngine>,
    /// 已上传的图集 revision，用来避免每帧重传 4MB。
    pub(crate) atlas_revision_uploaded: u64,
    /// 已注册的图像纹理（名字 → SRV 槽位）。
    pub(crate) textures: TextureTable,
    /// 设备代次。设备重建时 +1；各窗口据此重建自己的交换链。
    pub(crate) generation: u64,
}

// SAFETY: `Shared` 只通过 `SHARED` 的 `Mutex` 访问，所有 D3D12 调用都在锁内串行。
// `Gpu` 里的 COM 指针与栅栏事件句柄本身是自由线程的；`TextEngine` 只在锁内被一个
// 线程使用（它没有线程局部状态 —— 那正是 `Windows` 上每线程一个事件循环仍然共用
// 同一份字形图集的前提）。
unsafe impl Send for Shared {}

/// 进程级唯一一份共享栈。
static SHARED: OnceLock<Arc<Mutex<Shared>>> = OnceLock::new();

/// 取（必要时建）进程级共享栈。
///
/// 第一个窗口建它；之后每个窗口（含别的线程上的 `WindowSet`）拿到同一个 `Arc`。
fn shared_stack() -> Arc<Mutex<Shared>> {
    SHARED
        .get_or_init(|| {
            // 文本引擎：core 的 `TextStack` + ab_glyph 字形来源。找不到字体时退回禁用模式。
            let (engine, note) = zephyra_fx::from_system_font();
            eprintln!("[zephyra-d3d12] {note}");
            let gpu = Gpu::new().expect("创建 D3D12 设备：需要可用的 GPU");
            // `msaa_samples()` 是**进程级**环境变量读取，所以所有窗口的采样数一致 ——
            // 共享的那个 `Renderer` 的 PSO 采样数因此对每个窗口都成立。
            let renderer = Renderer::new(
                &gpu.device,
                zephyra_core::text::atlas::ATLAS_SIZE,
                crate::swapchain::msaa_samples(),
            )
            .expect("建根签名 / PSO / 描述符堆");
            Arc::new(Mutex::new(Shared {
                gpu,
                renderer,
                engine: std::cell::RefCell::new(engine),
                atlas_revision_uploaded: 0,
                textures: TextureTable::new(),
                generation: 0,
            }))
        })
        .clone()
}

/// 锁住共享栈。锁中毒时继续用（`panic` 的那个窗口已经挂了，其余窗口不该跟着死）。
pub(crate) fn lock_shared(shared: &Arc<Mutex<Shared>>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// 已在锁内的调试层消息拉取（`frame_inner` / `render_pass` 走这条，避免自锁）。
fn drain_debug_shared(shared: &Shared) {
    shared.gpu.drain_debug();
}

/// 未指定尺寸时的默认客户区尺寸。
///
/// winit 的默认值在本机是 640×480 —— 对按视口自适应的应用够用，对**固定画布**
/// 的应用（`width: 1400px` 那类）会裁掉一大块。要固定画布就用
/// [`HostWindow::new_lenient_sized`]。
pub const DEFAULT_WINDOW_SIZE: (u32, u32) = (1100, 700);

/// [`HostWindow::sync_swapchain`] 的结果：这一帧交换链与客户区的关系。
///
/// # 为什么要三个值而不是原来的 `bool`
///
/// 原来的 `bool` 只有一个含义「尺寸变了没」，而宿主实际需要分辨的是**三件事**，
/// 其中两件的处置完全相反：
///
/// | 值 | 含义 | 宿主该做什么 |
/// |---|---|---|
/// | [`Self::Same`] | 尺寸本来就没变 | **照常画**（鼠标在窗口里动也走这条） |
/// | [`Self::Synced`] | 刚 resize 完，链与客户区一致 | 照常画 |
/// | [`Self::Failed`] | resize 想做没做成，链还停在旧尺寸 | **本帧空过** |
///
/// `Failed` 被原来的 `bool` 和 `Same` 混为同一个 `false`，于是宿主照旧画 + present ——
/// DComp 把**旧尺寸**的内容拉进**新**窗口，那就是「窗口一直闪 / 内容被拉伸」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapState {
    /// 尺寸没变，什么都没做。
    Same,
    /// resize 成功，链已与客户区一致。
    Synced,
    /// resize 失败但设备还活着（GPU 还攥着后缓冲那类）。
    ///
    /// 此时**不能**画：链还停在旧尺寸，画了就是把旧内容拉进新窗口。
    Failed,
}

/// 设备恢复尝试的最小间隔。
///
/// 恢复要**重建整条 GPU 栈**（设备、交换链、DComp、渲染器），不便宜；而驱动重置后
/// 通常几百毫秒内就恢复。1 秒是这两者之间的稳妥点：更短会让每次尝试都撞在
/// 「还没恢复」上，更长则让用户多等。
const DEVICE_RECOVERY_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// 距上一次恢复尝试已经过了 `elapsed`，这一帧要不要再试。**纯函数**。
///
/// 与 [`should_draw_frame`] 同一手法：恢复要撞上的是真机上偶发的
/// `DEVICE_RESET`（见 `docs/dev-log/2026-10-gpu-device-hung.md`），而这条修复的
/// 全部价值就在那一刻。没有门禁的话，把它写错等于没有修复。
pub fn should_attempt_recovery(elapsed: std::time::Duration) -> bool {
    elapsed >= DEVICE_RECOVERY_RETRY_INTERVAL
}

/// 这一帧到底该不该画。**纯函数** —— 于是能在没有 GPU 的机器上门禁。
///
/// # 为什么必须是纯函数
///
/// 这个判断是「设备消失 / resize 失败」两条规则的唯一汇合点：
///
/// 1. 设备已死 ⇒ 不画（否则每帧三连错 + present 一个无效 visual = 闪）；
/// 2. resize 想做没做成 ⇒ 不画（否则把旧尺寸内容拉进新窗口 = 闪 / 拉伸）。
///
/// 两条都只在**真机拖动**时复现，而本机 D3D12 起不来（`tests/d3d12_draw.rs`
/// 头部记着证据，`tests/swapchain_draw.rs` 的真机用例因此全是 `#[ignore]`）。
/// 抽成 `(SwapState, bool) -> bool` 就能在没有设备的环境里断言它 ——
/// 否则这条修复等于没有门禁。
///
/// `Same` 与 `Synced` 都返回 `true`：前者是「尺寸没变」的**常态**（鼠标在窗口里
/// 动也走这条），误把它当成「别画」会让 UI 完全不响应，而那比原来的 bug
/// 更难发现。
pub fn should_draw_frame(state: SwapState, device_dead: bool) -> bool {
    if device_dead {
        return false;
    }
    !matches!(state, SwapState::Failed)
}

/// 本帧的清屏色。**纯函数**：透明窗口清成 alpha=0，否则用主题底色。
///
/// 与 `zephyra-winit` 的同名函数是**同一判据的两份实现**——两个后端各写一份
/// 是因为它们不互相依赖（后端三原则），但判据必须一致，所以两边各有一条
/// 无 GPU 门禁（本 crate 见 `tests/transparent_clear_gate.rs`）。不抽到共享
/// crate 的理由：`zephyra-fx` 是「后端无关的那一半」的住处，而 clear 色是
/// **窗口呈现策略**，不是绘制词汇。
///
/// 抽成纯函数的理由与 [`should_draw_frame`] 相同：`--dump-cpu` 不呈现窗口，
/// 这条判据只有在真窗口下才看得见，而没有门禁的结论会烂掉。
pub fn clear_color(
    transparent: bool,
    surface: zephyra_core::core::Color,
) -> zephyra_core::core::Color {
    if transparent {
        surface.with_alpha(0.0)
    } else {
        surface
    }
}

impl<V: View> HostWindow<V> {
    /// 建窗口与 GPU 资源，**找不到系统字体时不 panic**。
    ///
    /// 字体缺失不该让整个应用起不来：文本不显示，其余部分照常。这与「崩溃」之间的
    /// 差别很大 —— 后者让「这台机器没装字体」变成「这个程序不能跑」。
    pub fn new_lenient(event_loop: &ActiveEventLoop, view: V) -> Self {
        Self::new_lenient_opts(
            event_loop,
            view,
            DEFAULT_WINDOW_SIZE,
            zephyra_app_config::WindowOpts::default(),
        )
    }

    /// 扩展上下文句柄：外部纹理/Hook 注册（按窗口 targets）。
    pub fn ext_context(&self) -> zephyra_ext::ExtContextHandle {
        let shared = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        self.targets.ext_handle(&shared.gpu.device)
    }

    /// [`Self::new_lenient`] + **指定初始客户区尺寸**（**物理像素**）。
    ///
    /// # 为什么必须有这条
    ///
    /// `winit` 在不给尺寸时用系统默认值（本机是 **640×480**），而按固定画布
    /// 布局的应用（`box.window { width: 1400px }` 那类）会在这个窗口里
    /// 被**裁掉一大块** —— 而且裁得毫无提示：布局是对的、绘制是对的，
    /// 只是有一多半在窗口外，看起来像「排版坏了」。
    ///
    /// # 为什么是**物理**像素而不是逻辑像素
    ///
    /// 核心里布局的单位是**像素**，而且宿主把交换链的像素尺寸原样当作视口传进去
    /// （见 [`Self::client_size`]）—— 即整条链是「像素对像素」的。若这里收逻辑
    /// 尺寸，高 DPI 下窗口会被放大（实测 125%：逻辑 1400×800 → 物理 1750×1000），
    /// 于是固定画布的内容只占窗口左上角，右/下留出空白：布局没错、绘制没错，
    /// 只是"窗口比界面大了一圈"。
    ///
    /// 症状与「布局算错」难以区分，所以画布尺寸必须由**应用**按像素给。
    /// `new_lenient` 保留默认尺寸，是为了不改变既有调用点（按视口自适应的应用
    /// 不受影响）。
    pub fn new_lenient_sized(event_loop: &ActiveEventLoop, view: V, size: (u32, u32)) -> Self {
        Self::new_lenient_opts(
            event_loop,
            view,
            size,
            zephyra_app_config::WindowOpts::default(),
        )
    }

    /// [`Self::new_lenient_sized`] + **按窗口的创建时选项**。
    ///
    /// 透明是 per-window 参数（[`zephyra_app_config::WindowOpts`]，显式给定
    /// 优先于 `window.transparent`），且**transparent ⇒ frameless**：
    /// 系统标题栏是实色矩形，叠在 alpha 窗口上语义不成立。
    /// 窗口属性之外，DComp 迷你链（composition swapchain + premultiplied）
    /// 才是 alpha 真正进合成器的通路 —— 这条**本来就**是本后端的唯一呈现
    /// 路径，所以透明在本后端只差 winit 的 `with_transparent` 与清屏色。
    pub fn new_lenient_opts(
        event_loop: &ActiveEventLoop,
        view: V,
        size: (u32, u32),
        opts: zephyra_app_config::WindowOpts,
    ) -> Self {
        let config = zephyra_app_config::load_from_current_dir();
        let transparent = opts.resolve_transparent(&config);
        let frameless = opts.resolve_frameless(&config);
        let resizable = opts.resolve_resizable(&config);
        let title = opts
            .title
            .clone()
            .unwrap_or_else(|| "ZephyraUI (D3D12)".to_string());
        let attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(winit::dpi::PhysicalSize::new(size.0, size.1))
            .with_decorations(!frameless)
            .with_transparent(transparent)
            .with_resizable(resizable);
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("create_window（D3D12 后端）"),
        );
        // winit **默认不允许** IME（`Window::set_ime_allowed` 的文档原话），
        // 不开就永远收不到 `WindowEvent::Ime` —— 那条分支会变成死代码。
        //
        // ⚠ 真实应用应当「文本框获得焦点时才开」：组字期间 winit 不送
        // `KeyboardInput`，于是组字中的快捷键本来也不生效。焦点系统还没建，
        // 所以先全局打开；有了焦点系统后改成随焦点开关。
        window.set_ime_allowed(true);
        if opts.resolve_always_on_top(&config) {
            window.set_window_level(winit::window::WindowLevel::AlwaysOnTop);
        }
        eprintln!(
            "[zephyra-d3d12] 窗口装饰: {}{}",
            if frameless { "frameless" } else { "system" },
            if transparent {
                "（透明窗口）"
            } else {
                ""
            }
        );
        let size = window.inner_size();
        // 宿主必须能**自己说出**坐标系：「点哪儿不好使」这类问题的头号嫌疑就是
        // 事件坐标（物理）与布局坐标（DIP）不在一个空间里，而画面永远是"对的"，
        // 只有命中会偏 —— 不把这三个数打出来就只能靠猜。
        eprintln!(
            "[zephyra-d3d12] 窗口: inner_size(物理) {:?} scale {}",
            window.inner_size(),
            window.scale_factor()
        );

        // 共享 GPU 栈：第一个窗口建它，之后同一进程的所有窗口共用。
        let shared = shared_stack();
        let shared_guard = lock_shared(&shared);
        let gpu = &shared_guard.gpu;

        // 后缓冲格式必须与 PSO 的 `RTVFormats[0]` 一致（`R8G8B8A8`）。用
        // `SwapchainDesc::default()` 的 `B8G8R8A8` 会让 D3D12 在 `DrawIndexedInstanced`
        // 时逐槽比对并报「render target format does not match」，而画面全错、
        // 不报任何 HRESULT。
        let desc = SwapchainDesc {
            buffer_count: 2,
            format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM,
        };
        let chain = Swapchain::new(
            gpu,
            (size.width, size.height),
            &desc,
            crate::swapchain::msaa_samples(),
        )
        .expect("创建 composition swapchain");

        // composition swapchain **必须**挂在 visual 上才会被合成，否则窗口是黑的。
        // 所以这里失败要如实说出来，不能只打一行 warn 就继续跑。
        let (compositor, visual) = match hwnd_of(window.as_ref()) {
            Some(h) => match attach_visual(HWND(h as *mut _), gpu, &chain) {
                Ok((c, v)) => {
                    eprintln!("[zephyra-d3d12] 呈现路径: DirectComposition");
                    (Some(c), Some(v))
                }
                Err(e) => {
                    eprintln!("[zephyra-d3d12] ⚠ DComp 建树失败（{e}）");
                    eprintln!(
                        "[zephyra-d3d12] ⚠ composition swapchain 没有 visual 就不会被合成，\
                         窗口会是黑的（这一条不可能靠参数调好）"
                    );
                    (None, None)
                }
            },
            None => {
                eprintln!("[zephyra-d3d12] ⚠ 取不到 HWND，无法建立 DComp target，窗口会是黑的");
                (None, None)
            }
        };

        let atlas_size = zephyra_core::text::atlas::ATLAS_SIZE;
        let _ = atlas_size;
        // ⚠ 采样数必须**取自交换链**，不能在这里另写一个常量 —— PSO 建好就固定了，
        // 而它必须与实际绑定的渲染目标一致（见 `Swapchain::samples` 的文档）。
        //
        // 共享 `Renderer` 的采样数在 `shared_stack()` 里由同一个进程级
        // `msaa_samples()` 决定，所以与这里的 `chain.samples()` 一致。
        let targets =
            WindowTargets::new(&gpu.device).expect("建本窗口渲染目标（几何缓冲 / 离屏 RTV 堆）");
        let generation = shared_guard.generation;
        drop(shared_guard);
        eprintln!(
            "[zephyra-d3d12] MSAA: {}×",
            if chain.samples() > 1 {
                "开"
            } else {
                "关（无多重采样目标）"
            }
        );
        // 二分开关必须**自己打出来**：跑完一轮二分，回头看日志时如果分不清
        // 「关掉 MSAA 之后不挂了」和「那次根本没设环境变量」，这轮二分就白做。
        // 这一行让每份日志自带生效配置。
        if crate::renderer::filters_disabled() {
            eprintln!("[zephyra-d3d12] 离屏滤镜：整条关掉（ZEPHYRA_NO_FILTERS）");
        }

        let theme: &'static Theme = Box::leak(Box::new(Theme::dark()));
        Self {
            chain,
            compositor,
            visual,
            shared,
            targets,
            window_key: WindowKey::unspecified(),
            handle: None,
            seen_generation: generation,
            view,
            theme,
            cache: FrameCache::new(),
            router: InputRouter::new(),
            pointer: None,
            bridge_pointer_pressed: false,
            pointer_pos: (0.0, 0.0),
            pending_text_events: Vec::new(),
            translator: zephyra_winit_input::Translator::new(),
            pointer_translator: zephyra_winit_input::PointerTranslator::new(),
            focus: zephyra_core::input::FocusState::new(),
            activation: zephyra_core::input::ActivationState::new(),
            pending_scroll: None,
            pending_pointer_events: Vec::new(),
            pending_gestures: Vec::new(),
            pending_manipulations: Vec::new(),
            pending_inertia: None,
            dirty: true,
            window_visible: true,
            input_at: None,
            input_latency_us: None,
            render_submit_us: 0,
            reported_device_dead: false,
            last_recovery_attempt: None,
            swap_state: SwapState::Same,
            resize_failures: 0,

            transparent,
            frames: 0,
            system_languages: zephyra_platform_win::current().system_languages(),
            last_frame_at: None,
            traced_viewport: None,
            bridge: zephyra_fx::bridge::DebugBridge::from_env(),
            last_stats: zephyra_fx::bridge::FrameStats::default(),
            rendered_frames: 0,
            last_present_at: None,
            present_interval: refresh_interval(&window),
            resource_telemetry: None,
            profiler: zephyra_fx::bridge::PerformanceProfiler::new(),
            uia: None,
            uia_redraw: false,
            window,
        }
    }

    /// Install a styler (called every frame after diff, before layout).
    ///
    /// Forwards to `FrameCache::set_styler`. Kept out of `new`'s signature on
    /// purpose: a stylesheet is a run-time thing (and dev hot-reloads it),
    /// while the window must exist at startup. Folding it into the constructor
    /// would make "start the window" depend on "the stylesheet parsed" -- one
    /// wrong property name and the app will not launch.
    pub fn set_styler(&mut self, styler: Box<dyn zephyra_core::styler::Styler>) {
        self.cache.set_styler(styler);
    }

    /// 切换主题（Light / Dark）。
    ///
    /// # 为什么必须显式作废缓存
    ///
    /// 资源键是**读取时**解析的（`resolve_resource` 不写回属性表），所以换主题
    /// 不需要动树——但 `FrameCache` 可能正处在稳态（`Unchanged`），那一帧会整帧
    /// 跳过，新主题要等下一次别的事件才上屏。这里立刻 `invalidate`，让下一帧
    /// 必然重画重解析。与 `set_styler` 同一个理由。
    ///
    /// `Theme` 是 `Copy` + 这里 `Box::leak` 成 `&'static`：与构造时同一手法
    /// （进程内切主题次数有界，泄漏的是两份 200 字节的结构）。
    pub fn set_theme(&mut self, variant: zephyra_core::theme::ThemeVariant) {
        let theme: &'static Theme = Box::leak(Box::new(Theme::builtin(variant)));
        self.theme = theme;
        self.cache.invalidate();
        // 换主题本身就是「该重画」的信号。`cache.invalidate()` 只清 core 的判据，
        // 而 `tick` 先看宿主自己的 `dirty` —— 漏掉这一行时，事件驱动宿主在静止
        // 状态下换主题**不会**画出新主题，要等下一次输入。
        self.dirty = true;
    }

    /// 当前主题变体（应用据此判断「想切的」与「在用的」是否一致）。
    pub fn theme_variant(&self) -> zephyra_core::theme::ThemeVariant {
        self.theme.variant
    }

    /// 已注册的图像纹理数量。
    ///
    /// 纹理表是进程级共享的（`Shared::textures`，私有字段），所以没法返回一个
    /// `&TextureTable`。注册走 [`Self::register_texture`]。
    pub fn texture_count(&self) -> usize {
        lock_shared(&self.shared).textures.len()
    }

    /// 图集边长（像素）。
    pub fn atlas_size(&self) -> u32 {
        lock_shared(&self.shared).renderer.atlas_size()
    }

    /// 注册一张图像纹理，返回写进 `props::IMAGE_TEXTURE` 的槽位句柄。
    ///
    /// D3D12 里「SRV」不是对象而是**描述符堆里的槽位**，所以这里返回 `u32` 而不是
    /// 一个 `IShaderResourceView` ptr_of。
    ///
    /// # 为什么走一条**独立**的提交
    ///
    /// 上传要发屏障 + `CopyTextureRegion`，那是命令列表上的一条命令。而注册通常发生在
    /// 建窗那一帧的中途（此刻帧还没 `begin_frame`）。所以这里用
    /// [`Gpu::submit_one_off`] 单独开一条命令列表、提交、**并等 GPU 做完** ——
    /// 等这一手是必须的：纹理与暂存缓冲随即就要被下一帧的 draw 读到，
    /// 而「下一帧」不会自动等这一条命令。
    pub fn register_texture(
        &mut self,
        name: &str,
        pixels: &[u8],
        w: u32,
        h: u32,
    ) -> Result<u32, String> {
        let mut shared_guard = lock_shared(&self.shared);
        let Shared {
            gpu,
            renderer,
            textures,
            ..
        } = &mut *shared_guard;
        let mut slot = None;
        if let Err(e) = gpu.submit_one_off(|list| {
            slot = Some(renderer.upload_texture(list, pixels, w, h));
        }) {
            return Err(format!("提交纹理上传命令失败: {e}"));
        }
        let slot = slot.expect("submit_one_off 一定跑一遍回调");
        let slot = slot.map_err(|e| format!("上传纹理 {name}: {e}"))?;
        textures.insert(name.to_string(), slot);
        Ok(slot)
    }

    /// 窗口（供宿主设置标题等）。
    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    /// 客户区像素尺寸。
    ///
    /// 最小化时 w/h 可能是 0，而布局拿 0 去算会让所有元素的尺寸塌成 0、恢复后第一帧
    /// 全错。所以这里**夹到至少 1** —— 那不是「修正尺寸」，是「不让布局看到 0」。
    pub fn client_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width.max(1), s.height.max(1))
    }

    /// 客户区**有效像素**尺寸 = 物理 / scale。
    ///
    /// # 为什么单独一个方法，而不是把 `client_size` 改掉
    ///
    /// 两者用途不同，混在一起会一次改两件事：
    /// - `client_size()`（物理）是**布局视口**，改它会让全部视觉尺寸一起变
    ///   （1.25 缩放下整体小 25%）—— 那是独立的一轮，见
    ///   `docs/developer/responsive-and-touch.md` §5 的记账；
    /// - 本方法（有效）只服务**断点判定**。官方口径写得很清楚：
    ///   `AdaptiveTrigger.MinWindowWidth` = "the minimum window width
    ///   (**in effective pixels**)"。本机 scale 1.25 下，1400 物理 = 1120 有效，
    ///   而 800 物理 = 640 有效，正好压在「小/中」的边界上 ——
    ///   **按物理判会整体错一档**。
    pub fn effective_size(&self) -> (f32, f32) {
        let (w, h) = self.client_size();
        let scale = self.window.scale_factor();
        // scale 为 0 / 非正 / NaN 时退回 1.0：除零会得到 inf，而 inf 会让
        // 每个断点判定都走向同一支，那种"静默退化"比"明显错了"更难发现。
        let scale = if scale.is_finite() && scale > 0.0 {
            scale as f32
        } else {
            1.0
        };
        (w as f32 / scale, h as f32 / scale)
    }

    /// 当前 hover 的节点 key。
    pub fn hovered(&self) -> Option<&ElementKey> {
        self.router.hovered()
    }

    /// 按应用请求设置窗口外观（暗色模式 / 圆角 / 标题栏与边框颜色）。
    ///
    /// # 为什么要宿主转发一层，而不是让应用直接调 `zephyra-platform-win`
    ///
    /// 三个理由，都不是"省一行代码"：
    ///
    /// 1. **句柄转换是本 crate 的事**：应用拿到的是 `&Window`，`HWND` 只有
    ///    拿到 `window.hwnd()` 的那一层才该碰；
    /// 2. **能力降级要在这里收敛**：平台不支持时不该让应用拿到一个"我以为
    ///    设上了"的结果 —— [`zephyra_platform::ApplyOutcome`] 逐项如实返回；
    /// 3. **可移植性**：应用调的是宿主方法，Linux 上宿主内部换成别的实现，
    ///    应用代码一个字不改。
    ///
    /// 返回值即 [`zephyra_platform::ApplyOutcome`]，逐项说明"哪些真的生效了"。
    pub fn apply_window_chrome(
        &self,
        chrome: zephyra_platform::WindowChrome,
    ) -> zephyra_platform::ApplyOutcome {
        // 复用本文件已有的句柄提取（`raw_window_handle` 路径），不重复造轮子。
        // 拿不到（理论上只在非 Windows 上）⇒ 0 号句柄 ⇒ 平台层静默跳过。
        let raw = hwnd_of(self.window.as_ref()).unwrap_or(0) as usize;
        zephyra_platform_win::current()
            .apply_window_chrome(zephyra_platform::WindowHandle::from_raw(raw), chrome)
    }

    /// 拉取调试层消息（`ZEPHYRA_D3D12_DEBUG=1` 时才有内容）。
    pub fn drain_debug(&self) {
        let shared = lock_shared(&self.shared);
        shared.gpu.drain_debug();
    }

    /// 一帧什么都没做时返回的东西。
    ///
    /// `redrawn: false` 是关键：`about_to_wait` 见到它就**不 present** —— 而设备
    /// 坏着或链还没 resize 成功时，present 正是最该省掉的那一步。
    fn empty_frame(&self) -> Frame {
        Frame {
            diff: zephyra_core::diff::DiffStats::default(),
            render: zephyra_fx::quad::RenderReport::default(),
            hovered: None,
            damage: zephyra_core::frame::Damage::Full,
            redrawn: false,
        }
    }

    /// 这一帧到底该不该画。**纯函数** —— 于是能在没有 GPU 的机器上门禁。
    ///
    /// # 为什么必须是纯函数
    /// 跑一帧：布局 → `DisplayList` → 顶点展开 → 记录 → 提交。
    ///
    /// **不**包含 `present` —— 那由 [`Self::present`] 单独做，因为 resize 必须在
    /// 绘制之前处理（见 `sync_swapchain`），而呈现必须在绘制之后。
    pub fn frame(&mut self) -> Frame {
        self.frame_inner(None).0
    }

    /// 跑一帧；`capture` 非空时顺便读回像素。
    ///
    /// ⚠ `capture` **不**改变这一帧的脏区（除非 `opts.repaint`）：截图是观测
    /// 手段，整帧重画会把「上一帧没被清掉的像素」擦掉 —— 那样残影类缺陷就
    /// **永远截不出来**。见 `zephyra_fx::bridge::ShotOptions`。
    fn frame_inner(
        &mut self,
        capture: Option<zephyra_fx::bridge::ShotOptions>,
    ) -> (Frame, Option<Vec<u8>>) {
        let frame_started = self.profiler.is_running().then(std::time::Instant::now);
        // 整帧（resize + 布局 + 记录 + 提交）都在共享栈的锁内完成。
        //
        // 锁在这里而不是在 `render_pass`：布局要读 `shared.engine`（字形图集），
        // 而设备恢复要替换整份 `shared`。两处各自加锁会在 `frame_inner` 调
        // `render_pass` 时自锁。
        let shared_arc = self.shared.clone();
        let mut shared_guard = lock_shared(&shared_arc);
        let shared = &mut *shared_guard;
        // ⚠ resize 必须在**布局与绘制之前**，见 [`Self::sync_swapchain`] 的说明。
        // 顺序反了会留下一个「从未被写过的新后缓冲」，而且它可能永远等不到下一帧。
        //
        // ⚠ 设备已死时**整个函数直接返回**：不进布局、不画、不 present。
        //
        // 早先是照样往下走，于是每帧产出三连错（resize 重试 / 后缓冲下标越界 /
        // present 失败），而 DComp 视角下是「一个 Present 不出来的 visual」——
        // 症状就是「窗口一直闪 + 错误刷屏」。
        // 设备如果在**别的窗口**那边被重建过，本窗口的交换链与目标都已失效 ——
        // 先按新设备重建本链，再走 resize。
        if self.seen_generation != shared.generation {
            if let Err(e) = self.rebuild_chain(shared) {
                eprintln!("[zephyra-d3d12] 重建本窗口交换链失败（{e}）");
                return (self.empty_frame(), None);
            }
        }
        self.swap_state = self.sync_swapchain(shared);
        let dead = self.chain.device_dead();
        if !should_draw_frame(self.swap_state, dead) {
            // ⚠ 只有**设备真的死了**才报「设备不可用」。resize 暂时失败时设备还活着，
            // 那条路径唯一该响的是 `sync_swapchain` 里的 trace 行（它每秒可能来几十
            // 次，不该按错误报）。
            if dead {
                self.report_device_dead_once(shared);
                // 设备重置后**按间隔重试**：驱动通常几百毫秒内恢复，重试成功就接着画。
                //
                // ⚠ 没有这一步的话，设备一旦在空闲时挂掉，宿主就**永久**停止绘制 ——
                // 用户看到的正是「点哪都没反应」。注意那不是点击分派坏了：
                // `window_event` 仍无条件 `sync_input()`，点击一直在处理，只是画面
                // 不再更新，于是「看着没反应」。
                self.try_recover_device(shared);
            }
            return (self.empty_frame(), None);
        }
        drain_debug_shared(shared);

        let update_started = frame_started.map(|_| std::time::Instant::now());
        let theme = self.theme;
        // 作用域要短：`ctx` 借着 `self.engine`，而下面还要可变借用 `self`。
        //
        // 在作用域**内**就把 `FrameOutcome` 拆成 `Copy` 标量：`Updated` 借着一份
        // `&DisplayList`，带出作用域会让借用活到函数末尾，与下面的
        // `&mut self` 冲突。列表留到 `render_pass` 里用 `self.cache.list()` 现取。
        let (stats, damage, content_redrawn) = {
            // 挂上**内置**图标集 —— 于是矢量图标在这条路径上也走完整的
            // 「图标名 → 展平 → 三角化 → `Command::Path`」链条。
            //
            // 用内置表而不是让宿主登记自己的：内置表是编译期常量（`assets/*.svg`
            // 经 `include_str!`），零运行时准备成本，且 `OnceLock` 只解析一次。
            // 应用自己的图标库应当自己建一个 `IconSet` 再挂进来 —— 那是宿主的事，
            // 与纹理句柄同一层。
            let ctx = Context::with_engine(theme, &shared.engine)
                .with_icons(zephyra_core::icon::builtin());
            let element = self.view.view(&ctx);
            let size = self.client_size();

            // ⚠️ 图集是「内容变了」的一个**独立**来源，而 core 的判据只看节点脏标记，
            // 看不见纹理侧的变化。所以图集一改就强制重绘。
            //
            // 真实触发路径：图集满时的**淘汰重排**（`make_room`）发生在 layout 的
            // measure 阶段，那时可能一个节点都没 paint-dirty ⇒ core 返回
            // `Unchanged` ⇒ 本帧不回放 ⇒ GPU 抱着**重排前**的纹理。症状是
            // 「文字忽然整片消失，或每个字跑到别的位置上」，且**只在图集填满后出现**。
            //
            // 判据是 `atlas_revision()` 而不是「已缓存字形数」：加了 LRU 淘汰之后
            // 「数量不变 ⇒ 内容不变」不成立 —— 淘汰一个再放进一个，数量不变而纹理已
            // 整体重排。
            let revision_now = shared.engine.borrow().atlas_revision();
            if revision_now != shared.atlas_revision_uploaded {
                self.cache.invalidate();
            }

            // 「这一帧画什么」由核心里唯一的那份判据决定，见
            // `zephyra_core::frame::FrameCache`。
            match self
                .cache
                .frame(&element, ctx, (size.0 as f32, size.1 as f32))
            {
                FrameOutcome::Unchanged => (DiffStats::default(), Damage::Full, false),
                FrameOutcome::Updated { stats, damage, .. } => (stats, damage, true),
            }
        };
        let update_us = update_started
            .map(|started| started.elapsed().as_micros() as u64)
            .unwrap_or_default();

        // ⚠ `Unchanged` 时**连 present 都不能做**：本后端是 flip model
        // （`FLIP_DISCARD`，DComp 路径 `FLIP_SEQUENTIAL`），`Present` 之后后缓冲内容
        // 未定义。不画就 present 会把未定义内容推上屏。
        let redrawn = content_redrawn || capture.is_some();
        if redrawn {
            self.rendered_frames += 1;
        }
        // 截图**默认沿用本帧脏区**：脏区外保留上一帧的像素（MSAA 目标纹理逐帧
        // 存活），于是残影（上一帧没被清掉的东西）**截得出来**。
        // 只有显式 `--repaint` 才整帧 —— 那时要的是一张干净参考图。
        let damage = match capture {
            Some(opts) if opts.repaint => Damage::Full,
            _ => damage,
        };
        let render_started = frame_started.map(|_| std::time::Instant::now());
        let always_started = std::time::Instant::now();
        let (report, captured) = if redrawn {
            self.render_pass(shared, damage, capture.is_some())
        } else {
            (RenderReport::default(), None)
        };
        let render_submit_us = render_started
            .map(|started| started.elapsed().as_micros() as u64)
            .unwrap_or_default();
        self.render_submit_us = always_started.elapsed().as_micros() as u64;

        if trace_enabled() {
            self.frames += 1;
            // 视口一变就打一行，不按帧号节流：resize 类缺陷的判据全在这一行里。
            //
            // `redrawn == false` 且视口刚变过 ⇒ **core 认为不用重画**（问题在
            // 判据，不在绘制）；`redrawn == true` 而屏幕没变 ⇒ **画了但没显示**
            // （问题在交换链 / DComp）。这两类症状看起来一模一样（都是"窗口变大了
            // 但内容没跟着变"），而排查方向完全相反 —— 所以必须分开打。
            let vp = {
                let s = self.client_size();
                (s.0 as f32, s.1 as f32)
            };
            if self.traced_viewport != Some(vp) {
                if let Some(old) = self.traced_viewport {
                    eprintln!(
                        "[trace] 第 {} 帧视口 {old:?} → {vp:?}：redrawn={redrawn} damage={damage:?}",
                        self.frames,
                    );
                }
                self.traced_viewport = Some(vp);
            }
            // 首帧与之后每 60 帧：稳态下这些数恒定，一变就说明布局或图集在漂。
            if self.frames == 1 || self.frames.is_multiple_of(60) {
                // `kind` 直方图：片元着色器**只**按 `kind` 分支，所以「矩形出来了、
                // 字形没出来」这件事一定能在这一行里看到（是 `GLYPH` 一个都没有，
                // 还是有一堆但采样不出来）。这两者的排查方向完全不同。
                let mut kinds = [0usize; 3];
                for c in self.list_kind_histogram(&shared.textures) {
                    kinds[c] += 1;
                }
                eprintln!(
                    "[trace] 第 {} 帧：四边形 {}（跳过 {}）、kind 矩形/字形/图像 {}/{}/{}、\
                     图像批 {}、图集字形 {}、顶点 {} 字节、窗口 {:?}、交换链 {:?}",
                    self.frames,
                    report.quads,
                    report.skipped,
                    kinds[0],
                    kinds[1],
                    kinds[2],
                    report.image_batches,
                    shared.engine.borrow().atlas_stats().glyphs,
                    report.vertex_bytes,
                    self.client_size(),
                    self.chain.size(),
                );
            }
        }

        // 3. 清事件标志。脏标记由 `FrameCache` 内部清（它知道什么时候该清）。
        self.dirty = false;

        // 4. 无障碍：发布快照 + 施加 UIA 侧排队的动作。
        //
        // 必须在 `self.dirty = false` **之后**：它要标脏，而那行会清掉标脏。
        self.pump_uia();

        // 桥的 `stats` 答案在这里刷新（与 D3D11 侧同源同帧）。
        //
        // ⚠ `damage` 也在里面：残影类缺陷的判据就是「这一帧是整帧还是局部」
        // （脏区外保留上一帧的像素 ⇒ 收窄成局部矩形就会留残像）。这一项原先
        // 根本不在统计里，排查的人只能改宿主源码加临时日志重编一遍才看得到。
        //
        // ⚠ **跳过的帧不覆盖「上一次画出来的东西」**
        // （`stats_after_skipped` 里有这条规则的完整说明）。
        //
        // 症状实测：`zephyra perf watch` 在静止的应用上那一列恒为 0，
        // 而画面上明明有 73 个节点的界面。
        // 「输入 → 帧就绪」的耗时。
        //
        // ⚠ 测的是**应用这一段**（事件循环调度 + 命中测试 + diff + 布局 +
        // 绘制 + 提交），**不包含** DWM 的合成时间 —— 那一段是系统的。
        self.input_latency_us = self.input_at.map(|at| at.elapsed().as_micros() as u64);
        self.input_at = None;

        let new_stats = zephyra_fx::bridge::FrameStats {
            quads: report.quads,
            skipped: report.skipped,
            dirtied: stats.dirtied,
            needs_layout: stats.needs_layout,
            redrawn,
            hovered: self.router.hovered().map(|k| k.to_string()),
            damage: Some(damage),
            // 分段耗时：core 侧量的（对账 / 样式 / 脏区扫描 / 布局 / 绘制）。
            // 「这一帧慢在哪」从此是**读出来的**，不用改代码加临时日志。
            stages: Some(self.cache.timings()),
            // 已渲染帧数：客户端靠它算 fps（两次采样相减 / 时间差）。
            frames: self.rendered_frames,
            // 临时探针：上一帧 `begin_frame` 里 wait_gpu() 等了多久。
            // 「输入 → 帧就绪」的耗时。
            input_latency_us: self.input_latency_us,
            render_submit_us: Some(self.render_submit_us),
            // 离屏趟数 = 这一帧 GPU 干了多少活的**主指标**（一趟 ≈ 四趟全屏光栅化）。
            offscreen_passes: report.offscreen_passes,
            backdrop_blur_passes: report.backdrop_blur_passes,
            lens_passes: report.lens_passes,
            refraction_passes: report.refraction_passes,
        };
        self.last_stats = if redrawn {
            new_stats
        } else {
            zephyra_fx::bridge::stats_after_skipped(&self.last_stats, &new_stats)
        };
        // ⚠ **只记真的重画了的帧**。
        //
        // 不记「跑过」的帧：事件循环在没有重画时也会跑（桥连着时更高频），
        // 而那些 tick 会把 fps 撑成一个假数字 —— 实测报告「2500 fps」而应用
        // 其实只画了 **9 帧**。性能采样要回答的是「**画一帧**要多久」，
        // 不是「空转一圈要多久」。
        if redrawn {
            if let Some(started) = frame_started {
                self.profiler.record(zephyra_fx::bridge::FrameTiming {
                    update_us,
                    render_submit_us,
                    total_us: started.elapsed().as_micros() as u64,
                    // core 侧的分段：于是性能报告能说清「这一帧的 300 µs 花在哪」。
                    stages: Some(self.cache.timings()),
                });
            }
        }

        (
            Frame {
                diff: stats,
                render: report,
                hovered: self.router.hovered().map(|k| k.to_string()),
                damage,
                redrawn,
            },
            captured,
        )
    }

    /// 一帧的 GPU 部分。
    ///
    /// # 图集上传与绘制**必须同帧**
    ///
    /// 图集纹理是每帧重建的（`record_upload_atlas` 内部建暂存 + 拷贝 + 销毁），所以
    /// 「先上传、等一帧、再画」会在中间那帧读到**已销毁**的纹理资源。
    ///
    /// 因此这里把两件事记进**同一条命令列表**：要么都成，要么都不成。
    fn render_pass(
        &mut self,
        shared: &mut Shared,
        damage: Damage,
        capture: bool,
    ) -> (RenderReport, Option<Vec<u8>>) {
        // # 「只重画脏区」的前提：脏区之外的上一帧画面**还在渲染目标里**
        //
        // flip model（`CreateSwapChainForComposition` 恒为 FLIP_SEQUENTIAL）的
        // 后缓冲在 `Present` 之后**内容未定义** —— 它是会被丢弃的临时目标，
        // 不是"上一帧那张图"。
        //
        // 分两种情况：
        //
        // - **MSAA 开**：渲染目标是**自有的**多重采样纹理（`chain.msaa_rtv()`），
        //   逐帧存活、不被系统丢弃；帧末整体 resolve 进后缓冲。所以脏区之外的
        //   上一帧内容确实还在 —— 只清脏区、只画脏区是对的。
        // - **MSAA 关**：渲染目标**就是**那个会被丢弃的后缓冲 ⇒ 脏区之外是
        //   未定义内容。此时按脏区绘制等于把未定义像素推上屏（实测：整屏变底色，
        //   只有脏区那一小块是对的）。所以这里**强制整帧**。
        //
        // 代价是诚实的：MSAA 关时每个交互帧都整帧重绘。取舍写在明处 ——
        // 画面错与画面慢之间，这个仓选画面。
        let damage = if self.chain.samples() > 1 {
            damage
        } else {
            Damage::Full
        };
        if let Err(e) = shared.gpu.begin_frame() {
            eprintln!("[zephyra-d3d12] 开帧失败: {e}");
            drain_debug_shared(shared);
            return (RenderReport::default(), None);
        }

        // 图集：只有**位图内容**变了才传。判据是 `atlas_revision()`，不是字形数 ——
        // 淘汰一个再进一个时字形数不变而纹理已整体重排，用数量当判据会让 GPU 一直
        // 抱着旧图集（画面上字全跑到别的位置）。每帧传 4MB 又会让稳态多几毫秒。
        let revision_now = shared.engine.borrow().atlas_revision();
        if revision_now != shared.atlas_revision_uploaded {
            if let Err(e) = shared
                .renderer
                .record_upload_atlas(shared.gpu.list(), shared.engine.borrow().atlas().pixels())
            {
                eprintln!("[zephyra-d3d12] 图集上传失败（{e}），本帧文字可能不显示");
            }
            shared.atlas_revision_uploaded = revision_now;
        }

        // 后缓冲在每帧开头都是 `PRESENT`。不转就没有渲染目标 —— 且**不报任何错**。
        //
        // MSAA 开启时它**仍然要转**：它是 resolve 的目标，而 D3D12 不为 resolve
        // 自动转换状态。所以这一段与 MSAA 无关，始终执行。
        if let Err(e) = self.chain.barrier_to_render_target(shared.gpu.list()) {
            eprintln!("[zephyra-d3d12] 后缓冲转 RENDER_TARGET 失败: {e}");
            return (RenderReport::default(), None);
        }
        // ⚠ MSAA 开启时渲染目标是**多重采样纹理**，不是后缓冲。
        //
        // PSO 的 `SampleDesc.Count` 与 RTV 的必须一致，所以这一步不能"看情况"：
        // `samples` 决定用哪一个，MSAA 关闭时 `msaa_rtv()` 明确报错而不是回退成
        // 后缓冲 —— 回退会让"采样数不一致"这个真错误变成"画面全黑且零 HRESULT"。
        let rtv = if self.chain.samples() > 1 {
            match self.chain.msaa_rtv() {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("[zephyra-d3d12] 取不到 MSAA 的 RTV: {e}");
                    return (RenderReport::default(), None);
                }
            }
        } else {
            match self.chain.current_rtv() {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("[zephyra-d3d12] 取不到后缓冲的 RTV: {e}");
                    return (RenderReport::default(), None);
                }
            }
        };
        let clear = clear_color(self.transparent, self.theme.surface);
        // 折射 / 可分离混合要读「本帧**已经画在目标上**的内容」，必须把目标当**资源**
        // 传进去。取失败时传 `None`：那 14 个模式会退回 `SrcOver`，画面仍然正确只是少了效果。
        //
        // ⚠⚠ 源**不是**后缓冲，而是「本帧实际画进去的那张」——MSAA 开启时那是多重
        // 采样纹理，后缓冲要等帧末 `resolve_msaa` 之后才有内容。
        //
        // 拿后缓冲当源的症状：折射玻璃里残留**上一页**的控件残影（实测：gallery
        // 玻璃页导航回来后，两块玻璃里浮着状态页的滑块/开关，而玻璃外一切正常）。
        // 因为绘制中途读到的后缓冲是**上一帧** resolve 后的旧画面。
        // `ZEPHYRA_MSAA=0` 时绘制目标本就是后缓冲，两者合一，故该缺陷只在 MSAA 开时出现。
        let rt = self
            .chain
            .msaa_tex()
            .cloned()
            .or_else(|| self.chain.back_buffer().ok());
        let size = self.client_size();
        let report = match shared.renderer.record_with(
            &mut self.targets,
            RenderCtx {
                device: &shared.gpu.device,
                list: shared.gpu.list(),
                rtv,
                display_list: self.cache.list(),
                damage,
                size,
                clear,
                textures: &shared.textures,
                rt: rt.as_ref(),
            },
        ) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[zephyra-d3d12] 记录绘制命令失败: {e}");
                RenderReport::default()
            }
        };
        // ⚠ MSAA 解析必须在 `barrier_to_present` **之前**、且必须画完之后。
        //
        // 画进 MSAA 纹理的内容不会出现在后缓冲上 —— 漏掉这步的症状是「画面卡在
        // 旧内容」或「全黑」，且零 HRESULT 错误。顺序也不能反：先转回 PRESENT
        // 再 resolve 是非法的（resolve 目标必须在 RENDER_TARGET）。
        if self.chain.samples() > 1 {
            if let Err(e) = self.chain.resolve_msaa(shared.gpu.list()) {
                eprintln!("[zephyra-d3d12] MSAA 解析失败: {e}");
            }
        }
        let staging = if capture {
            match self
                .chain
                .record_readback(&shared.gpu.device, shared.gpu.list())
            {
                Ok(staging) => Some(staging),
                Err(e) => {
                    eprintln!("[zephyra-d3d12] 截图读回命令记录失败: {e}");
                    None
                }
            }
        } else {
            None
        };
        if let Err(e) = self.chain.barrier_to_present(shared.gpu.list()) {
            eprintln!("[zephyra-d3d12] 后缓冲转回 PRESENT 失败: {e}");
        }
        if let Err(e) = shared.gpu.submit() {
            eprintln!("[zephyra-d3d12] 提交失败: {e}");
            return (report, None);
        }
        let captured = if let Some(staging) = staging {
            if let Err(e) = shared.gpu.wait_gpu() {
                eprintln!("[zephyra-d3d12] 等待截图读回失败: {e}");
                None
            } else {
                let size = self.chain.content_size();
                let pixels = crate::Offscreen::map(&staging, size);
                if pixels.is_empty() {
                    eprintln!("[zephyra-d3d12] 截图读回缓冲为空");
                    None
                } else {
                    Some(zephyra_fx::pixels::bmp_bytes(
                        &zephyra_fx::PixelRead {
                            data: pixels,
                            order: zephyra_fx::pixels::ChannelOrder::Rgba,
                            row_pitch: size.0 * 4,
                            width: size.0,
                            height: size.1,
                            declared_width: size.0,
                            declared_height: size.1,
                        },
                        true,
                    ))
                }
            }
        } else {
            None
        };
        (report, captured)
    }

    /// 让拖动窗口边时**逐帧跟随**（而不是拖完才更新一次）。
    ///
    /// # 问题出在哪（不是本 crate 的 bug）
    ///
    /// 拖边时 Windows 在 `DefWindowProc(WM_NCLBUTTONDOWN)` 里跑一个**模态循环**。
    /// 该循环**会**把一串 `WM_SIZE` 派发到窗口过程，但 winit 的主循环此刻被
    /// 阻塞在 `DefWindowProc` 里面 —— winit 自己的 `WndProc` 收到 `WM_SIZE` 后只做
    /// `userdata.send_event(Resized(..))`，**入队**，而主循环取不出来
    /// （winit-0.30.13 `platform_impl/windows/event_loop.rs:1397-1418`）。
    /// 于是 `Self::about_to_wait` 那种「脏了就画」直到拖动结束才跑第一次。
    ///
    /// # 修法（照 XAML 的形状，不照直觉的形状）
    ///
    /// 直觉方案是「`WM_ENTERSIZEMOVE` 起个 16 ms 定时器，`WM_TIMER` 里重绘」。
    /// **不这么做**，两条理由都来自 `E:\microsoft-ui-xaml`：
    ///
    /// - XAML 侧**零** resize 定时器，所以那条路没有官方先例；
    /// - XAML 对「渲染期间意外泵消息」是 fail-fast 的
    ///   （`dxaml/xcp/core/dll/xcpcore.cpp:6338-6345`，注释原文就是防止渲染循环里
    ///   不小心泵消息导致又一次 paint 进来）。定时器方案恰恰是在拖动期间反复进
    ///   渲染，等于主动制造它最怕的重入。
    ///
    /// XAML 真正**同步**处理尺寸的地方是窗口过程里的链条：`WM_SIZE` →
    /// `CWindowChrome::UpdateContainerSize` → `UpdateBridgeWindowSizePosition` →
    /// `::SetWindowPos`（`CWindowChrome.cpp:338-343, 388`）。
    /// **本方法就是那个形状**：子类化窗口过程，在 `WM_SIZE` 里同步把这一帧做完。
    ///
    /// # 为什么不需要另起线程
    ///
    /// D3D12 命令列表与交换链都是**创建线程所有**的，所以渲染必须在 UI 线程。
    /// 而模态循环期间 UI 线程**正是空闲的**（它只是卡在 `DefWindowProc` 里），
    /// 换句话说这恰好是唯一能安全画的时候。
    ///
    /// # SAFETY（这里有一处 `unsafe`，不藏）
    ///
    /// 回调必须是 `'static`，拿不到 `&mut Self`。用裸指针的前提有三条：
    ///
    /// 1. **地址稳定**。宿主必须在 `Box`/栈上原地存活，且装钩子之后**不许**再挪。
    ///    调用方（`Bootstrap::resumed` 里的 `Option<HostWindow>` 内联存储）满足。
    /// 2. **不并发**。回调只在模态循环派发 `WM_SIZE` 时触发，而那时 winit 的主循环
    ///    被阻塞着，不存在活的 `&mut HostWindow`。平台层的 `IN_HOOK` 再挡一层重入。
    /// 3. **不重入**。回调里不泵消息、不调 `SetWindowPos`、不调 `DefWindowProc`。
    ///
    /// 这三条写在 `zephyra-platform-win/src/live_resize.rs` 的模块文档里也有，
    /// 两处必须一致 —— 改一处就要改另一处。
    pub fn enable_live_resize(&mut self) -> Result<(), zephyra_platform_win::LiveResizeError>
    where
        V: 'static,
    {
        let Some(hwnd) = hwnd_of(self.window.as_ref()) else {
            // 非 Win32 平台没有 HWND 可子类化。不算错：这个后端本来就是 Windows 专属。
            return Ok(());
        };
        let me = HostPtr(self as *mut Self);
        zephyra_platform_win::set_render_hook(
            hwnd,
            Arc::new(move || {
                // ⚠ 这一行不是废代码。Rust 2021 的 `move` 闭包**按字段**捕获：
                // 下面要用 `me.0`，编译器就会只把那个裸指针字段搬进闭包 ——
                // 于是 `HostPtr` 上那两条 `unsafe impl Send/Sync` **根本没被用上**，
                // 整个包装白写，编译直接报 E0277。
                // 把整个 `me` 重新绑一次，捕获的就变成 `me` 本身。
                let me = &me;
                // SAFETY：见上面「SAFETY」三条。这三段是同一个不变量的三个侧面。
                let host = unsafe { &mut *me.0 };
                host.dirty = false;
                let f = host.frame();
                if f.redrawn {
                    host.present();
                }
                // 返回 `true` = 这一帧确实画了。没画也无所谓 —— 调用方只看
                // 「有没有跳过」，那由 `IN_HOOK` 决定，不是这里。
                true
            }),
        );
        zephyra_platform_win::install_live_resize(hwnd)
    }

    /// 装上无障碍（Windows UI Automation）。
    ///
    /// # 装上之后会发生什么
    ///
    /// - 屏幕阅读器（Narrator）与自动化脚本能**看见**这棵界面树：复选框是
    ///   CheckBox 角色、带名字与位置，开关状态是 ToggleState；
    /// - 它们能**操作**控件：Toggle 图案会真的把复选框勾上，Invoke 图案会真的
    ///   按下按钮 —— 走的是**与键盘完全同构**的路径（见 `zephyra_uia::Bridge`）。
    ///
    /// 应用**不需要**为无障碍写任何代码：无障碍树是从画出来的那棵树派生的。
    ///
    /// # 装序
    ///
    /// 必须在 [`Self::enable_live_resize`] **之后**装 —— 两者都是
    /// `SetWindowLongPtrW(GWLP_WNDPROC)`，消息链才是
    /// `UIA → live_resize → winit`。
    ///
    /// # 失败怎么办
    ///
    /// 失败**不**是致命错误：无障碍是附加能力，窗口该开还得开。调用方应当
    /// 打印一条警告后继续跑。
    pub fn enable_uia(&mut self) -> Result<(), zephyra_uia::Error> {
        let Some(hwnd) = hwnd_of(self.window.as_ref()) else {
            // 非 Win32 平台没有 HWND 可子类化。不算错：这个后端本来就是 Windows 专属。
            return Ok(());
        };
        zephyra_uia::install(hwnd)?;
        self.uia = Some(zephyra_uia::Bridge::new());
        Ok(())
    }

    /// 有没有待处理的无障碍工作（该跑一帧了）。
    ///
    /// # 为什么需要它
    ///
    /// 本后端是**事件驱动**的：静止时 `about_to_wait` 一帧都不跑。而 UIA 的
    /// 动作是**外部发起**的 —— 屏幕阅读器在自己的线程上喊一声，动作排进队列，
    /// 我们的事件循环并不会因此醒。
    ///
    /// 于是这一帧必须由「队列非空」或「上一帧闩住了还没松」触发，否则那个动作
    /// 会一直躺在队列里，直到碰巧有别的输入把窗口叫醒。
    fn uia_needs_frame(&self) -> bool {
        if self.uia.is_none() {
            return false;
        }
        zephyra_uia::pending_action_count() > 0
            || self.uia.as_ref().is_some_and(|b| b.is_latched())
            || self.uia_redraw
    }

    /// 每帧的无障碍维护：发布快照 + 施加 UIA 侧排队的动作。
    ///
    /// # 调用时机
    ///
    /// 在 `frame_inner` 里、**清掉脏标记之后**调 —— 它要标脏，而那行会清掉标脏。
    ///
    /// # 为什么这里要自己补 `sync_input`
    ///
    /// 本后端的 `sync_input` 只在**事件回调**里调（静止时事件不来 ⇒ 不调）。
    /// 而应用把 click 认成「`pressed` 由 `Some` 变成 `None`」—— 它必须**分别**
    /// 看见「按下」与「松开」两拍。
    ///
    /// 所以这里在 `pressed` 变了之后连补两次 `sync_input`：第一拍报「按下」，
    /// 第二拍（在松开之后）报「松开」⇒ 应用认 click。
    ///
    /// 少了任何一拍，应用就永远等不到 click，而画面上完全看不出异常。
    fn pump_uia(&mut self) {
        if self.uia.is_none() {
            return;
        }
        // 上一帧欠的那一帧画完了 ⇒ 清掉「还欠一帧」。
        self.uia_redraw = false;
        // 1. 发布无障碍树快照。
        //
        // 快照来自**保留树**（画出来的那棵），所以它不会与画面漂移。
        if let Some(root) = self.cache.node() {
            let win = zephyra_uia::window_info(hwnd_of(self.window.as_ref()).unwrap_or(0));
            zephyra_uia::publish(zephyra_uia::rules::build(
                root,
                &win,
                Context::new(self.theme),
            ));
        }
        // 2. 施加 UIA 侧排队的动作。
        let pumped = {
            let Some(root) = self.cache.node_mut() else {
                return;
            };
            let ctx = Context::new(self.theme);
            let Some(bridge) = self.uia.as_mut() else {
                return;
            };
            bridge.pump(root, &mut self.focus, &mut self.activation, ctx)
        };
        if !pumped.pressed_changed {
            return;
        }
        // 3. 「按下」那一拍。
        self.sync_input();
        // 4. 立刻松开 + 「松开」那一拍 ⇒ 应用认 click。
        //
        // 为什么必须当场松开：本后端静止时一帧都不跑，等不到「下一帧」——
        // 而「松开」那一拍是 click 的另一半，缺了它点击就丢了。
        let released = {
            let Some(bridge) = self.uia.as_mut() else {
                return;
            };
            bridge.release_now(&mut self.activation)
        };
        if released {
            self.sync_input();
        }
        // 5. 应用可能改了状态 ⇒ 欠一帧重画。
        self.dirty = true;
        self.uia_redraw = true;
    }

    /// 呈现一次。
    ///
    /// **不含 resize** —— 见 `sync_swapchain`：resize 必须发生在**绘制之前**，
    /// 否则新建的后缓冲要到下一次 `frame()` 才有内容，而那一次可能永远不会来。
    pub fn present(&mut self) {
        let shared_arc = self.shared.clone();
        let mut guard = lock_shared(&shared_arc);
        let shared = &mut *guard;
        // 设备如果在**别的窗口**那边被重建过，本窗口的交换链与目标都已失效 ——
        // 先按新设备重建本链，再 present。
        if self.seen_generation != shared.generation && self.rebuild_chain(shared).is_err() {
            return;
        }
        // 设备已死时**一个 API 都不调**。每次拖动/恢复都重试 present，
        // 在 DComp 看来就是反复推一个无效 visual ⇒ 闪。
        if self.chain.device_dead() {
            self.report_device_dead_once(shared);
            return;
        }
        // 新交换链第一次 present 之前把 GPU 排空。
        //
        // 官方依据见 `Swapchain::take_first_present_flush`（Chromium
        // `ui/gl/swap_chain_presenter.cc:1155`）：不这么做，DComp 会在新链
        // 第 1~2 次 present 之间显示黑帧。拖动期间我们每次 `recreate()` 都会
        // 走到这儿 —— 也就是说这正是「拖动时闪」的另一半成因。
        if self.chain.take_first_present_flush() {
            // 等不到也不阻断 present：这是消闪，不是正确性前提。真在这儿死等，
            // 症状会变成「UI 静止不变」—— 比黑一闪难查得多。
            let _ = shared.gpu.wait_gpu();
        }
        if let Err(e) = self.chain.present(1) {
            eprintln!("[zephyra-d3d12] present 失败: {e}");
            drain_debug_shared(shared);
        }
        // 记下呈现时刻 —— 呈现率封顶用它（见 `last_present_at`）。
        self.last_present_at = Some(std::time::Instant::now());
    }

    /// 把「设备已不可用」只打一次，并给出官方要求的后续动作。
    ///
    /// # 为什么必须只打一次
    ///
    /// 早先是每帧三条错误（resize / 后缓冲越界 / present），拖动期间一秒几十条，
    /// 把真正有用的信息全埋掉 —— 用户看到的就是一片刷屏。而原因码要
    /// `GetDeviceRemovedReason()` 才拿得到，那才是「该采取什么措施」的答案。
    fn report_device_dead_once(&mut self, shared: &Shared) {
        if self.reported_device_dead {
            return;
        }
        self.reported_device_dead = true;
        let code = self.chain.device_dead_reason();
        let (name, advice) = match code {
            // 官方四个常量，按「该怎么办」分组而不是按名字排。
            Some(0x887A_0007) => (
                "DXGI_ERROR_DEVICE_HUNG",
                "GPU 挂了（TDR）。本宿主会按间隔重建整条 GPU 栈并自动恢复",
            ),
            Some(0x887A_0006) => (
                "DXGI_ERROR_DEVICE_RESET",
                "设备正在重建，可恢复 —— 本宿主会按间隔重建整条 GPU 栈并自动恢复",
            ),
            Some(0x887A_0005) => (
                "DXGI_ERROR_DEVICE_REMOVED",
                "设备被移除（通常是驱动更新/显卡复位）。GetDeviceRemovedReason \
                 给出了通用码，真实原因见上面两者之一",
            ),
            // 未知原因码：不能沉默 —— 静默的黑屏比响一声的黑屏难查得多。
            _other => ("未知", "GetDeviceRemovedReason 的返回值不在预期集合里"),
        };
        if let Some(c) = code {
            eprintln!(
                "[zephyra-d3d12] 设备不可用：{name} (0x{c:08X})\n\
                 [zephyra-d3d12] 后续动作：{advice}\n\
                 [zephyra-d3d12] 本帧起暂停绘制与呈现，每 {} 秒尝试重建 GPU 栈（成功即自动恢复）",
                DEVICE_RECOVERY_RETRY_INTERVAL.as_secs()
            );
        } else {
            // 走到这里说明 `device_dead` 是真的但原因码丢了 —— 不该发生，
            // 但也不能沉默：静默的黑屏比响一声的黑屏难查得多。
            eprintln!("[zephyra-d3d12] 设备被标记为不可用，但拿不到原因码");
        }
        drain_debug_shared(shared);
        // ⚠ 上面那几行 `[d3d12 debug] ID=... severity=...: <描述>` 就是**唯一的
        // 一手证据**，请把它们连同本条日志一起贴回来 —— 它们会直接点名是哪个 API
        // 在哪个资源上出的问题（典型：`ID=527` = 屏障的 Before 状态与实际不符）。
        //
        // 如果**一条都没有**，说明调试层没挂上（`ZEPHYRA_D3D12_DEBUG=0` 把它关了，
        // 或者这台机器的 `d3d12.dll` 不支持事后开启）。那就先去掉这个环境变量再跑。
        //
        // ⚠ 为什么这里要专门提一句：`begin_frame` 之前每帧都会 drain 一次并
        // `ClearStoredMessages`，所以**出问题那一帧**的消息是在下一帧才打出来的。
        // 早先设备一判死就整体返回，把那一帧的消息连同上下文一起丢了 ——
        // 于是每次都只能拿到「GPU 挂了」这个结论，拿不到原因。
    }

    /// 重建整条 GPU 栈（设备、交换链、DComp、渲染器）。
    ///
    /// # 为什么恢复只能靠重建
    ///
    /// `DEVICE_REMOVED` 之后设备对象失效，绑在它上面的**一切**都失效：PSO、
    /// 描述符堆、图集纹理、离屏 RT、命令分配器。继续用旧资源只会每帧失败，而且
    /// 失败点各不相同（正是最难查的那种）。所以唯一的恢复是重建。
    ///
    /// # 为什么**先建新的、再换掉旧的**
    ///
    /// 中途任何一步失败都直接 `return Err`，此时 `self` 一个字都没改 —— 旧的链、
    /// 设备、渲染器都还在，下一轮重试的起点是干净的。反过来先拆旧的再建新的，
    /// 一旦中途失败就留下一个「旧的没了、新的没建起来」的空壳，窗口彻底不显示。
    fn rebuild_gpu(&mut self, shared: &mut Shared) -> Result<(), String> {
        let (width, height) = self.client_size();
        let gpu = Gpu::new().map_err(|e| format!("创建 D3D12 设备: {e}"))?;
        // 后缓冲格式必须与 PSO 的 `RTVFormats[0]` 一致（`R8G8B8A8`）—— 见
        // `new_lenient_sized` 里同一段的说明。
        let desc = SwapchainDesc {
            buffer_count: 2,
            format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM,
        };
        let chain = Swapchain::new(
            &gpu,
            (width, height),
            &desc,
            crate::swapchain::msaa_samples(),
        )
        .map_err(|e| format!("创建 composition swapchain: {e}"))?;
        let (compositor, visual) = match hwnd_of(self.window.as_ref()) {
            Some(h) => match attach_visual(HWND(h as *mut _), &gpu, &chain) {
                Ok((c, v)) => (Some(c), Some(v)),
                Err(e) => {
                    eprintln!("[zephyra-d3d12] ⚠ DComp 建树失败（{e}）");
                    (None, None)
                }
            },
            None => {
                eprintln!("[zephyra-d3d12] ⚠ 取不到 HWND，无法建立 DComp target");
                (None, None)
            }
        };
        let renderer = Renderer::new(
            &gpu.device,
            zephyra_core::text::atlas::ATLAS_SIZE,
            chain.samples(),
        )
        .map_err(|e| format!("建根签名 / PSO / 描述符堆: {e}"))?;
        // 本窗口的渲染目标也一起重建 —— 它引用的设备马上要被替换。
        let targets =
            WindowTargets::new(&gpu.device).map_err(|e| format!("重建本窗口渲染目标: {e}"))?;
        // 全部成功才替换。赋值顺序按「谁引用谁」：先放引用方（DComp），再放被引用的
        // （chain），最后是设备与渲染器 —— 与字段声明顺序**相反**，因为这里是部分
        // 重建而不是整体析构。
        shared.gpu = gpu;
        shared.renderer = renderer;
        // 旧纹理属于旧设备，必须清掉（槽位也失效了）。
        shared.textures = TextureTable::new();
        shared.atlas_revision_uploaded = 0;
        // 代次 +1：其它窗口在自己的下一帧看到它，就会重建各自的交换链。
        shared.generation = shared.generation.wrapping_add(1);
        self.compositor = compositor;
        self.visual = visual;
        self.chain = chain;
        self.targets = targets;
        self.seen_generation = shared.generation;
        // 保留树是旧视口的、图集纹理是空的（新渲染器）—— 强制重来。
        self.cache = FrameCache::new();
        self.reported_device_dead = false;
        self.dirty = true;
        Ok(())
    }

    /// 只重建**本窗口**的交换链与渲染目标（共享设备仍有效）。
    ///
    /// 触发条件：别的窗口（或另一个线程上的 `WindowSet`）重建了设备，
    /// `shared.generation` 变了。此时共享的 `Gpu` / `Renderer` 是新的，而本窗口的
    /// `chain` / `targets` 还指着旧设备上的资源。继续用旧链只会每帧失败，且失败点
    /// 各不相同 —— 所以必须重建。
    fn rebuild_chain(&mut self, shared: &Shared) -> Result<(), String> {
        let (width, height) = self.client_size();
        let desc = SwapchainDesc {
            buffer_count: 2,
            format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM,
        };
        let chain = Swapchain::new(
            &shared.gpu,
            (width, height),
            &desc,
            crate::swapchain::msaa_samples(),
        )
        .map_err(|e| format!("重建 composition swapchain: {e}"))?;
        let (compositor, visual) = match hwnd_of(self.window.as_ref()) {
            Some(h) => match attach_visual(HWND(h as *mut _), &shared.gpu, &chain) {
                Ok((c, v)) => (Some(c), Some(v)),
                Err(e) => {
                    eprintln!("[zephyra-d3d12] ⚠ DComp 建树失败（{e}）");
                    (None, None)
                }
            },
            None => (None, None),
        };
        let targets = WindowTargets::new(&shared.gpu.device)
            .map_err(|e| format!("重建本窗口渲染目标: {e}"))?;
        self.compositor = compositor;
        self.visual = visual;
        self.chain = chain;
        self.targets = targets;
        self.seen_generation = shared.generation;
        self.cache = FrameCache::new();
        self.reported_device_dead = false;
        self.dirty = true;
        Ok(())
    }

    /// 设备不可用时按间隔尝试恢复。成功则恢复正常绘制；失败则等下一轮。
    ///
    /// # 为什么必须限流
    ///
    /// 恢复要重建整条 GPU 栈，而拖动期间 `frame()` 每秒被叫几十次 —— 不限流就是
    /// 每秒几十次重建，那比「画面冻结」更糟。
    fn try_recover_device(&mut self, shared: &mut Shared) {
        let now = std::time::Instant::now();
        let due = match self.last_recovery_attempt {
            None => true,
            Some(last) => should_attempt_recovery(now.duration_since(last)),
        };
        if !due {
            return;
        }
        self.last_recovery_attempt = Some(now);
        match self.rebuild_gpu(shared) {
            Ok(()) => {
                eprintln!("[zephyra-d3d12] 设备已恢复：GPU 栈重建成功，继续绘制");
            }
            Err(e) => {
                eprintln!(
                    "[zephyra-d3d12] 设备恢复失败（{e}），{} 秒后重试",
                    DEVICE_RECOVERY_RETRY_INTERVAL.as_secs()
                );
            }
        }
    }

    /// 把交换链对齐到窗口尺寸。**返回尺寸是否变了**（变了就要重排）。
    ///
    /// # 为什么在**每帧开头**做，而不是画完再 resize
    ///
    /// 「先画、再 resize」有两个后果：
    ///
    /// 1. 重建后的后缓冲是**从未被写过的** —— 而画它的那一帧已经过去了。若此后没有
    ///    新的脏（窗口不缩放不移动），就再没有 `frame()`，于是**永久**停在那一块
    ///    未初始化的缓冲上：屏幕上是一块纯白（或纯黑）矩形，外面是窗口背景。
    /// 2. 更要紧：`resize` 要**等 GPU 跑完上一帧**（`Swapchain::resize` 里的
    ///    `gpu.wait_gpu()`，那是 `ResizeBuffers` 的硬要求）。放在画完之后就等于
    ///    刚提交完就立刻同步等栅栏，每帧多一次 GPU 往返。
    ///
    /// 先 resize 再画，那一帧就直接画进新缓冲，没有中间态。
    ///
    /// # 重建后必须重新挂 DComp 视觉树
    ///
    /// visual 上挂的是**旧的** swapchain 对象。新的那个没人引用，合成器不会去取它 ——
    /// 于是「提交了、也没报错，就是不显示」。这与「没提交」在屏幕上完全一样。
    /// 把交换链对齐到窗口尺寸。
    ///
    /// # 返回值的三个含义（宿主据此决定这一帧还画不画）
    ///
    /// - [`SwapState::Synced`]：链已与客户区一致，正常画。
    /// - [`SwapState::Same`]：尺寸本来就没变（鼠标在窗口里动也走这条）。
    /// - [`SwapState::Failed`]：**resize 想做没做成，而设备还活着**。
    ///
    /// # 为什么 `Failed` 必须让宿主跳过本帧
    ///
    /// resize 失败时链还停在**旧尺寸**。此时若照旧"画 + present"，DComp 会把这块
    /// 旧内容拉进**新**窗口 —— 那就是「窗口一直闪 / 内容被拉伸」的直接来源。
    ///
    /// 跳过本帧而不是硬 present，另一头是「不 present 就不闪」：flip model 下
    /// DComp 会继续显示上一次成功 present 的内容，等下一次成功 resize 再更新。
    fn sync_swapchain(&mut self, shared: &mut Shared) -> SwapState {
        let (w, h) = self.client_size();

        // ⚠ 零尺寸 = 窗口被最小化。此时**什么都不做、本帧跳过**。
        //
        // 官方对应做法是把「不可见」当成**不画**的理由：Chromium 的最小化一路走到
        // `DisplayScheduler::ShouldDraw()` 返回 false
        // （`components/viz/service/display/display_scheduler.cc`）。
        // 这里复用 `Failed` 这条「本帧空过」的路径，不新增状态。
        if w == 0 || h == 0 {
            return SwapState::Failed;
        }

        // 内容尺寸没变 ⇒ 链也不用动，直接画。
        if (w, h) == self.chain.content_size() {
            return SwapState::Same;
        }

        // 链**不跟着窗口变**：它只保证「≥ 客户区」。绝大多数尺寸变化在这里什么都不做 ——
        // 只是把视口改成新的客户区，本帧照常按新尺寸排版并绘制，于是**拖动时逐帧跟随**。
        //
        // 只有窗口大到装不下时才重建整条链，且一次给 25% 余量。详见
        // [`Swapchain::resize`] 的文档（含为什么本机不能用 `ResizeBuffers`）。
        let before = self.chain.size();
        if let Err(e) = self.chain.resize(&shared.gpu, (w, h)) {
            self.resize_failures = self.resize_failures.saturating_add(1);
            eprintln!(
                "[zephyra-d3d12] 调整交换链容量到 {w}×{h} 失败（第 {} 次）：{e}",
                self.resize_failures
            );
            return SwapState::Failed;
        }
        if self.chain.was_recreated() {
            // 自述型日志：容量从多少涨到多少。没有它就分不清「窗口还在拖」与
            // 「同一次调整反复重建」。
            eprintln!(
                "[zephyra-d3d12] 后缓冲容量 {before:?} → {:?}（客户区 {w}×{h}），重建交换链",
                self.chain.size()
            );
            if self.rebind_visual() {
                // 必须**在 rebind 成功之后**才放旧链 —— rebind 是
                // SetContent(新链) + Commit()；在那之前 DComp 的 visual 上挂的还是
                // 旧链，早一步释放就是把 use-after-free 送给 DComp。
                //
                // 官方纪律见 Chromium ui/gl/dc_layer_tree.cc:626（只在内容变了时
                // SetContent，拆除时 SetContent(nullptr)，然后 Commit）。
                drop(self.chain.take_superseded());
            } else {
                eprintln!("[zephyra-d3d12] 重新挂载未生效，旧交换链继续保留（DComp 仍在用它）");
            }
        }
        if trace_enabled() {
            eprintln!(
                "[trace] 内容尺寸 → {w}×{h}；后缓冲容量 {:?}；重建 {}",
                self.chain.size(),
                self.chain.was_recreated(),
            );
        }
        SwapState::Synced
    }
    /// 把（可能已被重建的）交换链重新挂到 visual 上并提交。
    ///
    /// # 返回 `true` = DComp 已经在用新链了，调用方**可以**释放旧链
    ///
    /// 这个返回值不是「成没成功」这么简单，它是**释放旧链的许可**。
    /// 返回 `false` 时 visual 上挂的还是旧链 —— 那时把它释放就是把
    /// use-after-free 送给 DComp。调用方必须留住旧链（见
    /// [`Swapchain::take_superseded`]）。
    ///
    /// 两步都必须成功才算：`SetContent` 只是**记下**新内容，`Commit()` 才让它
    /// 生效（官方：DComp 的所有改动都要 Commit 才落地，见
    /// `ui/gl/dc_layer_tree.cc:626`）。Commit 失败 ⇒ 视觉树仍是旧内容。
    fn rebind_visual(&mut self) -> bool {
        let (Some(comp), Some(vis)) = (self.compositor.as_ref(), self.visual.as_ref()) else {
            // 没有合成器/visual（窗口已销毁、或平台层说不支持）—— 无可挂，
            // 但也**没有别人在用旧链**，所以旧链可以安全释放。
            return true;
        };
        if let Err(e) = comp.set_content_swapchain(vis, self.chain.as_swap_chain1()) {
            eprintln!("[zephyra-d3d12] 重新挂 visual 失败: {e}");
            return false;
        }

        if let Err(e) = comp.commit() {
            // ⚠ 这一步失败也算「DComp 还没切过去」—— 见方法文档。
            eprintln!("[zephyra-d3d12] DComp commit 失败: {e}");
            return false;
        }
        true
    }

    /// 把上一帧的顶点流按 `kind` 归类（0 矩形 / 1 字形 / 2 图像），逐个返回。
    ///
    /// 只给 `trace` 用 —— 它要重跑一次 `quad::expand`，不该进稳态路径。
    fn list_kind_histogram(&self, textures: &TextureTable) -> Vec<usize> {
        use zephyra_fx::quad::{self, kind};
        let size = self.chain.size();
        let batch = quad::expand(self.cache.list(), size, |n| textures.contains(n));
        batch
            .solid
            .iter()
            .chain(batch.images.iter())
            .step_by(4)
            .map(|v| {
                if (v.kind - kind::GLYPH).abs() < 0.01 {
                    1
                } else if v.kind > 1.5 {
                    2
                } else {
                    0
                }
            })
            .collect()
    }

    /// 把新输入状态推给视图（每帧开头一次）。
    pub fn sync_input(&mut self) {
        // 长按是**时钟驱动**的：手指不动时没有任何输入事件，所以只能每帧查一次。
        //
        // 官方指南把长按列为静态手势之一（并明确它是「计时交互」的正当用例）。
        // 没有这一行，长按就永远不会触发 —— 而长按恰恰是「手指一动不动」的场景。
        if let Some((kind, target)) = self.router.tick(self.pointer_translator.now_ms()) {
            self.pending_gestures
                .push(zephyra_fx::GestureEvent { kind, target });
            self.dirty = true;
        }
        // 视口、缩放比与帧间隔一起推给视图：断点判定要**有效像素**（除 scale），
        // 列数计算要**布局宽**（本仓即物理像素），动画时钟要**真实 dt**。
        let size = self.client_size();
        let scale = self.window.scale_factor();
        // dt 实测而不是按刷新率拍：官方动画时长全是毫秒（83/167/250/333），
        // 只有真实时间下才成立。掉一帧动画会变慢 —— 但那比"每帧固定推进"好：
        // 后者在任何非标刷新率上都不准。
        let now = std::time::Instant::now();
        let dt_ms = self
            .last_frame_at
            .map(|t| now.duration_since(t).as_secs_f32() * 1000.0)
            .unwrap_or(0.0);
        self.last_frame_at = Some(now);
        // ⚠ 这一批指针事件里有没有**取消**（系统抢走了手势）。
        // 应用要靠它**不**把「按下 → 松开」当成点击 —— 用户的手指还在屏幕上。
        let cancelled = zephyra_fx::has_cancelled(&self.pending_pointer_events);
        let state = InputState {
            hovered: self.router.hovered().cloned(),
            // 键盘激活复用指针的 pressed 通道（指针优先）—— Space 按住时模板出
            // 按下态、抬起时归空，与应用既有的「上帧按着→这帧松了 = 点击」重建逻辑
            // 同构，应用零改动。
            pressed: self
                .router
                .pressed()
                .or_else(|| self.activation.held())
                .cloned(),
            // **全部**按下态（多指各按一个控件时长度 > 1）。
            //
            // ⚠ 键盘激活（Space/Enter）也要在**里面**：`pressed` 那一栏是
            // 「指针的按下态，否则是键盘激活」，这里必须**同款** ——
            // 否则「按 key 找自己」那条路在键盘激活时找不到自己
            // （症状：Space 激活的按钮画不出按下态）。
            pressed_keys: {
                let mut keys: Vec<zephyra_core::id::ElementKey> =
                    self.router.pressed_keys().cloned().collect();
                if keys.is_empty() {
                    if let Some(held) = self.activation.held() {
                        keys.push(held.clone());
                    }
                }
                keys
            },
            // 视觉焦点 = 焦点环可见时才报告（鼠标点击设焦点但不画环，见
            // `FocusState::is_visible`）。激活 / 无障碍另走 `self.focus.focused()`。
            focused: if self.focus.is_visible() {
                self.focus.focused().cloned()
            } else {
                None
            },
            pointers: self.router.pointer_count(),
            text_events: std::mem::take(&mut self.pending_text_events),
            scroll: self.pending_scroll.take(),
            pointer_events: std::mem::take(&mut self.pending_pointer_events),
            cancelled,
            gestures: std::mem::take(&mut self.pending_gestures),
            manipulations: std::mem::take(&mut self.pending_manipulations),
            inertia: self.pending_inertia.take(),
            viewport: (size.0 as f32, size.1 as f32),
            scale: scale as f32,
            dt_ms,
            // 系统语言偏好：每帧问一次很贵（一次 Win32 调用），而它是**启动时的
            // 外部事实**，之后不会变（用户切换系统语言要重启进程才该重新读）。
            // 所以只问一次并缓存 —— 与版本号、系统主题同一手法。
            languages: self.system_languages.clone(),
        };
        self.view.on_input(&state);
    }

    /// 记下「输入刚刚到达」。
    ///
    /// 用来测「输入 → 帧就绪」的延迟（见 [`Self::input_latency_us`]）。
    ///
    /// ⚠ 记的是**到达**的时刻，不是「被处理完」的时刻 —— 用户感知的是
    /// 「我按下去多久屏幕才有反应」，那从到达就开始算了。
    fn mark_input(&mut self) {
        self.input_at = Some(std::time::Instant::now());
    }

    /// 处理指针移动。返回是否产生了路由事件。
    pub fn on_cursor_moved(&mut self, x: f32, y: f32) -> bool {
        self.mark_input();
        self.pointer_pos = (x, y);
        let ev = self.pointer_translator.mouse_move(x, y);
        self.pending_pointer_events.push(zephyra_fx::PointerInput {
            id: ev.id,
            x,
            y,
            phase: zephyra_fx::PointerInputPhase::Move,
            source: ev.source,
        });
        if trace_enabled() {
            eprintln!("[trace] CursorMoved ({x:.1}, {y:.1})");
        }
        let theme = self.theme;
        self.route(|router, node, out| router.on_pointer(node, ev, Context::new(theme), out))
            .0
    }

    /// 处理一次按下。返回是否产生了路由事件。
    pub fn on_pointer_down(&mut self) -> bool {
        self.mark_input();
        self.pointer = Some(0);
        let (x, y) = self.pointer_pos;
        let ev = self.pointer_translator.mouse_button(true, x, y);
        self.pending_pointer_events.push(zephyra_fx::PointerInput {
            id: ev.id,
            x,
            y,
            phase: zephyra_fx::PointerInputPhase::Down,
            source: ev.source,
        });
        let theme = self.theme;
        self.route(|router, node, out| router.on_pointer(node, ev, Context::new(theme), out))
            .0
    }

    /// 处理一次抬起。返回是否产生了路由事件。
    pub fn on_pointer_up(&mut self) -> bool {
        self.mark_input();
        self.pointer = None;
        let (x, y) = self.pointer_pos;
        let ev = self.pointer_translator.mouse_button(false, x, y);
        self.pending_pointer_events.push(zephyra_fx::PointerInput {
            id: ev.id,
            x,
            y,
            phase: zephyra_fx::PointerInputPhase::Up,
            source: ev.source,
        });
        let theme = self.theme;
        let (changed, out) =
            self.route(|router, node, out| router.on_pointer(node, ev, Context::new(theme), out));
        // 点击设焦点：`RoutedEvent::Click` 是「按下 + 抬起在同一节点」。
        // 焦点设给被点到的那个 key —— 文本框点进去获得键盘焦点。
        // `route` 内部拿 `&Node`，焦点要 `&mut Node`，所以借用结束后在这里做。
        if let Some(target) = out.iter().find_map(|e| match e {
            RoutedEvent::Click(key) => Some(key.clone()),
            _ => None,
        }) {
            if let Some(node) = self.cache.node_mut() {
                if self.focus.focus(node, Some(&target), Context::new(theme)) {
                    self.dirty = true;
                }
            }
        }
        changed
    }

    /// 处理一次**触摸**事件（多点触控的主入口）。
    ///
    /// # 为什么这条路径以前是**空的**
    ///
    /// `winit` 在 Windows 上会 `RegisterTouchWindow` —— 那会**关掉**系统的
    /// 「触摸提升为鼠标」。于是触摸事件既不是触摸也不是鼠标，被整个丢掉：
    /// **触控屏上点不动任何东西**。
    ///
    /// # 触控与鼠标的差别（都要在这一层体现）
    ///
    /// - **多指并存**：每根手指一个 id（`ev.id`）；
    /// - **双状态**：触控**不产生悬停**（由 core 的路由器判）；
    /// - **取消 ≠ 抬起**：系统接管手势时收到的是取消（core 清按下态、
    ///   不提交点击）。
    ///
    /// # 坐标
    ///
    /// winit 给的是**物理像素**（`location`），与本宿主其它坐标同一套。
    pub fn on_touch(&mut self, phase: winit::event::TouchPhase, id: u64, x: f32, y: f32) -> bool {
        self.mark_input();
        let ev = self.pointer_translator.touch(phase, id, x, y);
        let fx_phase = match ev.phase {
            PointerPhase::Down => zephyra_fx::PointerInputPhase::Down,
            PointerPhase::Move => zephyra_fx::PointerInputPhase::Move,
            PointerPhase::Up => zephyra_fx::PointerInputPhase::Up,
            PointerPhase::Cancel => zephyra_fx::PointerInputPhase::Cancel,
        };
        self.pending_pointer_events.push(zephyra_fx::PointerInput {
            id: ev.id,
            x,
            y,
            phase: fx_phase,
            source: ev.source,
        });
        if trace_enabled() {
            eprintln!("[trace] Touch {:?} id={} ({x:.1}, {y:.1})", ev.phase, ev.id);
        }
        // 触控也要更新「指针当前位置」—— IME 插入点用它当代理，
        // 而用户在输入框里点的那一下正是触摸。
        self.pointer_pos = (x, y);
        let theme = self.theme;
        self.route(|router, node, out| router.on_pointer(node, ev, Context::new(theme), out))
            .0
    }

    /// 统一的「跑一次路由器 + 置脏」包装。
    ///
    /// 返回 `(是否变化, 路由结果)`。`out` 交出来让调用方消费（如 Click → 焦点），
    /// 因为 `route` 内部拿的是 `&Node`（`router.on_pointer` 要只读树），而焦点
    /// 需要 `&mut Node` —— 那必须等 route 的借用结束后、由调用方拿 `node_mut` 做。
    fn route<F>(&mut self, f: F) -> (bool, RoutedEvents)
    where
        F: FnOnce(&mut InputRouter, &Node, &mut RoutedEvents),
    {
        let Some(node) = self.cache.node() else {
            return (false, RoutedEvents::new());
        };
        let before = self.router.revision();
        let mut out = RoutedEvents::new();
        f(&mut self.router, node, &mut out);
        let changed = self.router.revision() != before;
        if changed {
            self.dirty = true;
        }
        // 静态手势 / 操作增量收进**待发缓冲** —— 它们是「给应用的增量信息」，
        // 而不是「宿主要据此决定重绘」的东西，所以不进 `revision`。
        //
        // ⚠ `Click` 不在这里：它由调用方消费（设焦点）。
        for ev in out.iter() {
            match ev {
                RoutedEvent::Static(kind, target) => {
                    self.pending_gestures.push(zephyra_fx::GestureEvent {
                        kind: *kind,
                        target: target.clone(),
                    });
                }
                RoutedEvent::Manipulation(delta) => self.pending_manipulations.push(*delta),
                RoutedEvent::ManipulationEnded {
                    velocity_x,
                    velocity_y,
                } => self.pending_inertia = Some((*velocity_x, *velocity_y)),
                _ => {}
            }
        }
        if trace_enabled() {
            // 「点哪儿不好使」的唯一一手证据：坐标 → 处理目标。
            // 画面永远是对的，命中偏了也只有这一行能看出来。
            eprintln!(
                "[trace] ptr_of ({:.0}, {:.0}) → {:?}",
                self.pointer_pos.0,
                self.pointer_pos.1,
                self.router.hovered()
            );
        }
        (changed, out)
    }
}

/// 每帧把关键计数打到 stderr（`ZEPHYRA_D3D12_TRACE=1` 时才开）。
///
/// # 为什么需要它
///
/// 「窗口全白」和「文字不见了」都是**只有画面能证明**的现象，而在没有窗口的测试里
/// 两者都看不见 —— `d3d12_draw` 与 `glyph_draw` 在离屏上都过，窗口却仍是错的。
/// 所以宿主必须能**自己说**出「这一帧展开出多少四边形、图集里有多少字形、窗口与
/// 交换链各是多大」，否则每次都得靠「截屏 → 肉眼 → 猜」。
fn trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("ZEPHYRA_D3D12_TRACE")
            .map(|v| v != "0")
            .unwrap_or(false)
    })
}

/// 建 DComp target / visual 并把交换链挂上去。
fn attach_visual(
    hwnd: HWND,
    gpu: &Gpu,
    chain: &Swapchain,
) -> Result<(DcompCompositor, VisualSlot), String> {
    // DComp 设备要从渲染设备建；本机 D3D12 command queue 路线 `E_NOINTERFACE`，
    // 所以走带 fallback 的那个构造（v2 → v3 → null）。
    let unknown: windows::core::IUnknown = gpu
        .queue
        .cast()
        .map_err(|e| format!("command queue QI 成 IUnknown: {e}"))?;
    let mut comp = DcompCompositor::new_from_rendering_device(hwnd, Some(&unknown))
        .map_err(|e| format!("DCompositionCreateDevice: {e}"))?;
    let visual = comp
        .add_visual_on_top()
        .map_err(|e| format!("AddVisual: {e}"))?;
    comp.set_content_swapchain(&visual, chain.as_swap_chain1())
        .map_err(|e| format!("SetContent(swapchain): {e}"))?;
    comp.commit().map_err(|e| format!("commit: {e}"))?;
    Ok((comp, visual))
}

/// 装进 `'static` 回调里的宿主指针。
///
/// # 为什么需要这个包装
///
/// 裸指针既不是 `Send` 也不是 `Sync`，而 [`HostWindow::enable_live_resize`] 的
/// 回调类型是 `Arc<dyn Fn() -> bool + Send + Sync>`（窗口过程可能被任意线程的
/// 消息派发调用，哪怕本进程只有 UI 线程）。所以必须显式声明一次。
///
/// # SAFETY
///
/// 真正的不变量在 [`HostWindow::enable_live_resize`] 的文档里写了三条
/// （地址稳定 / 不并发 / 不重入）。这里只是把那三条**转成类型系统要求的断言**。
///
/// 这里**不能**用 `Arc<Mutex<..>>` 之类替代：那要求把宿主的可变状态整体搬进
/// `Mutex`，而 `frame()` 要连着 `&mut self` 跑排版 + 录制 + 提交，锁的范围会被
/// 撑到整个帧 —— 而帧里会 `eprintln!`、会访问 `RefCell`、会回调 `View::view`，
/// 加一把可能被别人拿到的锁只会换来死锁而不是安全。
struct HostPtr<V: View>(*mut HostWindow<V>);

// SAFETY：见上。三条不变量由 `enable_live_resize` 的文档与平台层模块文档共同保证；
// 那两处任何一条被改，这条都要重新论证。
unsafe impl<V: View> Send for HostPtr<V> {}
// SAFETY：同上（回调可被并发调用的面由平台层的 `IN_HOOK` 原子位挡住）。
unsafe impl<V: View> Sync for HostPtr<V> {}

/// 从 winit 窗口取裸 HWND。失败返回 `None`（非 Win32 平台）。
fn hwnd_of<W: raw_window_handle::HasWindowHandle>(window: &W) -> Option<isize> {
    use raw_window_handle::RawWindowHandle;
    let handle = window.window_handle().ok()?;
    match RawWindowHandle::from(handle) {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

/// 调试桥的执行面（D3D12）。见 `zephyra_fx::bridge` 的模块文档。
///
/// # 与 D3D11 侧的两处差异
///
/// - **注入同源**：`inject_*` 直接调 [`HostWindow::on_cursor_moved`] /
///   [`HostWindow::on_pointer_down`] / [`HostWindow::on_pointer_up`]——合成
///   与物理事件零分叉（D3D11 侧同一论证）；
/// - **截图时强制渲染并读回**：`screenshot()` 经 `frame_inner(true)` 完整重绘，
///   MSAA resolve 后复制到 readback heap，再由桥返回 BMP 像素；D3D12 的真机闭环
///   已覆盖该路径，避免把旧的「尚未接线」能力说明留在实现旁。
impl<V: View + 'static> zephyra_fx::bridge::BridgeHost for HostWindow<V> {
    fn viewport(&self) -> (f32, f32) {
        let (w, h) = self.client_size();
        (w as f32, h as f32)
    }

    fn tree_dump(&self) -> String {
        match self.cache.node() {
            Some(node) => zephyra_fx::bridge::tree_dump_of(node),
            None => "(还没有布局树——首帧尚未运行)\n".into(),
        }
    }

    fn props_dump(&self, path: &[usize]) -> Option<String> {
        let node = zephyra_fx::bridge::node_at_path(self.cache.node()?, path)?;
        Some(zephyra_fx::bridge::props_dump_of(node))
    }

    fn stats(&self) -> zephyra_fx::bridge::FrameStats {
        self.last_stats.clone()
    }

    /// 布局树根节点：桥侧按 key / path 定位元素靠它。
    fn root(&self) -> Option<&zephyra_core::node::Node> {
        self.cache.node()
    }

    /// 渲染过的帧数（`wait_frames` 按它算）。
    fn rendered_frames(&self) -> u64 {
        self.rendered_frames
    }

    /// 改窗口客户区尺寸（**物理**像素）。
    ///
    /// AI 用它验「窄窗口下这个布局还成立吗」—— 读样式表猜不出来（弹性布局、
    /// 换行、裁切都得真排一遍）。
    ///
    /// # 为什么是**物理**像素
    ///
    /// 桥的坐标系**全是**物理像素（`viewport` 明说 physical-pixels、树的坐标是
    /// 物理、点击坐标是物理）。这里跟着走才不会让 AI 在两套单位之间换算。
    ///
    /// ⚠ winit 的 `request_inner_size` 收的是**逻辑**像素，所以要按缩放因子
    /// 换算回去（125% 的屏幕上 `resize 1000 750` ⇒ 逻辑 800×600 ⇒ 物理
    /// 1000×750）。
    ///
    /// ⚠ winit 0.30 的 Windows 后端**永远**返回 `None`（源码里是无条件
    /// `None`，不是「失败」）。所以**不能**拿它的返回值当成败判据 ——
    /// 早期版本据此报错，而窗口其实**已经改好了**（视口确实变了）。
    fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        use winit::dpi::LogicalSize;
        let scale = self.window.scale_factor() as f32;
        let logical_w = ((width as f32 / scale).round() as u32).max(1);
        let logical_h = ((height as f32 / scale).round() as u32).max(1);
        // ⚠ winit 0.30 的 Windows 后端**永远**返回 `None`（源码里是无条件 `None`，
        // 不是「失败」）。所以返回值**没有判据意义** —— 早期版本据此报错，
        // 而窗口其实**已经改好了**（视口确实变了）。
        let _ = self
            .window
            .request_inner_size(LogicalSize::new(logical_w, logical_h));
        Ok(())
    }

    fn resources(&mut self) -> Result<String, String> {
        let telemetry = self
            .resource_telemetry
            .get_or_insert_with(zephyra_platform_win::ProcessTelemetry::new);
        let sample = telemetry.sample()?;
        Ok(sample.to_text(telemetry.gpu_error()))
    }

    fn inject_hover(&mut self, x: f32, y: f32) {
        self.on_cursor_moved(x, y);
        self.dirty = true;
        self.sync_input();
    }

    fn inject_click(&mut self, x: f32, y: f32) {
        // Keep each edge visible to views that derive clicks from pressed-state changes.
        self.on_cursor_moved(x, y);
        self.sync_input();
        self.on_pointer_down();
        self.dirty = true;
        self.sync_input();
        self.on_pointer_up();
        self.dirty = true;
        self.sync_input();
    }

    fn inject_pointer_down(&mut self) {
        self.on_pointer_down();
        self.bridge_pointer_pressed = true;
        self.dirty = true;
        self.sync_input();
    }

    fn inject_pointer_up(&mut self) {
        self.on_pointer_up();
        self.bridge_pointer_pressed = false;
        self.dirty = true;
        self.sync_input();
    }

    /// MCP server 在按下与抬起之间消失（超时/崩溃/会话取消）：补一次抬起，
    /// 否则应用永远卡在拖动态，后续所有真实输入都被当成拖动的一部分。
    fn on_bridge_disconnect(&mut self) {
        if self.bridge_pointer_pressed {
            self.on_pointer_up();
            self.bridge_pointer_pressed = false;
            self.dirty = true;
            self.sync_input();
        }
    }

    fn profile_start(&mut self, duration_seconds: u64) -> Result<(), String> {
        self.profiler.start(duration_seconds)
    }

    fn profile_report(&mut self) -> Result<String, String> {
        self.profiler.report()
    }

    fn inject_key(&mut self, key: &str) {
        // 名字 → 事件那一步在共用 crate 里：三后端注入的按键行为必须一致。
        self.pending_text_events
            .extend(zephyra_winit_input::injected_events(key));
        self.dirty = true;
        self.sync_input();
    }

    fn inject_text(&mut self, text: &str) {
        // 逐字符**插入**事件（不是按键）：见 `zephyra_winit_input::injected_text_events`
        // 的说明——多发键边沿会让某些控件把每次输入当成一次提交。
        self.pending_text_events
            .extend(zephyra_winit_input::injected_text_events(text));
        self.dirty = true;
        self.sync_input();
    }

    fn inject_scroll(&mut self, x: f32, y: f32, delta_x: f32, delta_y: f32) {
        self.on_cursor_moved(x, y);
        self.pending_scroll = Some(zephyra_fx::ScrollInput {
            x,
            y,
            delta_x,
            delta_y,
        });
        self.dirty = true;
        self.sync_input();
    }

    /// 注入一次**触摸**（多点触控的主入口）。
    ///
    /// # 为什么走**同一条**路
    ///
    /// 真实触摸经 `WindowEvent::Touch` → [`Self::on_touch`]，注入的触摸也走
    /// [`Self::on_touch`] —— 两条路分叉的话，「注入能过、真机不行」这类问题
    /// 就永远查不清是哪条路的锅（与 `inject_click` 的同款理由）。
    fn inject_touch(
        &mut self,
        phase: zephyra_fx::bridge::TouchPhase,
        id: u64,
        x: f32,
        y: f32,
    ) -> Result<(), String> {
        use winit::event::TouchPhase as WPhase;
        let phase = match phase {
            zephyra_fx::bridge::TouchPhase::Down => WPhase::Started,
            zephyra_fx::bridge::TouchPhase::Move => WPhase::Moved,
            zephyra_fx::bridge::TouchPhase::Up => WPhase::Ended,
            zephyra_fx::bridge::TouchPhase::Cancel => WPhase::Cancelled,
        };
        self.on_touch(phase, id, x, y);
        self.dirty = true;
        self.sync_input();
        Ok(())
    }

    fn screenshot(&mut self, opts: zephyra_fx::bridge::ShotOptions) -> Option<Vec<u8>> {
        let (frame, screenshot) = self.frame_inner(Some(opts));
        if frame.redrawn {
            self.present();
        }
        screenshot
    }
}

impl<V: View + 'static> WindowHost for HostWindow<V> {
    fn key(&self) -> WindowKey {
        self.window_key
    }

    fn winit_id(&self) -> Option<WindowId> {
        Some(self.window.id())
    }

    fn handle_event(&mut self, event: WEvent) -> HostEventOutcome {
        // 窗口遮挡/最小化感知：必须在 match 之前处理，避免 Ime 等非 Copy 类型导致部分移动。
        if let WEvent::Occluded(occluded) = event {
            if occluded && self.window_visible {
                self.window_visible = false;
            } else if !occluded && !self.window_visible {
                self.window_visible = true;
                self.cache.invalidate();
                self.dirty = true;
            }
            return HostEventOutcome::Continue;
        }

        match event {
            WEvent::CloseRequested => {
                // 单窗口路径由 `ApplicationHandler` 退出事件循环；多窗口路径由
                // `WindowSet` 只关这一个窗口。这里只把结论报上去。
                return HostEventOutcome::Close;
            }
            WEvent::Resized(_) => {
                // 只置脏，**不**在这里 resize：同一帧里「提交 + resize + 呈现」会打架。
                // 下一帧由 `frame()` 开头的 `sync_swapchain` 处理。
                self.dirty = true;
            }
            WEvent::ScaleFactorChanged { .. } | WEvent::RedrawRequested => {
                self.dirty = true;
            }
            WEvent::CursorMoved { position, .. } => {
                self.on_cursor_moved(position.x as f32, position.y as f32);
                // `revision` 只在 hover/pressed **状态切换**时递增——拖拽中
                // hover 不变，`route()` 不会置脏。不加这行，事件被送进队列
                // 但没有任何一帧被泵起，视图永远看不到 Move（慢速拖动「卡住」）。
                self.dirty = true;
            }
            WEvent::MouseInput { state, .. } => {
                match state {
                    ElementState::Pressed => {
                        self.on_pointer_down();
                    }
                    ElementState::Released => {
                        self.on_pointer_up();
                    }
                }
                self.dirty = true;
            }
            WEvent::Touch(touch) => {
                // ⚠ 触控**必须**接：winit 在 Windows 上 `RegisterTouchWindow`，
                // 那会关掉「触摸提升为鼠标」—— 不接的话触摸事件被整个丢掉，
                // 触控屏上点不动任何东西。
                //
                // 坐标是**物理像素**（`touch.location`），与本宿主其它坐标同一套。
                self.on_touch(
                    touch.phase,
                    touch.id,
                    touch.location.x as f32,
                    touch.location.y as f32,
                );
                self.dirty = true;
            }
            WEvent::ModifiersChanged(m) => {
                self.translator.on_modifiers(m.state());
            }
            WEvent::KeyboardInput { event, .. } => {
                self.translator
                    .key_events(&event, &mut self.pending_text_events);
                // 键盘激活（Space/Enter）**当帧**生效，与下面的 Tab 同理：核心状态机
                // 自己判「这个键是不是激活键、焦点节点可不可激活」，宿主只递键边沿。
                {
                    let kp = zephyra_winit_input::KeyParts::from(&event);
                    let key_input = self.translator.key(kp);
                    let focused = self.focus.focused().cloned();
                    let node = self.cache.node();
                    if let Some(true) = self.activation.on_key(
                        &key_input,
                        node,
                        focused.as_ref(),
                        Context::new(self.theme),
                    ) {
                        self.dirty = true;
                    }
                }
                // Tab 移焦点是**当帧**就该生效的（不像字符要等下一帧 sync_input）：
                // 用户按 Tab 期望立刻看到焦点环跳到下一个控件。
                if event.state == ElementState::Pressed {
                    if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab) =
                        event.logical_key
                    {
                        let shift = self.translator.modifiers().shift();
                        if let Some(node) = self.cache.node_mut() {
                            if self.focus.tab(node, shift, Context::new(self.theme)) {
                                self.dirty = true;
                            }
                        }
                    }
                }
                self.dirty = true;
            }
            WEvent::Focused(false) => {
                // 窗口失焦：焦点必须清空（否则回来后旧焦点还亮着，且按键全落到
                // 一个看不见的节点上）。`clear` 会写回 FOCUSED=false 到旧节点。
                // 激活闩也清（与 D3D11 / Vulkan 宿主一致）。
                if let Some(node) = self.cache.node_mut() {
                    self.focus.clear(Some(node));
                    self.dirty = true;
                }
                self.activation.clear();
            }
            WEvent::Ime(ime) => {
                // 插入点：焦点系统还没建，宿主不知道控件把光标画在哪，
                // 于是用**最近一次指针位置**当代理（用户点进输入框就是在那）。
                // 有了焦点系统之后这里改成向控件要 caret rect —— 那之前不要
                // 假装知道，`Ime::Start` 的坐标本来就是「贴在哪儿」的提示。
                self.translator
                    .ime(&ime, self.pointer_pos, &mut self.pending_text_events);
                self.dirty = true;
            }
            WEvent::MouseWheel { delta, .. } => {
                let (delta_x, delta_y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x, y),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                self.pending_scroll = Some(zephyra_fx::ScrollInput {
                    x: self.pointer_pos.0,
                    y: self.pointer_pos.1,
                    delta_x,
                    delta_y,
                });
                self.dirty = true;
            }
            _ => {}
        }
        self.sync_input();
        HostEventOutcome::Continue
    }

    fn tick(&mut self) -> HostTick {
        // 桥的 poll 放在帧判定之前：注入的 hover/click 在**本帧**就参与命中
        // 测试与布局（与 D3D11 侧同一时序契约）。
        if let Some(mut b) = self.bridge.take() {
            b.poll(self);
            self.bridge = Some(b);
        }

        // ⚠ 窗口不可见（最小化 / 完全遮挡）时直接返回空帧：不 layout、不 paint、不 present。
        // 这让 CPU/GPU 在后台/遮挡时归零，符合 P3 验收标准。
        if !self.window_visible {
            let mut tick = HostTick {
                device_dead: self.chain.device_dead(),
                ..HostTick::default()
            };
            tick.wants_poll = true;
            tick.busy = false;
            return tick;
        }
        // 只在「有待处理的事件」时跑一帧。静止时一帧都不跑 —— 这既符合保留式 UI 的
        // 卖点，也让 demo 在静止时几乎没有 GPU 开销。
        //
        // ⚠ 是否**真的**要画由 core 判定（`Frame::redrawn`）：事件可能什么都没改
        // （鼠标在空白处移动 ⇒ `hovered` 前后都是 `None`），此时 core 返回
        // `Unchanged`，本帧不画。
        //
        // ⚠ 那时**连 present 都不能做**。本后端是 flip model（`FLIP_DISCARD`，
        // DComp 路径 `FLIP_SEQUENTIAL`），`Present` 之后后缓冲内容未定义 ——
        // 不画就 present 会把未定义内容推上屏（通常是黑，或上一次的帧）。
        //
        // ⚠ 设备已死时**无条件**跑一帧：`dirty` 可能已经被清掉（设备是在
        // `present()` 里死的），那样就再没有 `frame()` 可跑，恢复代码永远不执行。
        //
        // ⚠ 「动画播放中」由**视图**回答：保留式 UI 不常驻重画，动画只在事件
        // 到来时推进一帧，然后循环睡死 —— 中间整段弹簧曲线看不到。
        // 视图说「还要帧」时，这里**推一次时钟**（`sync_input` → `on_input` →
        // 动画 `advance`）再标脏；顺手再问一次，若这一拍刚好收敛就把控制流
        // 交还给 `Wait`（否则会多 Poll 一轮、多画一张终态帧）。
        //
        // ⚠ 只标脏不推时钟是不够的：本后端的 `sync_input()` 只在
        // `handle_event` 末尾调用，`frame()` 自己不推。那样泵出来的是一张
        // **冻住的旧画面**，循环空转却毫无变化（比不动更糟：烧 GPU 还看不见动画）。
        let mut animating = self.view.wants_frames();
        if animating {
            self.sync_input();
            self.dirty = true;
            animating = self.view.wants_frames();
        }
        let mut tick = HostTick {
            device_dead: self.chain.device_dead(),
            ..HostTick::default()
        };
        // ⚠⚠ **不要**因为「采样器在跑」就强制渲染。
        //
        // 原先这里有 `|| self.profiler.is_running()` —— 采样时每 tick 都跑一帧，
        // 而 `tick.busy` 里也有它，于是事件循环切到 `ControlFlow::Poll`（全速空转）。
        //
        // 后果是**采样出来的报告不代表真实运行**：全速空转会把帧塞得比 GPU 消费
        // 更快，队列积压 ⇒ 帧变慢。实测（gallery release 1400×800）：
        //
        // | | 采样时（被强制空转） | 正常运行时 |
        // |---|---|---|
        // | `render_submit` 中位数 | **5141 µs** | 300~900 µs |
        // | `wait_gpu` | 阻塞 | 1 µs |
        //
        // 也就是说「帧要 5 ms」这个结论是**测量方法造成的**，不是应用慢。
        //
        // 正确做法：采样器**只记录真的发生的帧**。应用静止 ⇒ 没有帧 ⇒
        // 「frames=0」—— 那本身就是结论（见 `PerformanceProfiler::report`）。
        // 桥的控制流分档：
        //
        // - **有未处理的数据** ⇒ Poll（数据在流，全速才对）；
        // - **有客户端但没数据** ⇒ WaitUntil(10 ms)（命令最多晚 10 ms，
        //   而 CPU 从「全速空转」降到 1% 量级）；
        // - **没有客户端** ⇒ Wait（静止零开销）。
        //
        // ⚠ 不能一律 Poll：TCP 数据唤不醒 Wait，所以「有客户端」看起来
        // 必须 Poll —— 而实测那样会把事件循环拉到 **2500 Hz**（应用只画了
        // 9 帧），白烧 CPU。见 `DebugBridge::has_pending_work` 的说明。
        if let Some(bridge) = self.bridge.as_ref() {
            if bridge.has_pending_work() {
                tick.wants_poll = true;
            } else if bridge.has_client() {
                tick.wake_at =
                    Some(std::time::Instant::now() + std::time::Duration::from_millis(10));
            } else {
                // 桥空闲但还开着：低频重试 accept（新连接也唤不醒 Wait）。
                tick.wake_at =
                    Some(std::time::Instant::now() + std::time::Duration::from_millis(50));
            }
        }
        // ---- 呈现率封顶 ----
        //
        // 距上次呈现不足一个刷新周期时，这一拍**不画**，睡到点再说。
        //
        // # 为什么必须封顶
        //
        // 「GPU 占用」= 呈现帧数 × 每帧 GPU 时间。本后端动画期间是
        // `ControlFlow::Poll`（能画多快画多快），所以**没有上界**：一帧便宜了
        // 就画得更勤，占用原地弹回原处 —— 优化会「看起来没生效」。
        //
        // 封顶之后，每一点「每帧更便宜」的优化都兑现成**占用下降**。
        //
        // # 为什么用显示器的刷新率而不是写死 60
        //
        // 120 Hz 屏上按 60 封顶会白白丢掉一半流畅度。取不到就按 60 算。
        //
        // ⚠ 设备已死时**不封顶** —— 恢复路径（`frame()` 里的 `device_dead()`
        // 分支）必须立刻跑，否则永远恢复不了。
        //
        // ⚠ `dirty` **不清**：这一帧的内容还欠着，到点后照画。输入因此最多
        // 晚一个刷新周期生效 —— 而这正是所有垂直同步应用的行为（屏幕本来就
        // 只能显示那么多帧）。
        if !tick.device_dead {
            if let Some(next) = self
                .last_present_at
                .map(|last| last + self.present_interval)
                .filter(|next| std::time::Instant::now() < *next)
            {
                // 桥的轮询唤醒（10 ms / 50 ms）比刷新周期**短**：两个唤醒点
                // 取**更晚**的那个 —— 桥的轮询只是为了尽快 accept 新连接，
                // 它不产生帧，早醒也只是白醒一趟。
                if tick.wake_at.is_none_or(|w| w < next) {
                    tick.wake_at = Some(next);
                }
                // 这一拍不画。
                //
                // ⚠ 同时把 Poll 关掉：桥的数据在 `tick()` 开头已经 poll 过了，
                // 剩下的只是「新数据到了没有」—— 睡到下个刷新周期再问也一样。
                // 不关的话事件循环会在这 16 ms 里空转 2500 圈（CPU 白烧）。
                tick.wants_poll = false;
                return tick;
            }
        }
        if self.dirty || self.chain.device_dead() || self.uia_needs_frame() {
            let f = self.frame();
            if f.redrawn {
                self.present();
                tick.painted = true;
            }
        }
        // ⚠ `busy` 里**不要**有 `profiler.is_running()`（理由见上面那段注释）：
        // 那会让事件循环在采样期间全速空转，而空转出来的帧**不代表真实运行**。
        tick.busy = self.dirty || self.uia_needs_frame() || animating;
        tick
    }

    fn start(&mut self) -> Result<(), String> {
        // 窗口在构造函数里就建好了；这里补第一帧。
        self.sync_input();
        self.frame();
        self.present();
        // 拖边时逐帧跟随。**必须放在第一帧之后**：钩子里会 `frame()`，
        // 而那时 `cache` 还是空的 —— 空树上跑一趟布局既是白费，也更容易
        // 把「首帧就该建好的缓存」留到一个不对劲的时刻再建。
        if let Err(e) = self.enable_live_resize() {
            eprintln!("[zephyra-d3d12] 装不上实时 resize 子类，拖边时会退回「拖完才刷新」: {e}");
        }
        Ok(())
    }

    fn set_theme(&mut self, variant: zephyra_core::theme::ThemeVariant) {
        HostWindow::set_theme(self, variant);
    }

    fn set_styler(&mut self, styler: Box<dyn zephyra_core::styler::Styler>) {
        HostWindow::set_styler(self, styler);
    }

    fn set_title(&mut self, title: &str) {
        self.window.set_title(title);
    }

    fn request_redraw(&self) {
        self.window.request_redraw();
    }

    fn request_close(&mut self) {
        // 有应用侧句柄就投命令（`WindowSet` 下一轮 drain 时关窗口）；
        // 单窗口兼容路径没有句柄，调用方自己决定怎么退出。
        if let Some(handle) = &self.handle {
            handle.close();
        }
    }

    fn teardown(&mut self) {
        // 摘掉窗口过程子类与渲染钩子：钩子闭包持有指向本宿主的裸指针，
        // 宿主一析构，下一次 `WM_SIZE` 就是 use-after-free。
        if let Some(hwnd) = hwnd_of(self.window.as_ref()) {
            zephyra_platform_win::restore_live_resize(hwnd);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// D3D12 后端的 [`Backend`] 实现。
///
/// 只是把 [`WindowSpec`] 解析成 [`zephyra_app_config::WindowOpts`] 再交给
/// [`HostWindow::new_lenient_opts`]。共享 GPU 栈由 `shared_stack()` 单例保证 ——
/// 每个窗口都拿到同一个 `Arc<Mutex<Shared>>`。
#[derive(Default)]
pub struct D3d12Backend;

impl D3d12Backend {
    /// 建一个后端入口（共享栈在第一个窗口创建时才真正初始化）。
    pub fn new() -> Self {
        Self
    }
}

impl Backend for D3d12Backend {
    type Window<V: View + 'static> = HostWindow<V>;

    fn create<V: View + 'static>(
        &mut self,
        event_loop: &ActiveEventLoop,
        view: V,
        spec: &WindowSpec,
        handle: WindowHandle,
    ) -> Result<Self::Window<V>, String> {
        let config = zephyra_app_config::load_from_current_dir();
        let opts = spec.resolve(&config);
        let size = opts.size.unwrap_or(DEFAULT_WINDOW_SIZE);
        let mut host = HostWindow::new_lenient_opts(event_loop, view, size, opts);
        host.window_key = handle.key();
        host.handle = Some(handle);
        Ok(host)
    }
}

impl<V: View + 'static> winit::application::ApplicationHandler for HostWindow<V> {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        let _ = self.start();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WEvent) {
        if self.handle_event(event) == HostEventOutcome::Close {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let tick = self.tick();
        // 单窗口兼容路径：自己决定控制流。多窗口时这个决定上提到 `WindowSet`
        // （N 个窗口各自设控制流会互相覆盖，最后一个说话的决定整个循环）。
        if tick.device_dead {
            let next = std::time::Instant::now() + DEVICE_RECOVERY_RETRY_INTERVAL;
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(next));
        } else if tick.wants_poll {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
        } else if let Some(wake) = tick.wake_at {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(wake));
        } else if tick.busy {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        }
    }
}

/// 呈现间隔 = 1 / 显示器的刷新率。
///
/// # 为什么取显示器的而不是写死 60
///
/// 120 Hz 屏上按 60 封顶会白白丢掉一半流畅度；而**不知道**刷新率时按 60
/// 算最保守（宁可少画，也不要超过 60 帧）。
///
/// 取不到（虚拟机 / 远程桌面 / 驱动不报）时退回 60 Hz。
fn refresh_interval(window: &winit::window::Window) -> std::time::Duration {
    let hz = window
        .current_monitor()
        .and_then(|m| m.refresh_rate_millihertz())
        .map(|mhz| mhz / 1000)
        .filter(|hz| (30..=500).contains(hz))
        .unwrap_or(60);
    std::time::Duration::from_nanos(1_000_000_000 / hz as u64)
}

/// 用给定视图跑一个 D3D12 窗口应用。
///
/// 与 `zephyra-winit::run_with` 同形：调用方若自举（`ApplicationHandler`），用它自己
/// 的那一套；否则用它。
pub fn run_with<V: View + 'static>(app: HostWindow<V>) -> Result<(), winit::error::EventLoopError> {
    let mut app = app;
    let el: EventLoop<()> = EventLoop::new()?;
    el.run_app(&mut app)
}

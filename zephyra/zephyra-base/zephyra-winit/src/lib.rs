#![deny(rustdoc::broken_intra_doc_links)]
//! # zephyra-winit
//!
//! ZephyraUI 的 winit + D3D11 宿主适配层，位于系统适配层 `zephyra/zephyra-base/`
//! （见该目录的 README）。
//!
//! ## 职责
//!
//! 1. 用 winit 建窗口、收事件，把指针/触摸/键盘/滚轮翻译成核心的输入类型；
//! 2. 用 D3D11 建设备与交换链，把 `zephyra_core::paint::DisplayList` 回放成一帧。
//!
//! ## 不做什么
//!
//! - 不做布局、不做命中、不做文本整形之外的任何决策 —— 那些在核心里；
//! - 不缓存字形图集的生存期语义（那是纹理资源的管理）。
//!
//! 判据很硬：换一个 base 实现（DXGI 传统路径 ↔ DirectComposition，或
//! D3D11 ↔ 别的后端），同一棵元素树必须产出逐字节相同的 `DisplayList`。
//! 凡是在替换本 crate 后会改变 `DisplayList` 内容的逻辑，都放错了层。
//!
//! ## 模块划分（每个只有一件事）
//!
//! | 模块 | 职责 | 依赖 GPU？ |
//! |---|---|---|
//! | [`shader`] | HLSL 源码 + `D3DCompile` FFI | 是（要设备） |
//! | [`renderer`] | 管线状态、顶点缓冲、每帧 draw | 是 |
//! | [`surface`] | 交换链 + 可选 DComp 树 + present | 是 |
//! | （`zephyra-fx`） | `DisplayList` → 顶点流、字形图集、纹理表、CPU 光栅器 | 否 |
//!
//! `DisplayList` → 顶点流那一整段（原先是本 crate 的 `quad`/`texture`/`text`/
//! `cpu_raster`/`cpu_only`）**已经搬到 `zephyra-fx`**。它们一个 GPU 类型都不碰，
//! 而放在这里会让 D3D12 后端既没有字体也没有 CPU 诊断能力。
//!
//! ## 当前状态
//!
//! [`Renderer`] 能把 `DisplayList` 回放成像素：圆角矩形与描边走片元 SDF，
//! 文本与图像走纹理采样，`Command::Path`（矢量图标）走 CPU 三角化后的三角形流、
//! 抗锯齿靠 MSAA。

#![cfg(windows)]
#![allow(unsafe_code)]

use std::sync::{Arc, Mutex, OnceLock};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};
use zephyra_core::context::Context;
use zephyra_core::core::Color;
use zephyra_core::diff::DiffStats;
use zephyra_core::element::Element;
use zephyra_core::frame::{Damage, FrameCache, FrameOutcome};
use zephyra_core::id::NodeTag;
use zephyra_core::input::{PointerPhase, RoutedEvent, RoutedEvents};
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_core::theme::Theme;
use zephyra_window::{
    Backend, HostEventOutcome, HostTick, WindowHandle, WindowHost, WindowKey, WindowSpec,
};

pub mod ext;
pub mod renderer;
pub mod shader;
pub mod surface;
pub mod texture;

// ── 来自 `zephyra-fx` 的再导出 ─────────────────────────────────────────────
//
// 这些类型**没有后端依赖**，却曾经住在这个 D3D11 后端 crate 里，于是 DX12 后端
// 一个也用不到。下面按原路径再导出，是为了让既有调用点
// （`zephyra_winit::cpu_only::CpuHost`、`zephyra_winit::quad::kind::GLYPH`）
// 不用改 —— 搬 crate 是结构调整，不该顺手改调用点的名字。
#[cfg(debug_assertions)]
pub use zephyra_fx::cpu_only;
pub use zephyra_fx::{cpu_raster, pixels, quad, text};
pub use zephyra_fx::{
    from_font_file, from_system_font, Frame, InputState, PixelRead, RenderReport, TextEngine,
    TextureNames, View, FONT_OVERRIDE_ENV,
};
pub use zephyra_fx::{srgb_bytes, write_bmp};

pub use renderer::{RenderCtx, Renderer};
pub use surface::{dcomp_enabled, drain_severe_messages, info_queue, Gpu, Surface, SurfaceError};
pub use texture::TextureTable;

pub use winit;
pub use zephyra_core;
pub use zephyra_fx;

/// 标量属性的本地定义（宿主自定义控件用）。
///
/// 定义搬到了 [`zephyra_fx::Z_INDEX`] —— 本 crate 与 `zephyra-winit-d3d12` 是两个
/// 独立 crate，而属性键靠**名字**派生 id，所以两份定义**恰好**能互换；一旦只改一处，
/// 另一处就静默变成另一个键，症状是「z-index 不生效」且没有任何编译或运行错误。
pub use zephyra_fx::Z_INDEX;

/// 用系统默认程序打开外部 URL（超链接的真实功能，View 只发意图）。
fn open_external_url(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn();
    }
    #[cfg(not(windows))]
    let _ = url;
}

/// 窗口 + GPU + 保留树的组合体。
///
/// 保留树（`Node`）跨帧存活 —— 这正是保留式相对即时式的收益：状态与纹理
/// 得以复用，只有真正变化的子树被重排/重绘。
///
/// # 字段顺序不是随意的
///
/// `surface` 持有 DComp 树（含 target 的 COM 引用），必须比窗口活得久
/// （DComp 硬约束 #1）。Rust 按**声明顺序**释放字段，所以 `surface` 必须声明在
/// `window` **之后** —— 否则窗口先死、DComp 树后死，绑定被拆除时窗口已不可见，
/// 现象是关窗瞬间闪一下黑。
/// 进程级共享的 D3D11 设备与立即上下文。
///
/// # 为什么共享
///
/// 一个进程一个设备：多窗口共用同一个 device/context，而 D3D11 的**立即上下文
/// 不是线程安全的** —— 所有记录 / `Present` 都在同一把锁内串行（同步 `Map` 与
/// 状态切换也在这把锁里，见各调用点）。
///
/// 每个窗口自己的 `Renderer` / `TextEngine` / `TextureTable` 仍归各自宿主：它们
/// 与交换链和内容树一起是窗口私有的。这与 D3D12 那份把 `Renderer` 也放进共享栈
/// 的做法不同 —— D3D12 的 PSO/图集是显式资源，共享收益更大；D3D11 的管线状态
/// 由驱动隐式缓存，共享 `Renderer` 反而会把两套图集挤进同一份 SRV 表。
struct SharedGpu {
    gpu: Gpu,
}

// SAFETY: 只通过 `SHARED_GPU` 的 `Mutex` 访问；所有 D3D11 调用都在锁内串行。
unsafe impl Send for SharedGpu {}

static SHARED_GPU: OnceLock<Arc<Mutex<SharedGpu>>> = OnceLock::new();

/// 取（必要时建）进程级共享设备。
fn shared_gpu() -> Arc<Mutex<SharedGpu>> {
    SHARED_GPU
        .get_or_init(|| {
            Arc::new(Mutex::new(SharedGpu {
                gpu: Gpu::new().expect("创建 D3D11 设备：需要可用的 GPU"),
            }))
        })
        .clone()
}

/// 锁住共享设备（锁中毒时继续用）。
fn lock_gpu(shared: &Arc<Mutex<SharedGpu>>) -> std::sync::MutexGuard<'_, SharedGpu> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// 获取显示器刷新周期（默认 60Hz）。
fn refresh_interval(window: &Window) -> std::time::Duration {
    let hz = window
        .current_monitor()
        .and_then(|m| m.refresh_rate_millihertz())
        .map(|mhz| mhz / 1000)
        .filter(|hz| (30..=500).contains(hz))
        .unwrap_or(60);
    std::time::Duration::from_nanos(1_000_000_000 / hz as u64)
}

pub struct HostWindow<V: View> {
    /// 上一次 `sync_input` 的时刻 —— 动画时钟的唯一来源（`InputState::dt_ms`）。
    ///
    /// 放在最前面与释放顺序无关（`Instant` 无 Drop 语义），只为让"帧间隔从哪来"
    /// 一眼可寻。`None` = 还没有上一帧，于是 dt 为 0 —— 首帧没有
    /// "距上一帧多久"可言，填 0 而不是猜一个值，否则首帧动画从中间开始。
    last_frame_at: Option<std::time::Instant>,
    window: std::sync::Arc<Window>,
    /// D3D11 设备与上下文（进程级共享，多窗口同一份）。
    gpu: Arc<Mutex<SharedGpu>>,
    /// 交换链 + 可选 DComp 树。
    surface: Surface,
    renderer: Renderer,
    /// 本窗口的稳定标识（`WindowSet` 分发用）。
    window_key: WindowKey,
    /// 应用侧句柄（命令队列）。单窗口兼容路径下为 `None`。
    handle: Option<WindowHandle>,
    view: V,
    theme: &'static Theme,
    /// 帧缓存 —— 保留树 + `DisplayList` + 上一帧视口，全在核心里。
    ///
    /// # 为什么这三样都不在本结构体里
    ///
    /// 它们原来分散在 `node` / `list` / `needs_layout` 三个字段里，而**数据**全在
    /// core（`Node` / `DisplayList` / `Dirty` / `DiffStats`）。本 crate 于是要自己
    /// 编排「要不要重排 / 要不要重绘」—— 那正是 `zephyra/zephyra-base/README.md` 第二条
    /// 契约禁止的（base 不决定画什么）。
    ///
    /// 现在这一帧画什么只有 core 一个地方决定（[`FrameCache::frame`]），本 crate
    /// 只回答另一个问题：要不要把它推上屏。
    cache: FrameCache,
    router: zephyra_core::input::InputRouter,
    pointer: Option<u32>,
    /// 桥注入的「主指针按住」状态。断连收尾靠它判断要不要补一次抬起
    /// （见 [`zephyra_fx::bridge::BridgeHost::on_bridge_disconnect`]）。
    /// 本后端的 `on_pointer_up` 不清 `pointer`，更不能拿它当判据。
    #[cfg(debug_assertions)]
    bridge_pointer_pressed: bool,
    pointer_pos: (f32, f32),
    pending_text_events: Vec<zephyra_core::input::TextEvent>,
    /// winit 按键 → core 文本事件的翻译器（修饰键状态也住在这里）。
    ///
    /// 与 D3D12 / Vulkan 宿主共用 `zephyra-winit-input` 里的同一份实现。
    translator: zephyra_winit_input::Translator,
    /// winit 指针事件 → core 指针事件的翻译器（**含单调时钟**）。
    ///
    /// ⚠ 触控这条路**必须**有它：`winit` 的 `Touch` 事件不带时间戳，而静态
    /// 手势（轻点 / 双击 / 长按）是计时的。
    pointer_translator: zephyra_winit_input::PointerTranslator,
    /// 焦点状态机：Tab 移动、点击设焦点、失焦清空。跨帧持有。
    focus: zephyra_core::input::FocusState,
    /// 键盘激活状态机：焦点落在 ACTIVATABLE 节点上时 Space/Enter 闩住 pressed。
    /// 与 [`Self::focus`] 同址跨帧持有；结果并进 `pressed` 通道（见 `sync_input`）。
    activation: zephyra_core::input::ActivationState,
    pending_scroll: Option<zephyra_fx::ScrollInput>,
    pending_pointer_events: Vec<zephyra_fx::PointerInput>,
    /// 待发的**静态手势**（轻点 / 双击 / 长按）。
    pending_gestures: Vec<zephyra_fx::GestureEvent>,
    /// 最近一次**输入事件**到达的时刻（测「输入 → 帧就绪」的延迟）。
    input_at: Option<std::time::Instant>,
    /// 上一帧测到的「输入 → 帧就绪」耗时（微秒）。
    input_latency_us: Option<u64>,
    /// 待发的**操作增量**（多指平移 / 缩放 / 旋转 + 速度）。
    pending_manipulations: Vec<zephyra_core::input::ManipulationDelta>,
    /// 操作结束时的平移速度（px/秒）—— **惯性的初值**。
    pending_inertia: Option<(f32, f32)>,
    /// 有没有**待处理的事件**（resize / ptr_of / 按键 / 切主题）。
    ///
    /// ⚠ 这**不是**「内容变了」。内容变没变由 core 判定（`FrameOutcome`）。
    /// 本字段只表示「值不值得跑一帧」—— 因为判断后者必须先跑 `view()`，而那是
    /// 应用代码。
    dirty: bool,
    /// 窗口是否可见（未最小化、未被完全遮挡）。
    ///
    /// `WindowEvent::Occluded` / `WindowEvent::Focused` 维护。不可见时 `tick()`
    /// 直接返回空帧、不 layout、不 paint、不 present。恢复可见时
    /// `cache.invalidate()` 强制下一帧完整重绘。
    window_visible: bool,
    /// 上一次呈现的时刻（用于 VSync 帧 pacing）。
    last_present_at: Option<std::time::Instant>,
    /// 显示器刷新周期（纳秒精度的 `Duration`）。
    present_interval: std::time::Duration,
    /// 文本引擎。
    ///
    /// 泛型参数固定为 [`text::AbGlyphSource`]（本 crate 的唯一字体后端）。用
    /// `RefCell` 是为了让 core 的 `Context` 保持 `Copy` —— 布局递归要求这一点。
    engine: std::cell::RefCell<TextEngine>,
    /// 已上传的图集字形数，用来避免每帧重传 4MB。
    atlas_revision_uploaded: u64,
    /// 图像纹理表：`Command::Image` 的句柄 → 采样视图。
    textures: TextureTable,
    /// 调试桥。两种拓扑共用同一套连接协议（见 `zephyra_fx::bridge`）：
    ///
    /// - **监听**：设了 `ZEPHYRA_DEBUG_PORT` ⇒ 等客户端连进来；
    /// - **连出**：CLI 注入 `ZEPHYRA_MCP_PORT` ⇒ 应用连入 MCP。
    ///
    /// 两者可并存。连出模式尚未连上 MCP 时，宿主每 50 ms 唤醒一次驱动重连
    /// （见本文件 `HostTick::wake_at` 的计算）；连上后改为 10 ms 轮询。
    ///
    /// MCP 重启 ⇒ 连出流断开，`BridgeHost::on_bridge_disconnect` 只在**客户端
    /// 全消失那一刻**调用一次，宿主借此补抬注入的指针（否则会卡在「按住」态）；
    /// 自动重连成功后不会重复调用。
    ///
    /// `Option` 而不是总是建：桥是**调试设施**，默认必须零开销——不监听
    /// 任何端口，不占一个字节缓冲。
    #[cfg(debug_assertions)]
    bridge: Option<zephyra_fx::bridge::DebugBridge>,
    /// dev 资源热重载器（.zstyle / .zypui）。release 构建里字段不存在。
    #[cfg(debug_assertions)]
    dev_reloader: Option<zephyra_fx::dev_reload::DevReloader>,
    /// 最近一帧的统计文本（`stats` 命令的答案，`frame()` 末尾刷新）。
    /// 最近一帧的统计（`stats` 命令的答案）。结构体而不是文本：文本与 JSON
    /// 两种形态从同一份数据出。
    #[cfg(debug_assertions)]
    last_stats: zephyra_fx::bridge::FrameStats,
    /// **渲染过**的帧数（`wait_frames` 按它算）。
    rendered_frames: u64,
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    resource_telemetry: Option<zephyra_platform_win::ProcessTelemetry>,
    #[cfg(debug_assertions)]
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
    /// 窗口句柄（裸 `HWND`）。
    ///
    /// 存的是 `isize` 而不是 `windows::…::HWND`：`HWND` 是 `Send`/`Sync` 的
    /// 门槛更高（它带裸指针），而本结构体要在 `Arc` 里被别的线程碰到。
    hwnd: Option<isize>,
    /// 本窗口是否透明（逐像素 alpha 进合成器）。
    ///
    /// 它决定**清屏色**：透明窗口清成 alpha=0（premultiplied 语义下 RGB 随
    /// alpha 归零），不透明窗口清成主题底色。创建时确定，运行期不变 ——
    /// 与「资源拓扑创建时定」同一取舍。
    transparent: bool,
}

impl<V: View> HostWindow<V> {
    /// 建窗口与 GPU 资源，**找不到系统字体时不 panic**。
    ///
    /// 字体缺失不该让整个应用起不来：文本会不显示，其余部分照常工作。这与"崩溃"
    /// 之间的差别很大 —— 后者让"这台机器没装字体"变成"这个程序不能跑"。
    ///
    /// 同步构造（可在 winit 的 `resumed` 回调里直接调用）：D3D11 的设备与交换链
    /// 都是同步 API，不像 wgpu 的设备请求那样需要 `block_on` 两个 future
    /// —— 所以本 crate **不再依赖 pollster**。
    pub fn new_lenient(event_loop: &ActiveEventLoop, view: V) -> Self {
        Self::new_lenient_opts(event_loop, view, zephyra_app_config::WindowOpts::default())
    }

    /// [`Self::new_lenient`] + **按窗口的创建时选项**。
    ///
    /// # 为什么透明是 per-window 参数而不是进程级开关
    ///
    /// 「不同窗口可以有不同定义」是本次的硬要求（`window.transparent` 只是
    /// 主窗口默认值）。透明目前只能由**创建时**决定 —— 窗口的 alpha 语义一旦
    /// 交给 DWM/合成器就固定了，事后翻转意味着资源拓扑重建，那是 resize 都
    /// 不用的重量级操作。
    ///
    /// 隐含规则：**transparent ⇒ frameless**。系统标题栏是实色矩形，叠在
    /// alpha 窗口上语义不成立；配置里两个都写时透明优先于「带装饰」。
    pub fn new_lenient_opts(
        event_loop: &ActiveEventLoop,
        view: V,
        opts: zephyra_app_config::WindowOpts,
    ) -> Self {
        let config = zephyra_app_config::load_from_current_dir();
        let transparent = opts.resolve_transparent(&config);
        let frameless = opts.resolve_frameless(&config);
        let resizable = opts.resolve_resizable(&config);
        let title = opts
            .title
            .clone()
            .unwrap_or_else(|| "ZephyraUI".to_string());
        let attrs = Window::default_attributes()
            .with_title(title)
            .with_decorations(!frameless)
            .with_transparent(transparent)
            .with_resizable(resizable);
        let window = std::sync::Arc::new(event_loop.create_window(attrs).expect("create_window"));
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
            "[zephyra] 窗口装饰: {}",
            if frameless { "frameless" } else { "system" }
        );
        let size = window.inner_size();

        // 文本引擎：core 的 `TextStack` + 本 crate 的 ab_glyph 字形来源。
        // 找不到字体时退回禁用模式 —— 文本不显示，其余部分照常。
        let (engine, note) = text::from_system_font();
        eprintln!("[zephyra] {note}");

        let gpu = shared_gpu();
        let gpu_guard = lock_gpu(&gpu);
        let gpu_ref = &gpu_guard.gpu;

        // DComp 走不走由 `ZEPHYRA_DCOMP` 决定。它只影响**要不要建 DComp 树**，
        // 两条路的调用方式完全一致（见 `surface` 模块文档），所以这里没有回退分支：
        // 启用后 DComp 建树失败就是失败，不静默退回 —— 那会让"我开了 DComp 却没生效"
        // 无从排查。
        //
        // 透明是另一条强制启用 DComp 的理由：普通 HWND 交换链实测拒收
        // `PREMULTIPLIED`（`surface.rs` 建 HWND 交换链处的注释），只有 composition
        // 交换链能把 alpha 交给合成器。所以 transparent=true 时无视
        // `ZEPHYRA_DCOMP=0` 直接走 DComp —— 那不是配置覆盖，是「透明只能走这条路」。
        let use_dcomp = dcomp_enabled() || transparent;
        let hwnd = surface::hwnd_of(window.as_ref())
            .map(|h| windows::Win32::Foundation::HWND(h as *mut _))
            .ok_or_else(|| panic!("DComp 需要 Win32 HWND；非 Windows 后端尚未实现渲染器"))
            .expect("取 HWND");
        // 裸句柄存一份：UIA 的窗口过程子类要用它（`zephyra_uia::install`）。
        let raw_hwnd = hwnd.0 as isize;

        let surface = match Surface::new(hwnd, gpu_ref, (size.width, size.height), use_dcomp) {
            Ok(s) => s,
            Err(e) => {
                if use_dcomp {
                    if transparent {
                        // 透明窗口没有「退回普通 DXGI」这个选项：普通交换链拒收
                        // premultiplied（见上面 dcomp_enabled 处的注释），退过去
                        // 不是降级，是把窗口**悄悄**变回不透明 —— 那是改定义。
                        panic!("透明窗口必须走 DComp，而 DComp 建链失败: {e}");
                    }
                    // 建树失败不能静默：少了 DComp 特性窗口依然完全可用，为此启动失败
                    // 是本末倒置；但**必须让用户看得见**，否则无从排查。
                    eprintln!("[zephyra] DComp 不可用（{e}），退回普通 DXGI 交换链");
                    Surface::new(hwnd, gpu_ref, (size.width, size.height), false)
                        .expect("普通交换链")
                } else {
                    panic!("创建交换链失败: {e}");
                }
            }
        };
        eprintln!(
            "[zephyra] 呈现路径: {}{}",
            if use_dcomp {
                "DirectComposition"
            } else {
                "DXGI swapchain"
            },
            if transparent {
                "（透明窗口）"
            } else {
                ""
            }
        );
        let renderer = Renderer::new(&gpu_ref.device).expect("编译着色器 / 建管线");
        // 首帧就要上传图集（字体字形此刻还没打包，但纹理得先有内容）。
        renderer.upload_atlas(&gpu_ref.context, engine.atlas()).ok();
        drop(gpu_guard);

        let theme: &'static Theme = Box::leak(Box::new(Theme::dark()));
        let present_interval = refresh_interval(&window);

        Self {
            last_frame_at: None,
            window,
            gpu,
            window_key: WindowKey::unspecified(),
            handle: None,
            surface,
            renderer,
            view,
            theme,
            cache: FrameCache::new(),
            router: zephyra_core::input::InputRouter::new(),
            pointer: None,
            #[cfg(debug_assertions)]
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
            input_at: None,
            input_latency_us: None,
            pending_manipulations: Vec::new(),
            pending_inertia: None,
            dirty: true,
            window_visible: true,
            engine: std::cell::RefCell::new(engine),
            atlas_revision_uploaded: 0,
            textures: TextureTable::new(),
            #[cfg(debug_assertions)]
            bridge: zephyra_fx::bridge::DebugBridge::from_env(),
            #[cfg(debug_assertions)]
            dev_reloader: None,
            #[cfg(debug_assertions)]
            last_stats: zephyra_fx::bridge::FrameStats::default(),
            rendered_frames: 0,
            resource_telemetry: None,
            #[cfg(debug_assertions)]
            profiler: zephyra_fx::bridge::PerformanceProfiler::new(),
            uia: None,
            hwnd: Some(raw_hwnd),
            transparent,
            last_present_at: None,
            present_interval,
        }
    }

    /// 替换调试桥（P2-5：IDE 直接启动 / 嵌入式宿主也能声明连入 MCP）。
    ///
    /// 默认构造仍走 `from_env()`；不依赖环境变量时用
    /// `DebugBridge::with_config(BridgeConfig { connect_to: Some(addr), .. })` 构造后传入。
    #[cfg(debug_assertions)]
    pub fn set_bridge(&mut self, bridge: Option<zephyra_fx::bridge::DebugBridge>) {
        self.bridge = bridge;
    }

    /// 装一个 dev 资源热重载器。**仅 debug 构建存在**；release 没有这个方法。
    #[cfg(debug_assertions)]
    pub fn set_dev_reloader(&mut self, reloader: zephyra_fx::dev_reload::DevReloader) {
        self.dev_reloader = Some(reloader);
    }

    /// 装一个样式注入点（每帧 diff 之后、布局之前调用）。
    ///
    /// 转发给 [`FrameCache::set_styler`]。分离在这里而不是塞进 `new` 的签名：
    /// 样式表是**运行期**才加载的东西（dev 模式还要能热重载），
    /// 而窗口是启动时就得建好的。把它塞进构造器会让「起窗口」依赖
    /// 「读样式表成功」——那样一个拼错的属性名就让程序起不来。
    pub fn set_styler(&mut self, styler: Box<dyn zephyra_core::styler::Styler>) {
        self.cache.set_styler(styler);
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
    /// 必须在**别的窗口过程子类之后**装（`zephyra-platform-win` 的实时 resize）——
    /// 消息链才是 `UIA → live_resize → winit`。
    ///
    /// # 失败怎么办
    ///
    /// 失败**不**是致命错误：无障碍是附加能力，窗口该开还得开。调用方应当
    /// 打印一条警告后继续跑。
    pub fn enable_uia(&mut self) -> Result<(), zephyra_uia::Error> {
        let Some(hwnd) = self.hwnd else {
            return Err(zephyra_uia::Error::WindowProc(0));
        };
        zephyra_uia::install(hwnd)?;
        self.uia = Some(zephyra_uia::Bridge::new());
        Ok(())
    }

    /// 每帧的无障碍维护：发布快照 + 施加 UIA 侧排队的动作。
    ///
    /// # 调用时机
    ///
    /// 在 `frame_inner` 里、`sync_input` **之后**调 —— 顺序不能换：
    /// `zephyra_uia::Bridge` 的动作要跨三帧才被应用看见（闩住 → 松开 → 应用
    /// 认 click），而它依赖「本帧的树已经建好」。
    ///
    /// 见 [`zephyra_uia::Bridge::pump`]。
    fn pump_uia(&mut self) {
        if self.uia.is_none() {
            return;
        }
        // 1. 发布无障碍树快照。
        //
        // 快照来自**保留树**（画出来的那棵），所以它不会与画面漂移。
        if let Some(root) = self.cache.node() {
            let win = zephyra_uia::window_info(self.hwnd.unwrap_or(0));
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
        if pumped.pressed_changed {
            // `pressed` 变了 ⇒ 下一帧的 `sync_input` 要把它报给应用
            // （本宿主每帧开头都 `sync_input`，所以这里只要标脏就够）。
            self.dirty = true;
        }
    }

    /// 图像纹理表（只读）。宿主用 [`Self::textures_mut`] 改。
    pub fn textures(&self) -> &TextureTable {
        &self.textures
    }

    /// 图像纹理表（可变）。宿主在这里注册 / 更新纹理。
    pub fn textures_mut(&mut self) -> &mut TextureTable {
        &mut self.textures
    }

    /// 扩展上下文句柄（与渲染器共享纹理池/Hook 表；D3D11 最小闭环）。
    pub fn ext_context(&self) -> zephyra_ext::ExtContextHandle {
        let gpu = lock_gpu(&self.gpu);
        self.renderer.ext_handle(&gpu.gpu.context)
    }

    /// 注册一张图像纹理，返回写进 `props::IMAGE_TEXTURE` 的句柄。
    ///
    /// 像素是 RGBA8、行优先。注册后**不要**销毁纹理 —— 表里持有它的采样视图，
    /// 而视图引用纹理的内存。要释放纹理时先 [`Self::forget_texture`]。
    ///
    /// 同名重复注册会**替换**纹理，且**不需要**任何额外的清理调用：下一帧自然用
    /// 新的采样视图（D3D11 每次 draw 直接绑 SRV，没有 wgpu 那种需要手动失效的
    /// bind group 缓存）。
    pub fn register_texture(
        &mut self,
        name: &str,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> zephyra_core::paint::TextureRef {
        let gpu = lock_gpu(&self.gpu);
        let tex =
            texture::create_rgba_texture(&gpu.gpu.device, &gpu.gpu.context, pixels, width, height)
                .expect("创建纹理");
        let srv = texture::create_srv(&gpu.gpu.device, &tex).expect("创建采样视图");
        drop(gpu);
        self.textures.insert(name, srv, tex);
        zephyra_core::paint::TextureRef::new(name.to_string())
    }

    /// 原地更新一张已注册纹理的像素（尺寸不变）。供视频帧、动态图标用。
    ///
    /// 返回 `false` = 纹理未注册或像素长度不足 —— 此时什么都没写入，调用方必须
    /// 自己重建纹理。采样视图不变，所以没有任何需要失效的缓存。
    pub fn update_texture(&self, name: &str, pixels: &[u8], width: u32, height: u32) -> bool {
        let gpu = lock_gpu(&self.gpu);
        self.textures
            .update(&gpu.gpu.context, name, pixels, width, height)
    }

    /// 移除一张图像纹理。**必须在销毁纹理之前调用。**
    pub fn forget_texture(&mut self, name: &str) {
        self.textures.remove(name);
    }

    /// 当前主题。
    pub fn theme(&self) -> &Theme {
        self.theme
    }

    /// 窗口物理像素尺寸。
    pub fn size(&self) -> (u32, u32) {
        self.surface.size()
    }

    /// 窗口缩放比。断点判定要它把物理视口折成有效像素（见 `InputState::viewport`）。
    pub fn scale_factor(&self) -> f32 {
        self.window.scale_factor() as f32
    }

    /// 切主题：换 `Theme` 引用并强制重绘。
    ///
    /// 树**不动** —— 属性里存的资源键保持为键，切主题只是换了一个 `&Theme`。
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = Box::leak(Box::new(theme));
        self.dirty = true;
    }

    /// 当前悬停的可交互节点 key（`Display` 形式，供示例比对）。
    pub fn hovered(&self) -> Option<String> {
        self.router.hovered().map(|k| k.to_string())
    }

    /// IME 候选窗该贴的插入点坐标（见 D3D12 宿主同名方法）。
    fn ime_caret(&self) -> (f32, f32) {
        let Some(node) = self.cache.node() else {
            return self.pointer_pos;
        };
        let Some(focused) = self.focus.focused() else {
            return self.pointer_pos;
        };
        let Some(field) = node.find(focused) else {
            return self.pointer_pos;
        };
        if let Some(cursor) = field.find(&zephyra_core::id::ElementKey::Name("cursor")) {
            let r = cursor.rect();
            // `rect()` 不含绘制期 translate-x（光标靠它跟随文本），必须加上。
            let tx = cursor.translate_x(zephyra_core::context::Context::new(self.theme));
            return (r.x + tx, r.y + r.height);
        }
        let r = field.rect();
        (r.x, r.y + r.height)
    }

    /// 推进一帧：声明 → 核心帧缓存 → （必要时）回放。
    ///
    /// # 分工
    ///
    /// 「这一帧画什么」由 [`FrameCache::frame`] 判定 —— 包括「与上次完全相同，
    /// 什么都不用做」。本函数只在 core 说 [`FrameOutcome::Updated`] 时才回放，
    /// 所以稳定帧省掉的是 `paint`（73 µs）+ `expand`（40 µs）+ 一次 present。
    ///
    /// ⚠ **图集上传不受这个门控保护**，它另有判据 —— 见下面那条警告。
    pub fn frame(&mut self) -> Frame {
        self.frame_inner(None)
    }

    /// [`Self::frame`] 的带截图版：调试桥的 `shot` 命令走这里。
    ///
    /// ⚠ 截图时**强制渲染**（`redrawn || capture.is_some()`）：AI 发 `shot` 时
    /// 界面往往静止（`FrameOutcome::Unchanged`），不强制的话没有 present 前的
    /// 读回时机，拿到的就只能是上一帧的残留或全黑。
    #[cfg_attr(not(debug_assertions), allow(unused_variables))]
    fn frame_inner(&mut self, capture: Option<&mut Vec<u8>>) -> Frame {
        #[cfg(debug_assertions)]
        let frame_started = self.profiler.is_running().then(std::time::Instant::now);
        #[cfg(not(debug_assertions))]
        let frame_started: Option<std::time::Instant> = None;
        let update_started = frame_started;
        self.sync_input();
        let theme = self.theme;
        let size = self.size();
        // 作用域要短：`ctx` 借着 `self.engine`，而下面要可变借用 `self` 去回放。
        // 把树的构建、图集上传全部放进这个作用域，ctx 在离开时释放。
        let (stats, damage, redrawn) = {
            // 挂上内置图标集：矢量图标走完整的「图标名 → 展平 → 三角化 →
            // `Command::Path`」链条。理由与 `zephyra-winit-d3d12` 那份一致。
            let ctx =
                Context::with_engine(theme, &self.engine).with_icons(zephyra_core::icon::builtin());

            let element = self.view.view(&ctx);

            // 1. 声明 → 保留 → （必要时）布局 → 降级，全部在核心里完成。
            let outcome = self
                .cache
                .frame(&element, ctx, (size.0 as f32, size.1 as f32));

            // ⚠️ 图集上传的判据是 `atlas_revision()`，**不是**「本帧有没有重绘」。
            //
            // 这两件事不等价，而把它们混为一谈会出一个极难查的 bug：字形图集满时的
            // **淘汰重排**（`zephyra_core::text::atlas` 的 `make_room`）发生在 layout 的
            // measure 阶段，那时 `FrameCache` 可能判定「无节点 paint-dirty」而返回
            // `Unchanged` —— 于是图集这次没上传，而 GPU 还抱着**重排前**的纹理。
            //
            // 症状是「文字忽然整片消失，或每个字都跑到别的位置上」，且**只在图集
            // 填满之后出现**，日志里一切正常。所以这里无条件检查，与 `outcome` 无关。
            //
            // 判据不能换成「已缓存字形数」：图集加了 LRU 淘汰之后，「数量不变 ⇒ 内容
            // 不变」这条**不成立** —— 淘汰一个再放进一个，数量不变而纹理已整体重排。
            let revision_now = self.engine.borrow().atlas_revision();
            if revision_now != self.atlas_revision_uploaded {
                let gpu = lock_gpu(&self.gpu);
                let uploaded = self
                    .renderer
                    .upload_atlas(&gpu.gpu.context, self.engine.borrow().atlas());
                drop(gpu);
                if uploaded.is_err() {
                    eprintln!("[zephyra] 图集上传失败，本帧文字可能不显示");
                }
                self.atlas_revision_uploaded = revision_now;
            }
            // ⚠️ 在作用域**内**就把 `FrameOutcome` 拆成 `Copy` 的标量。
            //
            // `FrameOutcome::Updated` 借着一份 `&DisplayList`，而下面
            // `acquire_and_render` 要 `&mut self` —— 直接把 `outcome` 带出作用域会
            // 让借用活到函数末尾，编译期就撞上（`cannot borrow *self as mutable more
            // than once`）。这里提前解构，列表留到回放时用
            // `self.cache.list()` 现取。
            match outcome {
                FrameOutcome::Unchanged => (DiffStats::default(), Damage::Full, false),
                FrameOutcome::Updated { stats, damage, .. } => (stats, damage, true),
            }
        };
        let update_us = update_started
            .map(|started| started.elapsed().as_micros() as u64)
            .unwrap_or_default();

        // 2. DisplayList → 像素。ctx 已出作用域，`self` 可变借用恢复。
        //
        // ⚠ `Unchanged` 时**连 present 都不能做**：本项目两个后端都是 flip model，
        // `Present` 之后后缓冲内容未定义。不画就 present 会把未定义的内容推上屏。
        // 什么都不做才是对的 —— 合成器继续显示上一帧。
        let render_started = frame_started.map(|_| std::time::Instant::now());
        let painted = redrawn || capture.is_some();
        let report = if painted {
            self.acquire_and_render_inner(true, Damage::Full, capture)
        } else {
            RenderReport::default()
        };
        if painted {
            // 渲染过的帧数（`wait_frames` 按它算）——本后端每帧都是整帧。
            self.rendered_frames += 1;
        }
        let render_submit_us = render_started
            .map(|started| started.elapsed().as_micros() as u64)
            .unwrap_or_default();

        // 3. 清事件标志。脏标记由 `FrameCache` 内部清（它知道什么时候该清）。
        self.dirty = false;

        // 4. 无障碍：发布快照 + 施加 UIA 侧排队的动作。
        //
        // 必须在 `sync_input` **之后**（本函数开头那次）—— 见 [`Self::pump_uia`]。
        // 也必须在 `self.dirty = false` **之后**：它要标脏，而那行会清掉标脏。
        self.pump_uia();

        // 桥的 `stats` 答案在这里刷新——`Frame` 的消费者（trace 打印）与本字段
        // 同源同帧，不会出现「stats 落后一帧」。
        //
        // ⚠ `damage` 也在里面：本后端**每帧整帧**（D3D11 路径没有持久 MSAA
        // 目标，脏区外的上一帧内容并不保留），所以这里恒为 `Full`。
        // D3D12 侧可能是局部矩形 —— 那一侧的差别就是「脏区外的内容还在不在」。
        // 「输入 → 帧就绪」的耗时（测应用这一段，不含 DWM 合成）。
        self.input_latency_us = self.input_at.map(|at| at.elapsed().as_micros() as u64);
        self.input_at = None;

        #[cfg(debug_assertions)]
        {
            let new_stats = zephyra_fx::bridge::FrameStats {
                quads: report.quads,
                skipped: report.skipped,
                dirtied: stats.dirtied,
                needs_layout: stats.needs_layout,
                redrawn,
                hovered: self.hovered(),
                damage: Some(zephyra_core::frame::Damage::Full),
                // 分段耗时：core 侧量的（对账 / 样式 / 脏区扫描 / 布局 / 绘制）。
                // 「这一帧慢在哪」从此是**读出来的**，不用改代码加临时日志。
                stages: Some(self.cache.timings()),
                // 已渲染帧数：客户端靠它算 fps（两次采样相减 / 时间差）。
                frames: self.rendered_frames,
                // 「输入 → 帧就绪」的耗时（本后端每帧都画，所以这个数一直有）。
                input_latency_us: self.input_latency_us,
                render_submit_us: None,
                // 离屏趟数 = 这一帧 GPU 干了多少活的**主指标**（一趟 ≈ 四趟全屏光栅化）。
                offscreen_passes: report.offscreen_passes,
                backdrop_blur_passes: report.backdrop_blur_passes,
                lens_passes: report.lens_passes,
                refraction_passes: report.refraction_passes,
            };
            // ⚠ **跳过的帧不覆盖「上一次画出来的东西」**（同 d3d12 侧，
            // 规则见 `zephyra_fx::bridge::stats_after_skipped`）。
            self.last_stats = if painted {
                new_stats
            } else {
                zephyra_fx::bridge::stats_after_skipped(&self.last_stats, &new_stats)
            };
        }

        // ⚠ **只记真的重画了的帧** —— 同 `zephyra-winit-d3d12` 的理由：
        // 不记「跑过」的帧，否则 fps 会被空转的 tick 撑成假数字。
        //
        // 判据用 `painted`（`redrawn || capture.is_some()`）：截图也算
        // 一次真的重画。
        #[cfg(debug_assertions)]
        if painted {
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

        Frame {
            diff: stats,
            render: report,
            hovered: self.hovered(),
            damage,
            redrawn,
        }
    }

    fn sync_input(&mut self) {
        // 长按是**时钟驱动**的：手指不动时没有任何输入事件，所以只能每帧查一次。
        //
        // 官方指南把长按列为静态手势之一（并明确它是「计时交互」的正当用例）。
        // 没有这一行，长按就永远不会触发 —— 而长按恰恰是「手指一动不动」的场景。
        if let Some((kind, target)) = self.router.tick(self.pointer_translator.now_ms()) {
            self.pending_gestures
                .push(zephyra_fx::GestureEvent { kind, target });
            self.dirty = true;
        }
        let size = self.size();
        let now = std::time::Instant::now();
        let dt_ms = self
            .last_frame_at
            .map(|t| now.duration_since(t).as_secs_f32() * 1000.0)
            .unwrap_or(0.0);
        self.last_frame_at = Some(now);
        // ⚠ 这一批指针事件里有没有**取消**（系统抢走了手势）。
        // 应用要靠它**不**把「按下 → 松开」当成点击 —— 用户的手指还在屏幕上。
        let cancelled = zephyra_fx::has_cancelled(&self.pending_pointer_events);
        // P1.2：布局后的 key → 矩形快照。
        let mut rects = Vec::new();
        if let Some(node) = self.cache.node() {
            node.collect_keyed_rects(&mut rects);
        }
        self.view.on_input(&InputState {
            rects,
            hovered: self.router.hovered().cloned(),
            // 键盘激活复用指针的 pressed 通道：指针优先，键盘闩作后备。于是
            // Space 按住时模板出按下态、抬起时归空——与指针按下完全同构，应用
            // 既有的「上帧按着→这帧松了 = 点击」重建逻辑自动为键盘触发。
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
            // 行为焦点：鼠标点击设的焦点也在这里（文本输入要送到它）。
            active_focus: self.focus.focused().cloned(),
            pointers: self.router.pointer_count(),
            text_events: std::mem::take(&mut self.pending_text_events),
            scroll: self.pending_scroll.take(),
            pointer_events: std::mem::take(&mut self.pending_pointer_events),
            cancelled,
            gestures: std::mem::take(&mut self.pending_gestures),
            manipulations: std::mem::take(&mut self.pending_manipulations),
            inertia: self.pending_inertia.take(),
            viewport: (size.0 as f32, size.1 as f32),
            scale: self.scale_factor(),
            dt_ms,
            // This host has no platform integration crate to query language preferences.
            languages: Vec::new(),
        });
        if let Some(url) = self.view.take_pending_url() {
            open_external_url(url);
        }
    }

    /// 取本帧渲染目标并回放 `DisplayList`。
    /// 取本帧渲染目标并回放 `DisplayList`。
    ///
    /// # 「只重画脏区」的前提：脏区之外的上一帧画面**还在渲染目标里**
    ///
    /// flip model 的后缓冲在 `Present` 之后**内容未定义** —— 它是会被丢弃的
    /// 临时目标，不是「上一帧那张图」。本后端的渲染目标**就是**那个后缓冲
    /// （没有 MSAA 中间纹理），所以按脏区绘制等于把未定义像素留在屏幕上：
    /// 症状是「点一下之后界面花成一整片底色，只有那一小块是对的」。
    ///
    /// 因此这里强制整帧，**忽略**调用方给的 `_damage`。代价写在明处：交互帧
    /// 整帧重绘；换来的是画面永远对。（D3D12 后端有 MSAA 中间纹理，
    /// 那条路上按脏区绘制才成立，见它的 `render_pass`。）
    ///
    /// `capture`：调试桥的截图——Some 时渲完后、present 前把后缓冲读回
    /// BMP 存进去（时机依据见函数体内 `maybe_dump_frame` 那段实测表）。
    fn acquire_and_render_inner(
        &mut self,
        allow_dump: bool,
        _damage: Damage,
        capture: Option<&mut Vec<u8>>,
    ) -> RenderReport {
        // 拆开借用：`Renderer::render` 要 `&mut self.renderer`，而取 RTV 要
        // `&self.surface`。整体借用 `self` 会在第一个就撞上借用检查器。
        let rtv = match self.surface.current_rtv() {
            Ok(v) => v.clone(),
            Err(e) => {
                eprintln!("[zephyra] 取不到后缓冲: {e}");
                return RenderReport::default();
            }
        };
        let clear = clear_color(self.transparent, self.theme.surface);
        let gpu = lock_gpu(&self.gpu);
        let report = self.renderer.render(RenderCtx {
            context: &gpu.gpu.context,
            device: &gpu.gpu.device,
            rtv: &rtv,
            list: self.cache.list(),
            damage: Damage::Full,
            size: self.surface.size(),
            clear,
            textures: &self.textures,
        });
        drop(gpu);

        // 诊断截帧必须在 `present` **之前**，且由 `read_pixels` 用
        // `current_rtv()` 反查资源 —— 两个条件缺一不可。
        //
        // 实测三种组合（Intel UHD + flip model，只探到 1 个可用后缓冲）：
        //
        // | 何时读 | 按什么取 | 结果 |
        // |---|---|---|
        // | present **前** | `current_rtv()` 反查 | **完整一帧** ✅ |
        // | present **前** | `GetCurrentBackBufferIndex` | 撕裂：左半有内容、右半全黑 |
        // | present **后** | 新索引 | 全黑 |
        //
        // 两次误判的教训是同一句：**D3D11 的失败模式是"不报错但内容不对"，所以
        // 唯一可靠的判据是看像素**。当时日志里一切正常（`quads=143`、零 HRESULT）。
        if allow_dump {
            #[cfg(debug_assertions)]
            self.maybe_dump_frame();
        }

        // 调试桥截图：与 `maybe_dump_frame` 同一时机（present 前）——理由同源：
        // flip model 下 present 之后读回的全零/未定义。`read_pixels` 内部自己
        // 解析 MSAA（见 `maybe_dump_frame` 那段注释），与 resolve 顺序不冲突。
        if let Some(out) = capture {
            match self.surface.read_pixels() {
                Ok(px) => *out = zephyra_fx::pixels::bmp_bytes(&px, true),
                Err(e) => eprintln!("[zephyra] 调试桥截图读回失败: {e}"),
            }
        }

        // ⚠ MSAA 解析必须在 present **之前**。
        //
        // 画进 MSAA 纹理的内容不会出现在后缓冲上 —— 后缓冲拿到的是上一次
        // `ResolveSubresource` 的结果。漏掉这步的症状是「画面卡在旧内容」
        // 或「全黑」，且**零 HRESULT 错误**。
        //
        // 位置也在 `maybe_dump_frame` **之后**：截图走 `read_pixels`，而它内部
        // 已经自己解析了一次（见 `Surface::read_pixels`）。两次解析幂等（后者读
        // 同一份已解析内容），但让截图与屏幕**同源**比省一次 GPU 操作重要。
        if let Err(e) = self.surface.resolve_msaa() {
            eprintln!("[zephyra] MSAA 解析失败: {e}");
            return RenderReport::default();
        }

        // 提交失败不能静默：丢掉一帧还能继续，丢掉整帧的呈现会让窗口停住更新，
        // 而那与"UI 卡住了"是同一种症状 —— 区别只有日志里有。
        if let Err(e) = self.surface.present(1) {
            eprintln!("[zephyra] present 失败: {e}");
            return RenderReport::default();
        }
        report
    }

    /// 记下「输入刚刚到达」。
    ///
    /// 用来测「输入 → 帧就绪」的延迟。
    fn mark_input(&mut self) {
        self.input_at = Some(std::time::Instant::now());
    }

    /// 处理一次指针移动：命中测试 → 路由 → 是否需要重绘。
    pub fn on_cursor_moved(&mut self, x: f32, y: f32) -> bool {
        self.mark_input();
        self.pointer.get_or_insert(0);
        self.pointer_pos = (x, y);
        let ev = self.pointer_translator.mouse_move(x, y);
        self.pending_pointer_events.push(zephyra_fx::PointerInput {
            id: ev.id,
            x,
            y,
            phase: zephyra_fx::PointerInputPhase::Move,
            source: ev.source,
        });
        let Some(node) = self.cache.node() else {
            return false;
        };
        let theme = self.theme;
        let before = self.router.revision();
        let mut out = RoutedEvents::new();
        self.router
            .on_pointer(node, ev, Context::new(theme), &mut out);
        let changed = self.router.revision() != before;
        if changed {
            self.dirty = true;
        }
        self.collect_routed(&out);
        changed
    }

    /// 处理一次按下：返回是否产生了路由事件。
    pub fn on_pointer_down(&mut self) -> bool {
        self.mark_input();
        self.pointer.get_or_insert(0);
        let (x, y) = self.pointer_pos;
        let ev = self.pointer_translator.mouse_button(true, x, y);
        self.pending_pointer_events.push(zephyra_fx::PointerInput {
            id: ev.id,
            x,
            y,
            phase: zephyra_fx::PointerInputPhase::Down,
            source: ev.source,
        });
        let Some(node) = self.cache.node() else {
            return false;
        };
        let theme = self.theme;
        let mut out = RoutedEvents::new();
        self.router
            .on_pointer(node, ev, Context::new(theme), &mut out);
        let any = !out.is_empty();
        if any {
            self.dirty = true;
        }
        self.collect_routed(&out);
        any
    }

    /// 处理一次抬起：返回是否产生了路由事件。
    ///
    /// 从 `window_event` 的 Released 分支提取成方法：调试桥的点击注入
    /// （`inject_click`）要走**同一条**路径——合成事件与物理事件分叉的话，
    /// 「AI 点了但没反应」这类问题就永远查不清是哪条路的锅。
    pub fn on_pointer_up(&mut self) -> bool {
        self.mark_input();
        self.pointer.unwrap_or(0);
        let (x, y) = self.pointer_pos;
        let ev = self.pointer_translator.mouse_button(false, x, y);
        self.pending_pointer_events.push(zephyra_fx::PointerInput {
            id: ev.id,
            x,
            y,
            phase: zephyra_fx::PointerInputPhase::Up,
            source: ev.source,
        });
        let Some(node) = self.cache.node() else {
            return false;
        };
        let theme = self.theme;
        let mut out = RoutedEvents::new();
        self.router
            .on_pointer(node, ev, Context::new(theme), &mut out);
        // 点击设焦点：`RoutedEvent::Click` 是「按下 + 抬起在同一节点」。
        // 焦点设给被点到的那个 key —— 文本框点进去获得键盘焦点。
        let click_target: Option<zephyra_core::id::ElementKey> = out.iter().find_map(|e| match e {
            RoutedEvent::Click(key) => Some(key.clone()),
            _ => None,
        });
        if let Some(target) = click_target {
            if let Some(node) = self.cache.node_mut() {
                self.focus.focus(node, Some(&target), Context::new(theme));
                self.dirty = true;
            }
        }
        self.collect_routed(&out);
        !out.is_empty()
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
    /// - **多指并存**：每根手指一个 id；
    /// - **双状态**：触控**不产生悬停**（由 core 的路由器判）；
    /// - **取消 ≠ 抬起**：系统接管手势时收到的是取消（core 清按下态、
    ///   不提交点击）。
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
        // 触控也要更新「指针当前位置」—— IME 插入点用它当代理。
        self.pointer_pos = (x, y);
        let Some(node) = self.cache.node() else {
            return false;
        };
        let theme = self.theme;
        let before = self.router.revision();
        let mut out = RoutedEvents::new();
        self.router
            .on_pointer(node, ev, Context::new(theme), &mut out);
        let changed = self.router.revision() != before;
        if changed {
            self.dirty = true;
        }
        self.collect_routed(&out);
        changed
    }

    /// 把路由结果里的**静态手势 / 操作增量**收进待发缓冲。
    ///
    /// 它们是「给应用的增量信息」，不是「宿主要据此决定重绘」的东西 ——
    /// 所以不进 `revision`。
    fn collect_routed(&mut self, out: &RoutedEvents) {
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
    }

    /// 按路径取节点（子下标链）——桥的 `props` 命令用。
    #[cfg(debug_assertions)]
    fn bridge_node(&self, path: &[usize]) -> Option<&zephyra_core::node::Node> {
        zephyra_fx::bridge::node_at_path(self.cache.node()?, path)
    }

    /// 诊断：把某一帧的后缓冲写成 BMP（`ZEPHYRA_FRAME_DUMP=<路径>` 时启用）。
    ///
    /// **必须在 `present` 之前调用。** flip model 下 `Present` 一执行，DXGI 就
    /// 可以回收/换掉那张后缓冲；之后再 `CopyResource` 拿到的是**另一张**（通常
    /// 是刚被清空的、或还没被画的）—— 读出来全零，而画面其实是对的。
    ///
    /// 这就是为什么它挂在 [`Self::acquire_and_render`] 内部（渲染完、提交前）
    /// 而不是 `window_event` 里。我第一版挂在 `window_event` 里，读回的
    /// STAGING 纹理**全零**，而那正是这个原因 —— 差点误判成"渲染器什么都没画"。
    ///
    /// # 为什么需要它
    ///
    /// 本项目的渲染失败模式**不是崩溃，而是"不崩但画面不对"**：着色器没绑上、
    /// 顶点写歪了、混合状态错了 —— 症状全都是"一片纯色"或"全黑"，而
    /// `RenderReport` 里 `quads=143` 一切正常。
    ///
    /// "进程没崩"与"画对了"是两件事，而后者只能靠看像素证明。这个开关把后缓冲
    /// 落到文件上，于是"画对了"变成可复核的。
    ///
    /// 写到 `ZEPHYRA_FRAME_DUMP` 给出的路径；`-` 表示只统计颜色数不写文件。
    #[cfg(debug_assertions)]
    fn maybe_dump_frame(&self) {
        // 调试层消息**每帧都要拉** —— 与是否 dump 像素无关。
        //
        // 早先把这一句放在 `read_pixels` 成功分支之后，于是单独打开
        // ZEPHYRA_D3D_DEBUG=1 时**一条消息都没有**：而 D3D11 的问题从不以 HRESULT
        // 形式出现，只会进消息队列。所以「开了调试层却没输出」看起来像「没有消息」，
        // 实际上是「根本没去拉」。
        self.surface.dump_debug_messages();

        static ONCE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        let path = ONCE.get_or_init(|| std::env::var("ZEPHYRA_FRAME_DUMP").ok());
        let Some(path) = path.as_deref() else {
            return;
        };
        // 前几帧每次都报告（颜色数是关键判据），之后只写文件不报告。
        //
        // 帧号存在环境变量里：不同问题要看的帧不同（首帧看初始化，稳态看管线）。
        //
        // 注意：**稳态可能只有 1~2 帧**。`frame()` 只在 `dirty` 时被调用，而
        // 一帧跑完 `dirty` 立刻归零 —— 所以静止的 demo 总共只渲染那么几帧。
        // 指望"第 30 帧"是等不到的（我第一版就写了这个，结果永远不触发）。
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        const REPORT_UPTO: u32 = 4;
        if n > REPORT_UPTO {
            return;
        }
        match self.surface.read_pixels() {
            Ok(px) => {
                // 不同颜色数是"画没画出东西"最直接的判据：全屏纯色（无论黑白）
                // 都意味着什么都没画上。
                let colors = px.distinct_colors(2);
                // 顺便报几个采样点的原始字节值 —— 出问题时"全黑"与"应该是深灰"
                // 的区别就在这几十个字节里，不必去看图。
                let samples: Vec<String> = [2usize, 40, 300]
                    .iter()
                    .filter_map(|y| {
                        let row = px.row(*y as u32);
                        let i = (px.width as usize / 2) * 4;
                        row.get(i..i + 4)
                            .map(|p| format!("y={y}: BGRA=({},{},{},{})", p[0], p[1], p[2], p[3]))
                    })
                    .collect();
                if path != "-" && n == 0 {
                    if let Err(e) = write_bmp(path, &px, true) {
                        eprintln!("[zephyra] 写 {path} 失败: {e}");
                        return;
                    }
                }
                // 尺寸不一致必须**显式**说出来：症状是画面右侧/底部一整条黑边，
                // 而其它一切（HRESULT、RenderReport）都正常。不说的话，看图的人
                // 只会以为"边框就这么宽"。
                // 尺寸不一致必须**显式**说出来：症状是画面右侧/底部一整条黑边，
                // 而其它一切（HRESULT、RenderReport）都正常。
                let size_note = if px.size_matches() {
                    String::new()
                } else {
                    format!(
                        "；⚠ surface {}x{} 而后缓冲 {}x{}",
                        px.declared_width, px.declared_height, px.width, px.height,
                    )
                };
                // 窗口客户区尺寸也打出来：它是\"交换链该有多大\"的唯一权威，
                // 而 surface 里的 size 是我们自己记的 —— 两者不一致就说明
                // 某条 resize 路径漏掉了。
                let window_size = self.window.inner_size();
                // 清屏色在图上的样子（线性 0.063 经 sRGB 编码约 0x2A=42/43/45）
                // 用来定位\"哪些区域真的被清过\"。
                let clear_ref = srgb_bytes(self.theme.surface);
                let clear_str = format!("{},{},{}", clear_ref[0], clear_ref[1], clear_ref[2]);
                eprintln!(
                    "[zephyra] 第 {n} 帧 后缓冲 {}x{} 行距 {} surface {}x{} 窗口 {}x{} \
                     不同颜色 {colors} 种{size_note}；清屏≈{clear_str}；{}",
                    px.width,
                    px.height,
                    px.row_pitch,
                    px.declared_width,
                    px.declared_height,
                    window_size.width,
                    window_size.height,
                    samples.join("  "),
                );
            }
            Err(e) => eprintln!("[zephyra] 读回像素失败: {e}"),
        }
        // `ZEPHYRA_D3D_DEBUG=1`（建了带调试层的设备）时把驱动自己的话打出来。
        //
        // 放在这里而不是只在出错时打：D3D11 的问题**从不以 HRESULT 形式出现**，
        // 逐帧打才看得出"哪一帧开始不再报"。
        self.surface.dump_debug_messages();
    }

    /// 让呈现路径按当前 `surface` 尺寸重新配置。
    ///
    /// 只有"宿主在**没有**收到 `Resized` 的情况下改了尺寸"才需要它 —— 例如宿主
    /// 自己接管了尺寸来源。走 winit 事件循环时 `Resized` / `ScaleFactorChanged`
    /// 已经直接调 `surface.resize`，所以本方法通常用不上。
    ///
    /// 之所以留着而不是删掉：它的存在让"尺寸变更只有一个入口"这件事显式化 ——
    /// `Surface::resize` 幂等（尺寸相同就提前返回），多一个调用点不会出错。
    pub fn reconfigure_present(&mut self) {
        let (w, h) = self.surface.size();
        self.surface.resize(w, h);
    }
}

/// 调试桥的执行面。见 `zephyra_fx::bridge` 的模块文档。
///
/// # 注入走的是物理输入同一条路
///
/// `inject_hover` / `inject_click` 直接调 [`HostWindow::on_cursor_moved`] /
/// [`HostWindow::on_pointer_down`] / [`HostWindow::on_pointer_up`]——合成事件
/// 与物理事件没有任何分叉。这不是洁癖：命中测试、事件路由、脏标记的传播只有
/// 一份实现，「AI 点了没反应」才不会多出一整类「物理点得动、合成点不动」的
/// 查询黑洞。
#[cfg(debug_assertions)]
impl<V: View> zephyra_fx::bridge::BridgeHost for HostWindow<V> {
    fn viewport(&self) -> (f32, f32) {
        let (w, h) = self.size();
        (w as f32, h as f32)
    }

    fn tree_dump(&self) -> String {
        match self.cache.node() {
            Some(node) => zephyra_fx::bridge::tree_dump_of(node),
            None => "(还没有布局树——首帧尚未运行)\n".into(),
        }
    }

    fn props_dump(&self, path: &[usize]) -> Option<String> {
        self.bridge_node(path)
            .map(zephyra_fx::bridge::props_dump_of)
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
    }

    fn inject_click(&mut self, x: f32, y: f32) {
        // Keep down and up in separate view updates so pressed-state click handlers
        // observe both edges even when the bridge polls only once for this command.
        self.on_cursor_moved(x, y);
        self.on_pointer_down();
        self.sync_input();
        self.on_pointer_up();
        self.sync_input();
        self.dirty = true;
    }

    fn inject_pointer_down(&mut self) {
        self.on_pointer_down();
        self.bridge_pointer_pressed = true;
        self.dirty = true;
    }

    fn inject_pointer_up(&mut self) {
        self.on_pointer_up();
        self.bridge_pointer_pressed = false;
        self.dirty = true;
    }

    /// MCP server 在按下与抬起之间消失（超时/崩溃/会话取消）：补一次抬起，
    /// 否则应用永远卡在拖动态，后续所有真实输入都被当成拖动的一部分。
    fn on_bridge_disconnect(&mut self) {
        if self.bridge_pointer_pressed {
            self.on_pointer_up();
            self.bridge_pointer_pressed = false;
            self.dirty = true;
        }
    }

    fn profile_start(&mut self, duration_seconds: u64) -> Result<(), String> {
        self.profiler.start(duration_seconds)?;
        self.dirty = true;
        Ok(())
    }

    fn profile_report(&mut self) -> Result<String, String> {
        self.profiler.report()
    }

    fn inject_key(&mut self, key: &str) {
        // 名字 → 事件那一步在共用 crate 里：三后端注入的按键行为必须一致。
        self.pending_text_events
            .extend(zephyra_winit_input::injected_events(key));
        self.dirty = true;
    }

    fn inject_text(&mut self, text: &str) {
        // 逐字符**插入**事件（不是按键）：见 `zephyra_winit_input::injected_text_events`
        // 的说明——多发键边沿会让某些控件把每次输入当成一次提交。
        self.pending_text_events
            .extend(zephyra_winit_input::injected_text_events(text));
        self.dirty = true;
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
    }

    /// 注入一次**触摸**（多点触控的主入口）。
    ///
    /// # 为什么走**同一条**路
    ///
    /// 真实触摸经 `WindowEvent::Touch` → [`Self::on_touch`]，注入的触摸也走
    /// [`Self::on_touch`] —— 两条路分叉的话，「注入能过、真机不行」这类问题
    /// 就永远查不清是哪条路的锅。
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
        Ok(())
    }

    fn screenshot(&mut self, _opts: zephyra_fx::bridge::ShotOptions) -> Option<Vec<u8>> {
        // `frame_inner(Some)` 强制渲一帧并在 present 前读回（见它的文档）——
        // 不依赖「界面正好在动」。读回失败（空缓冲）返回 None，桥回 ERR。
        //
        // ⚠ 本后端**每帧整帧**（没有持久 MSAA 目标，脏区外的上一帧内容并不保留），
        // 所以 `repaint` 在这里是空操作：截出来的本来就是整帧。
        let mut buf = Vec::new();
        let _ = self.frame_inner(Some(&mut buf));
        if buf.is_empty() {
            None
        } else {
            Some(buf)
        }
    }
}

impl<V: View + 'static> WindowHost for HostWindow<V> {
    fn key(&self) -> WindowKey {
        self.window_key
    }

    fn drain_posted(&mut self) -> bool {
        let Some(handle) = &self.handle else {
            return false;
        };
        let drained = handle.waker().drain();
        if drained.wakes.is_empty() && drained.messages.is_empty() {
            return false;
        }
        for token in drained.wakes {
            self.view.on_wake(token);
        }
        for mut msg in drained.messages {
            self.view.on_message(&mut *msg);
        }
        self.dirty = true;
        true
    }

    fn winit_id(&self) -> Option<WindowId> {
        Some(self.window.id())
    }

    fn handle_event(&mut self, event: WindowEvent) -> HostEventOutcome {
        // 窗口遮挡/最小化感知：必须在 match 之前处理，避免 Ime 等非 Copy 类型导致部分移动。
        if let WindowEvent::Occluded(occluded) = event {
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
            WindowEvent::CloseRequested => return HostEventOutcome::Close,
            WindowEvent::Resized(size) => {
                let (w, h) = (size.width.max(1), size.height.max(1));
                self.surface.resize(w, h);
                self.dirty = true;
            }
            WindowEvent::RedrawRequested => {
                // 帧在 `tick` 里跑（`WindowSet` 每轮都会调它）。这里只标脏。
                self.dirty = true;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mark_input();
                // `revision` 只在 hover/pressed **状态切换**时递增——拖拽中 hover 不变，
                // `on_cursor_moved` 返回 false。不标脏，移动事件堆在队列里没人消费，
                // 视图永远看不到 Move（慢速拖拽「卡住」）。
                let _ = self.on_cursor_moved(position.x as f32, position.y as f32);
                self.dirty = true;
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if self.on_pointer_down() {
                    self.dirty = true;
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                // 抬起必须成对处理：否则按下捕获会残留，下一次点击会被判为抬起。
                if self.on_pointer_up() {
                    self.dirty = true;
                }
            }
            WindowEvent::Touch(touch) => {
                // ⚠ 触控**必须**接：winit 在 Windows 上 `RegisterTouchWindow`，
                // 那会关掉「触摸提升为鼠标」—— 不接的话触摸事件被整个丢掉，
                // 触控屏上点不动任何东西。
                let _ = self.on_touch(
                    touch.phase,
                    touch.id,
                    touch.location.x as f32,
                    touch.location.y as f32,
                );
                self.dirty = true;
            }
            WindowEvent::ModifiersChanged(m) => {
                self.translator.on_modifiers(m.state());
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.translator
                    .key_events(&event, &mut self.pending_text_events);
                // 键盘激活（Space/Enter）**当帧**生效，与下面的 Tab 同理：用户按
                // 空格期望立刻看到按下态，攒到下一帧 sync_input 会慢一拍。核心状态机
                // 自己判「这个键是不是激活键、焦点节点可不可激活」，宿主只把键边沿递
                // 过去——于是「Space/Enter 触发点击」的语义全仓只有一份（core）。
                {
                    let kp = zephyra_winit_input::KeyParts::from(&event);
                    let key_input = self.translator.key(kp);
                    let focused = self.focus.focused().cloned();
                    let node = self.cache.node();
                    let theme = self.theme;
                    if let Some(true) = self.activation.on_key(
                        &key_input,
                        node,
                        focused.as_ref(),
                        Context::new(theme),
                    ) {
                        self.dirty = true;
                    }
                }
                if event.state == ElementState::Pressed {
                    // Tab 移焦点是**当帧**就该生效的（不像字符/命令要等下一帧
                    // sync_input 才进 InputState）：用户按 Tab 期望立刻看到焦点环
                    // 跳到下一个控件。
                    if let Key::Named(NamedKey::Tab) = event.logical_key {
                        let shift = self.translator.modifiers().shift();
                        if let Some(node) = self.cache.node_mut() {
                            let theme = self.theme;
                            self.focus.tab(node, shift, Context::new(theme));
                            self.dirty = true;
                        }
                    }
                    match event.logical_key.clone() {
                        // 单窗口路径下由 `ApplicationHandler` 把它翻成
                        // `event_loop.exit()`；多窗口路径下只关这一个窗口。
                        Key::Named(NamedKey::Escape) => return HostEventOutcome::Close,
                        Key::Character(c) if c.as_str() == "t" => {
                            // 演示「切主题不碰树」：只换一个 &Theme，树完全不动。
                            let next =
                                if self.theme.variant == zephyra_core::theme::ThemeVariant::Dark {
                                    Theme::light()
                                } else {
                                    Theme::dark()
                                };
                            self.set_theme(next);
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::Ime(ime) => {
                // 候选窗贴焦点文本框的光标下沿，不再跟鼠标跑。
                // ⚠ 必须告诉 winit 光标区域，否则 OS 候选窗默认跑到屏幕左下角。
                let caret = self.ime_caret();
                self.window.set_ime_cursor_area(
                    winit::dpi::PhysicalPosition::new(caret.0, caret.1),
                    winit::dpi::PhysicalSize::new(2u32, 18u32),
                );
                self.translator
                    .ime(&ime, caret, &mut self.pending_text_events);
                self.dirty = true;
            }
            WindowEvent::Focused(false) => {
                // 窗口失焦：焦点必须清空（否则回来后旧焦点还亮着，且按键全落到
                // 一个看不见的节点上）。`clear` 会写回 FOCUSED=false 到旧节点。
                // 激活闩也要清（与 WinUI `OnLostFocus` 清 `m_bIsSpaceOrEnterKeyDown`
                // 一致）：否则失焦时若 Space 正按着，`held` 一直指着旧节点。
                if let Some(node) = self.cache.node_mut() {
                    self.focus.clear(Some(node));
                    self.dirty = true;
                }
                self.activation.clear();
            }
            WindowEvent::MouseWheel { delta, .. } => {
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
            WindowEvent::ScaleFactorChanged { .. } => {
                let s = self.window.inner_size();
                self.surface.resize(s.width.max(1), s.height.max(1));
                self.dirty = true;
            }
            _ => {}
        }
        HostEventOutcome::Continue
    }

    fn tick(&mut self) -> HostTick {
        // 桥的 poll 放在帧首：注入的 hover/click 在**本帧**就参与命中
        // 测试与布局，AI 读回的树永远是操控后的状态。
        //
        // `take` 出来再调：`poll` 要 `&mut BridgeHost`（= self）而桥自己
        // 也是 self 的字段——借检查器解不开这个环。桥没有 Drop 语义
        // （监听 socket 移动即可），take-放回是安全的。
        #[cfg(debug_assertions)]
        {
            if let Some(mut b) = self.bridge.take() {
                b.poll(self);
                self.bridge = Some(b);
            }
            // dev 资源热重载：与桥一样每帧驱动，但只在 debug 构建里存在。
            if let Some(reloader) = self.dev_reloader.as_mut() {
                reloader.poll();
            }
        }

        // ⚠ 窗口不可见（最小化 / 完全遮挡）时直接返回空帧：不 layout、不 paint、不 present。
        // 这让 CPU/GPU 在后台/遮挡时归零，符合 P3 验收标准。
        if !self.window_visible {
            let tick = HostTick {
                wants_poll: true,
                busy: false,
                ..Default::default()
            };
            return tick;
        }

        let mut tick = HostTick::default();

        // —— VSync 帧 pacing：距上次呈现不足一个刷新周期时，这一拍**不画**，睡到点再说。
        // 与 D3D12 侧同一契约：flip model 下不封顶会把 GPU 占用推高，且优化看起来没生效。
        if !self.dirty && self.last_present_at.is_some() {
            if let Some(next) = self
                .last_present_at
                .map(|last| last + self.present_interval)
                .filter(|next| std::time::Instant::now() < *next)
            {
                tick.wake_at = Some(next);
                return tick;
            }
        }

        // 「动画播放中」由视图回答：保留式 UI 不常驻重画，动画只在事件到来时
        // 推进一帧，然后循环睡死 —— 中间整段弹簧曲线看不到。
        let animating = self.view.wants_frames();
        if animating {
            self.dirty = true;
        }

        // ⚠ 是否**真的**要画由 core 判定（`Frame::redrawn`）：事件可能什么都没改
        // （鼠标在空白处移动 ⇒ `hovered` 前后都是 `None`），此时 core 返回
        // `Unchanged`，本帧不画。
        //
        // ⚠ 那时**连 present 都不能做**：本后端是 flip model，`Present` 之后后缓冲内容未定义。
        // 不画就 present 会把未定义内容推上屏（通常是黑，或上一次的帧）。
        #[cfg(debug_assertions)]
        let wants_frame = self.dirty || self.profiler.is_running();
        #[cfg(not(debug_assertions))]
        let wants_frame = self.dirty;
        if wants_frame {
            let f = self.frame();
            tick.painted = f.redrawn;
            if f.redrawn {
                self.last_present_at = Some(std::time::Instant::now());
            }
            #[cfg(debug_assertions)]
            if std::env::var("ZEPHYRA_TRACE").as_deref() == Ok("1") {
                eprintln!(
                    "[zephyra] quads={} skipped={} img_batches={} missing_tex={} \
                     dirty={} needs_layout={} hovered={:?}",
                    f.render.quads,
                    f.render.skipped,
                    f.render.image_batches,
                    f.render.missing_textures,
                    f.diff.dirtied,
                    f.diff.needs_layout,
                    f.hovered
                );
            }
        }

        // 桥的轮询分四档：有数据 ⇒ Poll；连着 / 拨号在途 ⇒ 10 ms（数据或连接
        // 结果都要尽快看到）；连出退避 / 旧模式空闲 ⇒ 50 ms（新连接唤不醒 Wait）。
        // `is_connecting` 让拨号在途不必等满 50 ms 才发现连上。
        #[cfg(debug_assertions)]
        if let Some(bridge) = self.bridge.as_ref() {
            if bridge.has_pending_work() {
                tick.wants_poll = true;
            } else if bridge.has_client() || bridge.is_connecting() {
                tick.wake_at =
                    Some(std::time::Instant::now() + std::time::Duration::from_millis(10));
            } else {
                tick.wake_at =
                    Some(std::time::Instant::now() + std::time::Duration::from_millis(50));
            }
        }

        // busy：有脏/动画/无障碍动作时为真。
        tick.busy = self.dirty || animating;

        // ⚠ VSync 封顶：距上次呈现不足一个刷新周期时，这一拍**不画**，睡到点再说。
        if let Some(next) = self
            .last_present_at
            .map(|last| last + self.present_interval)
            .filter(|next| std::time::Instant::now() < *next)
        {
            if tick.wake_at.is_none_or(|w| w < next) {
                tick.wake_at = Some(next);
            }
            tick.wants_poll = false;
            return tick;
        }

        tick
    }

    fn start(&mut self) -> Result<(), String> {
        self.dirty = true;
        self.sync_input();
        let frame = self.frame();
        if frame.redrawn {
            self.last_present_at = Some(std::time::Instant::now());
        }
        Ok(())
    }

    fn set_theme(&mut self, variant: zephyra_core::theme::ThemeVariant) {
        HostWindow::set_theme(self, Theme::builtin(variant));
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
        if let Some(handle) = &self.handle {
            handle.close();
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// D3D11 后端的 [`Backend`] 实现。
#[derive(Default)]
pub struct D3d11Backend;

impl D3d11Backend {
    /// 建一个后端入口（共享设备在第一个窗口创建时初始化）。
    pub fn new() -> Self {
        Self
    }
}

impl Backend for D3d11Backend {
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
        let mut host = HostWindow::new_lenient_opts(event_loop, view, opts);
        host.window_key = handle.key();
        host.handle = Some(handle);
        Ok(host)
    }
}

impl<V: View + 'static> ApplicationHandler for HostWindow<V> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.dirty = true;
        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.handle_event(event) == HostEventOutcome::Close {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let tick = self.tick();
        if tick.wants_poll || tick.busy {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

/// 用一个已构造好的窗口宿主跑事件循环。
pub fn run_with<V: View + 'static>(app: HostWindow<V>) -> Result<(), winit::error::EventLoopError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = app;
    event_loop.run_app(&mut app)
}

/// 用**已有**的 `EventLoop` 跑窗口宿主。
///
/// winit 0.30 要求 `run_app` 收到的 `EventLoop` 与建窗时那个是同一个，所以
/// [`HostWindow::new_lenient`] 的调用方必须自己持有它 —— 这时用本函数，而不是会
/// 新建 `EventLoop` 的 [`run_with`]。
pub fn run_with_on<V: View + 'static>(
    event_loop: EventLoop<()>,
    app: HostWindow<V>,
) -> Result<(), winit::error::EventLoopError> {
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = app;
    event_loop.run_app(&mut app)
}

/// 便捷构造：一个带背景、圆角与尺寸的元素。
pub fn panel(key: &'static str, bg: Color, radius: f32) -> Element {
    Element::new(NodeTag::Box)
        .with_key(key)
        .with(&props::BACKGROUND, PropertyValue::color(bg))
        .with(&props::CORNER_RADIUS, PropertyValue::float(radius))
}

/// 本帧的清屏色。**纯函数**：透明窗口清成 alpha=0，否则用主题底色。
///
/// # 为什么是纯函数
///
/// 「透明 ⇒ alpha=0」是本轮唯一一条 `--dump-cpu` 看不见的判据（CPU 路不呈现
/// 窗口），而它在 D3D11 / D3D12 两个后端各有一份。抽出来就能在没有 GPU 的机器
/// 上断言，免得它被顺手改回不透明 —— 那种退化的症状只是「窗口看着是实色」，
/// 像极了「应用根背景写死了」，隔一层排查成本。
///
/// premultiplied 的换算（RGB × alpha）在渲染器里做，这里只表达语义，
/// 见 `tests/d3d11_draw.rs::clear_color_is_premultiplied`。
pub fn clear_color(transparent: bool, surface: Color) -> Color {
    if transparent {
        surface.with_alpha(0.0)
    } else {
        surface
    }
}

// `srgb_bytes` 已搬到 `zephyra_fx::pixels`（它与后端无关）。上面的
// `pub use zephyra_fx::srgb_bytes` 让调用点不用改。

#[cfg(test)]
#[path = "tests/lib_tests.rs"]
mod tests;

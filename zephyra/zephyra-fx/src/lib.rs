#![deny(rustdoc::broken_intra_doc_links)]
//! GPU **后端无关**的那一层：顶点展开、字形图集、纹理表、CPU 参考光栅器。
//!
//! # 为什么要有这个 crate
//!
//! 之前这些代码住在 `zephyra-winit` 里，而那个 crate 实际上是 **D3D11 后端**。
//! 于是「DX11 和 DX12 分 crate、不互相依赖」这个决定带来两个坏结果：
//!
//! 1. `quad.rs` 被**逐字节复制**成两份（726 行）。任何一处改动都必须记得改两遍，
//!    否则两个后端的顶点布局会静默漂移 —— 而漂移的症状是「画面整体错位」，
//!    不报任何错。
//! 2. `cpu_raster` / `text` / `texture` **只存在于 DX11 那边**，于是 DX12 后端
//!    既没有字体、也没有纹理、也没法做 CPU 侧诊断。
//!
//! 两者的根因是同一个：**这些代码本来就没有后端依赖**，只是被放错了位置。
//!
//! # 层次
//!
//! ```text
//!   zephyra-core       纯逻辑，无外部依赖（DisplayList / 布局 / diff / 输入）
//!        │
//!        ▼
//!   zephyra-vector  ← 绘制库：几何 / 图标 / 文本 / 顶点展开 / 光栅
//!        ▲
//!        │  re-export
//!   zephyra-fx   ← 本 crate：帧循环 / 调试桥 / 宿主 API
//!        │
//!        ├──► base/zephyra-winit      D3D11 后端
//!        ├──► base/zephyra-winit-d3d12  D3D12 后端
//!        └──► base/zephyra-opengl     （将来的）OpenGL 后端
//! ```
//!
//! 三个后端**互不依赖**，只依赖本 crate。这正是 Skia 的做法：抽象层放在
//! Ganesh 层，各后端分别实现它的接口，而不是让后端之间互相引用、或者把
//! 后端无关的代码复制三份。
//!
//! # 为什么它在 `zephyra/` 下与 `zephyra-base/` 并列，而不是 `zephyra-base/` 的子目录
//!
//! 这一层**一个系统 API 都不碰**：没有 `ID3D11Device`、没有 `ID3D12Device`、
//! 没有 `IDXGIFactory`、没有窗口句柄，`unsafe_code = "forbid"`。凡是碰驱动的
//! 东西都在某个后端 crate 里。
//!
//! 所以物理位置跟着职责走 —— 它与 Skia 把 `src/effects` 放在与 `src/core`
//! **并列**处是同一个判断：效果（及其顶点展开、光栅化）的**描述**是纯数据，
//! 能在无 GPU 的机器上单测；而只有效果的**执行**才需要系统 API。
//!
//! # 本 crate 里没有的东西
//!
//! **任何 D3D 类型。** 没有 `ID3D11Device`、没有 `ID3D12Device`、没有
//! `IDXGIFactory`。凡是碰驱动的东西都该在某个后端 crate 里。
//!
//! 绘制代码也不在这里：`quad` / `geom_clip` / `offscreen` / `cpu_raster` /
//! `texture` / `pixels` 与字形来源都已搬进 `zephyra-vector`，本 crate 只按
//! 原路径再导出，宿主写法不变。

#![cfg_attr(windows, allow(dead_code))]

/// 桥的默认端口。`zephyra mcp` 与 `ZEPHYRA_DEBUG_PORT` 缺省都用它。
///
/// 放在 crate 根（而不是 dev-only 的 `bridge` 模块）：CLI 的跨仓一致性门禁
/// 在 release 测试里也要能引用这些常量。它们只是字面量，不是 dev 实现。
pub const DEFAULT_PORT: u16 = 7878;
/// 开启桥的环境变量名（值 = 端口）。
pub const PORT_ENV: &str = "ZEPHYRA_DEBUG_PORT";
/// CLI 注入：MCP server 的监听端口（应用据此连入 MCP）。
pub const MCP_PORT_ENV: &str = "ZEPHYRA_MCP_PORT";

// dev 设施：debug 构建才有；release 编译期彻底剔除（没有运行时开关可绕过）。
#[cfg(debug_assertions)]
pub mod bridge;
#[cfg(debug_assertions)]
pub mod cpu_only;
/// 资源热重载（.zstyle / .zypui）。release 下整个模块不存在。
#[cfg(debug_assertions)]
pub mod dev_reload;

// 后端无关的绘制代码已搬进 `zephyra-vector`。这里按原路径再导出，
// 使宿主与既有的 `crate::quad` / `crate::text` 写法继续可用。
pub use zephyra_vector::text;
pub use zephyra_vector::{cpu_raster, geom_clip, offscreen, pixels, quad, texture};

pub use pixels::{srgb_bytes, write_bmp, PixelRead};
pub use quad::RenderReport;
pub use text::{from_font_file, from_system_font, AbGlyphSource, FONT_OVERRIDE_ENV};
pub use texture::TextureNames;

/// 标量属性的本地定义（宿主自定义控件用）。
///
/// 刻意放这里而不是加进核心的 `props.rs`：z-index 是**渲染层**的词汇，而核心的
/// 属性表不该知道后端怎么用它。真要提升为标准键时再迁移。
///
/// 它原先只在 `zephyra-winit`（D3D11）里，而 D3D12 那个 crate 是**另一个独立 crate**
/// —— 于是同一个属性键有两份定义。属性键靠**名字**相等（`PropertyKey::new("ZIndex", …)`
/// 内部的 id 由名字派生），所以两份定义**恰好**能互换；一旦有人只改一处，另一处就静默
/// 变成另一个键，而症状是「z-index 不生效」且没有任何编译或运行错误。
///
/// 放这里就没有这份风险：两个后端共用一个定义。
pub static Z_INDEX: zephyra_core::property::PropertyKey = zephyra_core::property::PropertyKey::new(
    "ZIndex",
    zephyra_core::property::PropertyValue::int(0),
);

/// 应用的文本引擎 —— 本 crate 唯一支持的字体来源。
///
/// 之所以是类型别名而不是 `impl Trait`：它出现在各后端 `HostWindow` 与
/// [`cpu_only::CpuHost`] 的字段类型里，必须能被外部指名。
pub type TextEngine = zephyra_core::text::TextStack<AbGlyphSource>;

use zephyra_core::context::Context;
use zephyra_core::diff::DiffStats;
use zephyra_core::element::Element;
use zephyra_core::frame::Damage;
use zephyra_core::id::ElementKey;
use zephyra_core::input::TextEvent;

/// 当前输入状态 —— 由 [`zephyra_core::input::InputRouter`] 产出。
///
/// 捕获、穿透、按下抬起配对都已经在核心里处理完了，宿主与应用都不必自己写
/// 事件分发。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputState {
    /// 当前悬停的可交互节点。
    pub hovered: Option<ElementKey>,
    /// 按下时被捕获的节点（指针移出该节点后仍归它所有，直到抬起）。
    ///
    /// ⚠ 这是**主**按下态（最近一次按下的那个）。多指同时按多个控件时，
    /// 要画**全部**按下态请用 [`Self::pressed_keys`]。
    pub pressed: Option<ElementKey>,
    /// 当前**所有**被按下的节点（每个按下中的指针一个）。
    ///
    /// # 为什么需要它
    ///
    /// 多指同时按两个控件时，[`Self::pressed`] 只代表**一个** —— 而应用要画
    /// **两个**按下态。所以这里给出**全部**。
    ///
    /// 绝大多数应用只有一个指针，这个列表长度为 1 —— 于是「按 key 找自己」
    /// 那条路（而不是「只有一个按下态」）对多指也对。
    pub pressed_keys: Vec<ElementKey>,
    /// 当前**可见焦点**（焦点环该画在谁身上）。
    ///
    /// 由宿主在 `window_event` 里经 `FocusState` 维护（点击 / Tab / 失焦），
    /// 这里只**报告**——应用在 `view()` 里据此决定"这个控件要不要画光标 / 焦点环"。
    ///
    /// ⚠ 这是 [`zephyra_core::input::focus::FocusState::is_visible`] 过滤后的值：
    /// 鼠标点击设了焦点也**不**出现在这里（`focus-visible` 语义，点了开关不该
    /// 多一圈蓝边）；只有 Tab 引入的焦点才可见。焦点的**行为**（Space / Enter /
    /// 无障碍）走宿主内部的 `FocusState::focused`，与本字段无关。
    pub focused: Option<ElementKey>,
    /// **行为焦点**（键盘事件该送谁）：与 `focused` 不同，这里**不**经过
    /// `is_visible()` 过滤 —— 鼠标点进文本框得到的焦点也在这里。
    ///
    /// # 为什么必须和 `focused` 分开
    ///
    /// `focused` 是「焦点环该画在谁身上」（focus-visible 语义：鼠标点击不画蓝边）。
    /// 但**文本输入、Space/Enter 激活、无障碍**要的是「键盘事件送到谁」—— 鼠标
    /// 点进文本框后用户立刻打字，事件必须进那个框。把两者混成一个（只用
    /// `focused`）的症状是：点一下输入框，打字没反应，因为鼠标焦点被可见性过滤
    /// 成了 `None`。
    pub active_focus: Option<ElementKey>,
    /// 当前活跃指针数。
    pub pointers: usize,
    /// 本次同步收到的**文本输入**事件，按接收顺序排列。
    ///
    /// 三种事件（键盘边沿 / 可打印字符 / 输入法）在
    /// [`zephyra_core::input::TextEvent`] 里分开定义 —— 物理键与文本插入是
    /// 两件事，控件按需消费其一或两者。
    ///
    /// 一次 MCP `press_key` 会在同一轮同步中带入按下与抬起两条边沿；
    /// 应用按 [`zephyra_core::input::KeyInput::name`] 处理快捷键、按
    /// [`zephyra_core::input::TextEvent::Char`] 处理文本插入。
    pub text_events: Vec<TextEvent>,
    /// 本次同步收到的滚轮增量与当前鼠标位置。正 `delta_y` 表示向上滚动；
    /// 单位沿用 winit（物理像素或行数），应用按具体设备语义解释。
    pub scroll: Option<ScrollInput>,
    /// 本次同步按接收顺序发生的**指针**移动与按键边沿；按住移动时各帧持续收到 Move。
    ///
    /// ⚠ 名字里的「鼠标」已经不准了 —— 现在它承载**触控**与**鼠标**两条路。
    /// 字段名保持不变是为了不破坏应用侧的读法，而语义以
    /// [`PointerInput::source`] 为准。
    pub pointer_events: Vec<PointerInput>,
    /// 本次同步产出的**静态手势**（轻点 / 双击 / 长按）。
    ///
    /// 官方触控指南把静态手势单列一档：它们在**交互完成后**才触发，
    /// 与指针事件的「进行中」语义不同。
    ///
    /// ⚠ 与 [`Self::pressed`] 的关系：轻点**不是**「按下了」——
    /// 它多了「位移没超出容差」与「没超时」两条判据。
    pub gestures: Vec<GestureEvent>,
    /// 本次同步产出的**操作增量**（多指平移 / 缩放 / 旋转 + 速度）。
    ///
    /// 官方指南的操作事件**只给数据、不给形式** —— 「这是捏合还是旋转」
    /// 由应用自己判断（[`zephyra_core::input::ManipulationDelta`]）。
    pub manipulations: Vec<zephyra_core::input::ManipulationDelta>,
    /// 本次同步有一次指针交互被**取消**（系统抢走了手势 / 失焦）。
    ///
    /// # 为什么应用需要这个标志
    ///
    /// 「点击」在应用侧通常是这么还原的：「上一帧按着、这一帧松了」。
    /// 而**取消**也把 `pressed` 清成 `None` —— 于是应用会把一次
    /// **被系统抢走**的交互当成「用户松手了」⇒ **误触发一次点击**。
    ///
    /// 官方触控指南把「指针取消」单列为一个事件，正是因为它**不是**抬起：
    /// 用户的手指还在屏幕上，只是系统接管了手势（Windows 11 的三指/四指
    /// 手势就是这种情况）。
    ///
    /// 所以：**这一帧有取消 ⇒ 不要把「按下 → 松开」当成点击。**
    ///
    /// ⚠ 它是**本帧**的事实（不是「上一次」）。取消与 `pressed` 清零发生在
    /// 同一帧，所以判据「上帧按着 + 这帧没按着 + 这帧没取消」是自洽的。
    pub cancelled: bool,
    /// 操作结束时的平移速度（px/秒）—— **惯性的初值**。
    ///
    /// 官方指南的 `ManipulationInertiaStarting` 携带的正是速度：手指离开
    /// 屏幕后，内容应当按这个速度**继续滑**一小段。
    ///
    /// `None` = 本次同步没有操作结束。
    pub inertia: Option<(f32, f32)>,
    /// 客户区视口，**物理像素**，与宿主的 `inner_size()` 同源。
    /// scale 是当时的缩放比（`window.scale_factor()`）。
    ///
    /// # 为什么两个都要给
    ///
    /// 视图做**两种**不同单位的计算，缺一个就只能猜：
    ///
    /// - **断点**按有效像素 —— 官方 `AdaptiveTrigger.MinWindowWidth` 的定义是
    ///   "the minimum window width (**in effective pixels**)"，所以要 `viewport.0 / scale`；
    /// - **列数**按布局宽 —— 它除的是卡片宽度，而卡片宽度是布局单位（本仓当前
    ///   1 单位 = 1 物理像素），所以要 `viewport.0`。
    ///
    /// 只给其中一个，另一个就得靠约定推，而"忘了除 scale"是这个口径唯一的失败方式，
    /// 它的症状是「该折叠导航时没折叠」—— 不是报错，是布局明显不合身。
    ///
    /// 放在 [`Self`] 上而不是新增一个 trait 方法：`on_input` 已经是「宿主每帧开头
    /// 把状态推给视图」的既有通道，再加一个方法只是多一条要记得调的路径。
    pub viewport: (f32, f32),
    /// 视口的缩放比。见 [`Self::viewport`]。
    pub scale: f32,
    /// 距上一帧的**毫秒**数（宿主用 `Instant` 实测，不是按刷新率拍的）。
    ///
    /// # 为什么动画时钟走这里
    ///
    /// 动画 = 缓动后的进度，而进度必须由**真实经过的时间**推进。
    /// 若按"每帧推进一个固定量"，那么掉一帧动画就变慢、快机器上就变快 ——
    /// 而官方规范的时长全是**毫秒**（83 / 167 / 250 / 333），只在真实时间下成立。
    ///
    /// 首帧没有"上一帧"，宿主填 0。填 `None` 也可以，但那会让动画在第一帧
    /// 与余下帧之间多一条分支，而本条已经能表达"没经过时间"。
    pub dt_ms: f32,
    /// 系统的语言偏好（有序，最偏好的在前）—— 对应浏览器的 `navigator.languages`。
    ///
    /// 宿主经 `zephyra_platform::Platform::system_languages` 读，读不到给空表。
    /// **空表不是"英语"**，是"读不到"：上层据此回落默认语言，
    /// 而不是把界面钉死在某一门上。
    ///
    /// 放在这里而不是另开一个 trait 方法：它与视口 / 缩放比同属
    /// 「宿主每帧开头把外部事实推给视图」那一批，多一条路径只会更容易漏。
    pub languages: Vec<String>,
    /// 布局后带 key 节点的矩形快照（P1.2，物理像素，与 `pointer_events` 同一空间）。
    ///
    /// 宿主在 `on_input` 之前从已布局的树上采一份。`Vec` 而非 `HashMap` 是因为
    /// 节点数只有几十个、线性查足够快，且 `Vec` 保留 `Default` / `PartialEq`，
    /// 不会破坏应用里大量 `InputState { ..Default::default() }` 的写法。
    pub rects: Vec<(ElementKey, zephyra_core::core::Rect)>,
}

impl InputState {
    /// 查询带 key 节点的**布局后**矩形（P1.2）。
    ///
    /// 与 `pointer_events` 同一坐标系（物理像素、绝对坐标），可直接用于绝对定位、
    /// 滚动条拇指、下拉浮层等需要「控件在哪」的场景。`None` = 该 key 此刻不在树上
    /// 或没有布局矩形。
    pub fn rect_of(&self, key: &ElementKey) -> Option<zephyra_core::core::Rect> {
        self.rects.iter().find(|(k, _)| k == key).map(|(_, r)| *r)
    }

    /// 当前**全部**按下目标（多指各按一个时长度 > 1）。
    ///
    /// `pressed_keys` 为空时回落到 `pressed` —— 单指宿主只填后者也不会漏。
    /// 这是「按 key 找自己」那条判据的唯一入口，应用不该再各写一遍 fallback。
    pub fn pressed_set(&self) -> Vec<ElementKey> {
        if self.pressed_keys.is_empty() {
            self.pressed.iter().cloned().collect()
        } else {
            self.pressed_keys.clone()
        }
    }

    /// 本帧新产生的点击（上帧按着、这帧松了），用 [`Self::pressed_set`] 作当前集合。
    ///
    /// `last` 是**上一帧**的 [`Self::pressed_set`] 结果 —— 应用要在 `on_input`
    /// 末尾把它存下来，下一帧再传进来。判据本身（取消不是抬起、多指集合差）住在
    /// `zephyra_core::input::clicks_from`，与无障碍桥共用同一份。
    pub fn clicks_from(&self, last: &[ElementKey]) -> Vec<ElementKey> {
        zephyra_core::input::clicks_from(last, &self.pressed_set(), self.cancelled)
    }
}

/// 一次滚轮输入。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollInput {
    /// 事件发生时的客户区物理像素坐标。
    pub x: f32,
    pub y: f32,
    /// 水平滚动增量。
    pub delta_x: f32,
    /// 垂直滚动增量；正数向上。
    pub delta_y: f32,
}

/// 指针来源（触控 / 笔 / 鼠标）。
///
/// 转出给应用用 —— 应用要靠它判断「要不要给悬停反馈」（触控是**双状态**的，
/// 见官方触控指南）。
pub use zephyra_core::input::PointerSource;

/// 一次指针路径事件，坐标为客户区物理像素。
///
/// # 多点触控
///
/// `id` 是**指针身份**：触控时每根手指一个（多点触控的基础），
/// 鼠标恒为 0。`source` 说明它来自触控 / 笔 / 鼠标 —— 应用据此决定
/// 「要不要给悬停反馈」（触控是**双状态**的，见官方触控指南）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerInput {
    /// 指针身份（触控时每根手指一个）。
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub phase: PointerInputPhase,
    /// 这个指针的来源。
    pub source: zephyra_core::input::PointerSource,
}

/// 指针路径的生命周期相位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerInputPhase {
    Down,
    Move,
    Up,
    /// 交互被**异常取消**（系统接管手势 / 失焦）。
    ///
    /// ⚠ **取消不是抬起**：取消时**不**提交点击 —— 用户是被系统抢走的，
    /// 不是「松手了」。
    Cancel,
}

/// 这一批指针事件里有没有**取消**。
///
/// # 为什么需要它
///
/// 「点击」在应用侧通常是这么还原的：「上一帧按着、这一帧松了」。
/// 而**取消**也把 `pressed` 清成 `None` —— 于是应用会把一次
/// **被系统抢走**的交互当成「用户松手了」⇒ **误触发一次点击**。
///
/// 官方触控指南把「指针取消」单列为一个事件，正是因为它**不是**抬起：
/// 用户的手指还在屏幕上，只是系统接管了手势（Windows 11 的三指/四指手势）。
///
/// 所以宿主在把事件交给应用之前，先问一句「这一批里有取消吗」——
/// 有就把 [`InputState::cancelled`] 置真，应用据此**不**提交点击。
pub fn has_cancelled(events: &[PointerInput]) -> bool {
    events.iter().any(|e| e.phase == PointerInputPhase::Cancel)
}

/// 一个静态手势（官方触控指南的「静态手势事件」一档）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GestureEvent {
    /// 手势种类。
    pub kind: zephyra_core::input::StaticGesture,
    /// 落在哪个节点上。
    pub target: ElementKey,
}

/// 一帧里应用要做的事：宿主把「当前状态」变成一棵 `Element`。
///
/// # 两段式：先同步状态，再造树
///
/// [`View::on_input`] 与 [`View::view`] 分开是有意的：命中测试需要**已布局**的
/// 矩形，所以 hover 只能在布局之后才知道 —— 它必然晚于上一帧的 `view()`。因此
/// 宿主在每帧开头先把新状态推给视图，视图再据此造下一帧的树。
///
/// 这就是保留式 UI 的节奏：输入改变状态，状态改变声明，声明经 diff 落到保留树。
pub trait View {
    /// 宿主在每帧开头把输入状态推过来。
    fn on_input(&mut self, state: &InputState);

    /// 造出这一帧的声明树。
    ///
    /// `ctx` 提供当前主题 —— 核心不持有主题，主题是应用状态。
    fn view(&self, ctx: &Context<'_>) -> Element;

    /// 宿主是否还要继续泵帧（动画正在进行）。
    ///
    /// # 为什么必须作为一个正式钩子
    ///
    /// 本仓是保留式 UI：宿主**不**常驻重绘，帧只在 `self.dirty` 时产出。
    /// 「动画播放中」这个事实只有靠这个钩子才能表达：事件推进一格动画，
    /// 循环又睡死 —— 中间整段弹簧曲线在画面上根本看不到（症状即
    /// 「动画有问题」：动画进度全靠鼠标事件凑）。
    ///
    /// 默认 `false`：静止的保留式 UI 一帧都不画。实现者（应用）只需要回答
    /// 「某个动画是否还没收敛」—— 见各应用自己的 `View::wants_frames` 实现。
    fn wants_frames(&self) -> bool {
        false
    }

    /// 跨线程唤醒（P0.3）：有消息被投进本视图的队列，宿主随后会调
    /// [`Self::on_message`]；`waker_token` 是唤醒的单调序号，可在调试时对照。
    ///
    /// 默认空实现 —— 不需要跨线程通知的视图无需关心。
    fn on_wake(&mut self, _waker_token: u64) {}

    /// 处理一条跨线程投递的消息（P0.3）。
    ///
    /// 宿主在事件循环排空队列时按**投递顺序**逐条调用，调用后紧接着的一帧会重画，
    /// 因此状态变更当帧可见。默认空实现。
    fn on_message(&mut self, _msg: &mut dyn std::any::Any) {}

    /// 取出本帧「用外部程序打开 URL」的意图（默认无）。
    ///
    /// `View` 不碰系统 API —— 它在点击处理里记下 URL，宿主每帧 drain 后调用平台
    /// 壳打开浏览器。门禁/离线路径只调 `on_input` / `click`，不会触发外部进程。
    fn take_pending_url(&mut self) -> Option<&'static str> {
        None
    }
}

#[cfg(test)]
#[path = "../tests/unit/tests/input_state_tests.rs"]
mod input_state_tests;

/// 一帧的输出：diff 统计 + 渲染统计。
///
/// **不碰 GPU。** 它把「布局 + 排版 + `DisplayList` → 顶点流」这一整段单独跑完，
/// 所以在没有窗口、没有设备、没有交换链的环境里也能得到。
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// 声明 → 保留的对账结果。
    pub diff: DiffStats,
    /// DisplayList → 顶点流 的回放结果。
    pub render: RenderReport,
    /// 命中的最上层可交互节点（`None` = 点在空白处）。
    pub hovered: Option<String>,
    /// 本帧的脏区，供宿主做**帧内** scissor。
    ///
    /// ⚠ 不能用它跨帧跳过回放 —— flip model 下 `Present` 之后后缓冲内容未定义，
    /// 任何要 present 的帧都必须重画整个目标。详见
    /// [`zephyra_core::frame`](zephyra_core::frame) 模块文档。
    pub damage: Damage,
    /// 本帧是否真的画了并上屏了。
    ///
    /// `false` = core 判定 `DisplayList` 与上次逐字节相同，宿主整帧跳过
    /// （含 present）。所以 `render` 各项此时全是 0 —— 那不是「什么都没画」，
    /// 而是「这一帧不需要画」。
    ///
    /// 要判断「有没有东西可见」不能看 `render`，得看这个字段。
    pub redrawn: bool,
}

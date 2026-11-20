//! 调试桥：AI 与外部工具经 **localhost TCP** 读取布局、操控界面。
//!
//! # 定位：库层只管「管道」，不管「谁在听」
//!
//! MCP（Model Context Protocol）是 stdio 上的 JSON-RPC，而 AI 客户端拉起的是
//! `zephyra mcp` 这个 CLI 进程——它够不着运行中的应用。所以链路是三层：
//!
//! ```text
//! AI 客户端 ──stdio/JSON── zephyra mcp（CLI）──TCP 行协议── 应用（本模块）
//! ```
//!
//! 本模块住在 `zephyra-fx` 而不是某个渲染后端：它一个 GPU 类型都不碰
//! （判据照旧——「能不能在无 GPU 环境写单测」），而它的两个消费者
//! （`zephyra-winit` / `zephyra-winit-d3d12` 的 `HostWindow`）都依赖 fx。
//!
//! # 为什么是行协议而不是 JSON
//!
//! 应用端保持**零依赖**（workspace 至今没有 serde）。JSON 的解析责任放在 CLI
//! 一侧（它反正要为 MCP 持有 `serde_json`）；桥内协议只需表达「读树 / 读属性 /
//! 注入点击 / 截图」这几件事，一行一命令的文本协议足够，而转义、浮点格式这些
//! JSON 坑就永远不会出现在热路径上。
//!
//! # 状态纪律（谁负责什么）
//!
//! - **桥**：连接管理（非阻塞 accept / 半行缓冲 / 写响应），命令的**解析**；
//! - **宿主**（[`BridgeHost`] 的实现者）：命令的**执行**——树 dump、属性读取、
//!   输入注入、截图。桥对 `Node` / `InputState` / GPU 一无所知。
//!
//! 宿主每帧调一次 [`DebugBridge::poll`]（帧循环开头、`frame()` 之前）：
//! 注入的 hover/click 在**同一帧**就参与命中测试与布局，AI 读回的树永远是
//! 操控后的状态。截图是唯一例外——它必须发生在「渲染后、present 前」
//! （flip model 下 present 之后后缓冲内容未定义），由宿主实现
//! [`BridgeHost::screenshot`] 时自行择机（D3D11 与 D3D12 都在 present 前读回）。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

/// 默认端口。`zephyra mcp` 与 `ZEPHYRA_DEBUG_PORT` 缺省都用它。
pub const DEFAULT_PORT: u16 = 7878;

/// 开启桥的环境变量名（值 = 端口）。
pub const PORT_ENV: &str = "ZEPHYRA_DEBUG_PORT";

/// 由 `zephyra dev` / `zephyra run` 注入：MCP server 的监听端口。
///
/// 设置后应用**主动连入 MCP**（agent 只经 stdio 与 MCP 说话，不再拿端口连应用）。
/// 这是 CLI 启动应用时才有的能力：环境变量由 CLI 传给子进程，见 `zephyra-cli`
/// 的 `spawn_inherit` / `spawn_pumped`。不设置则维持旧模式（监听 `PORT_ENV`）。
pub const MCP_PORT_ENV: &str = "ZEPHYRA_MCP_PORT";

/// 一条从 TCP 上解析出来的命令。
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// 探活。回 `pong`。
    Ping,
    /// 布局树 dump（缩进文本，路径列可直接复制给 `props`）。
    Tree {
        /// JSON 形态（行式 JSON，一行一个节点）。
        json: bool,
    },
    /// 给模型的文本布局上下文：视口、最近帧统计与带绝对坐标的树。
    Layout,
    /// 指定节点的属性表。`path` 是树里的子下标链（`0.1.2`）。
    Props {
        path: Vec<usize>,
        /// JSON 形态。
        json: bool,
    },
    /// 最近一帧的统计（quads / dirty / hovered / **damage**）。
    Stats {
        /// JSON 形态。
        json: bool,
    },
    /// 视口尺寸（物理像素）。
    Viewport,
    /// 注入指针移动（同物理鼠标移到该坐标）。
    Hover { x: f32, y: f32 },
    /// 注入一次完整点击：move → down → up（顺序在同一帧内完成）。
    Click { x: f32, y: f32 },
    /// 按下主指针按钮并保持。
    PointerDown,
    /// 抬起主指针按钮。
    PointerUp,
    /// 在有限时长内开启性能采样。
    ProfileStart { duration_seconds: u64 },
    /// 查询性能采样状态或已完成报告。
    ProfileReport,
    /// 注入一个逻辑键的按下与抬起。
    PressKey { key: String },
    /// 在给定指针位置注入滚轮增量。
    Scroll {
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    },
    /// 获取进程 CPU/GPU 与内存资源数据。
    Resources,
    /// 渲一帧并读回像素，回 BMP 二进制（见 [`Response::Binary`]）。
    Shot {
        /// 强制整帧重画（默认沿用本帧脏区，见 [`ShotOptions`]）。
        repaint: bool,
    },
    /// 按**元素 key** 注入一次完整点击（等价于先定位再 `click`）。
    ///
    /// 为什么要有它：布局树的 key 列就是元素身份，而坐标要人从树里抄 ——
    /// 抄错一个数就是「点了别的东西」，而且看不出来。
    ClickKey { key: String },
    /// 按元素 key 注入指针移动。
    HoverKey { key: String },
    /// 按**布局树路径**注入一次完整点击。
    ClickPath { path: Vec<usize> },
    /// 按布局树路径注入指针移动。
    HoverPath { path: Vec<usize> },
    /// 等应用**真的画了** N 帧再应答。
    ///
    /// # 为什么不是「睡 N 毫秒」
    ///
    /// 排查动画类问题时，「等动画结束」要按**帧**算，不是按墙钟算：应用可能
    /// 正在忙（帧慢），也可能静止（不出帧）。按帧等才是「下一帧的画面到手了」。
    ///
    /// 语义细节：数的是**渲染过**的帧（不是「跑过」的帧）——静止的应用不出帧，
    /// 于是这个命令会一直等到客户端超时（10 秒）。要按墙钟等就用 `sleep`。
    WaitFrames { frames: u32 },
    /// 按**可见文字**找元素。
    ///
    /// # 为什么要有它
    ///
    /// AI 看到的是屏幕：「那个写着『快速开始』的导航项」。而树里那行的 key 是
    /// `label` —— 所有导航项的 key 都一样，光看树**分不出是哪一个**。
    ///
    /// 所以要有一条「按人看得见的字去找」的路径。
    Find {
        /// 查询串（大小写不敏感的包含匹配）。
        query: String,
        /// JSON 形态。
        json: bool,
    },
    /// 输入一段**文字**（不是按键）。
    ///
    /// # 为什么 `press_key` 不够
    ///
    /// `press_key` 一次一个逻辑键 —— 填一个输入框要按 N 次。而「输入 hello」
    /// 是**一个**语义动作。
    TypeText { text: String },
    /// 命中测试：这个点上是什么。
    ///
    /// # 为什么需要它
    ///
    /// AI 手里有一张截图，于是会问「(500, 300) 那里是什么」。没有这条命令，
    /// 只能自己扫树 + 算矩形 —— 而那是**猜**（哪个在上？哪个能被点？）。
    Hit { x: f32, y: f32 },
    /// 注入一次**触摸**（多点触控的主入口）。
    ///
    /// # 为什么需要它
    ///
    /// 没有触摸屏的机器上，触控那条路**根本没法验** —— 而「触控能不能用」
    /// 恰恰是最该验的一件事（Windows 11 把三指/四指手势交给系统时，应用
    /// 收到的是**取消**而不是抬起；那正是最容易出「误触发点击」的地方）。
    ///
    /// 它也给了 AI 一条**与真实设备同形**的路：`touch_down` / `touch_move` /
    /// `touch_up` / `touch_cancel` 与 `WindowEvent::Touch` 的相位一一对应，
    /// 于是「两指捏合」「系统抢走手势」都能被复现。
    Touch {
        /// 相位。
        phase: TouchPhase,
        /// 指针身份（多点触控靠它区分手指）。
        id: u64,
        x: f32,
        y: f32,
    },
    /// 改窗口客户区尺寸（**物理**像素）。
    ///
    /// # 为什么 AI 需要它
    ///
    /// 「这个布局在窄窗口下还成立吗」是 UI 框架最常见的自查项，而它**只能**
    /// 真的改窗口大小来看 —— 读样式表猜不出来（弹性布局、换行、裁切都
    /// 要真排一遍）。
    ///
    /// 单位是**物理**像素，与 `viewport` / 树的坐标一致（不引入第二套单位）。
    Resize { width: u32, height: u32 },
}

/// 一帧的统计（调试桥读的那一份）。
///
/// # 为什么是结构体而不是一行字符串
///
/// 残影类缺陷的判据是「这一帧重画了多大范围」——`damage=Full` 还是
/// `damage=Rect(...)`。它原先根本不在统计里（宿主只报 quads / dirty / hovered），
/// 于是排查的人只能改宿主源码加临时日志、**重编一遍**才看得到那一帧的脏区。
///
/// 结构化之后，文本形态与 JSON 形态都从**同一份数据**出，不会出现两份口径。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameStats {
    /// 本帧画出的四边形数。
    pub quads: usize,
    /// 被跳过的命令数。
    pub skipped: usize,
    /// 本帧被标脏的节点数。
    pub dirtied: usize,
    /// 本帧需要重排的计数。
    pub needs_layout: usize,
    /// 本帧是否真的重画了（false = 整帧跳过，连 present 都省）。
    pub redrawn: bool,
    /// 当前悬停的元素 key。
    pub hovered: Option<String>,
    /// 本帧的重画范围。`None` = 还没有帧。
    ///
    /// `Full` 与 `Rect(..)` 的区别就是「脏区之外的内容还在不在」——
    /// MSAA 目标纹理逐帧存活，脏区外保留上一帧的像素，所以收窄成局部矩形
    /// 就会把上一帧的残像留在屏幕上。
    pub damage: Option<zephyra_core::frame::Damage>,
    /// 宿主**已经渲染过**的帧数（自进程启动起累计）。
    ///
    /// # 为什么需要它
    ///
    /// 「应用现在多少帧」这件事，客户端**算不出来** —— 它只看得见「最后一帧的
    /// 快照」。有了这个计数器，两次采样相减就知道这期间画了几帧，于是
    /// fps = Δframes / Δ秒。
    ///
    /// ⚠ 它数的是**画过的**帧：应用静止时不出帧 ⇒ fps 掉到 0。那是**事实**
    /// （「应用没在动」），不是测量错误。
    pub frames: u64,
    /// 上一帧 `render_pass` 的耗时（微秒）。
    ///
    /// `None` = 宿主不提供（D3D11 / Vulkan 侧没有这一层）。
    ///
    /// # 为什么把它单列
    ///
    /// `frame_total` 是**整帧**（含 core 的对账 / 布局 / 绘制）。而
    /// `render_pass` 是**宿主侧**的那一段（开帧、记命令、提交）。两者的差值
    /// 就是「core 之外、宿主自己花掉的时间」—— 那是最容易藏问题的一段
    /// （缓冲重建、GPU 等待、图集上传）。
    pub render_submit_us: Option<u64>,
    /// **输入 → 这一帧就绪**的耗时（微秒）。
    ///
    /// `None` = 这一帧之前没有输入（**不是**「延迟为零」）。
    ///
    /// # 为什么需要它
    ///
    /// 「画面跟手吗」是个**可测量**的问题：从输入事件到达，到承载它的那一帧
    /// 准备好，中间花了多久。而这一段里藏着好几处可能拖慢它的东西 ——
    /// 事件循环的控制流、命中测试、diff、布局、绘制。
    ///
    /// 不测就只能猜；测了才知道该优化哪一段。
    ///
    /// ⚠ 它**不包含** DWM 的合成时间（present 之后到像素亮起还有一段）——
    /// 那一段是系统的，不是应用的。
    pub input_latency_us: Option<u64>,
    /// 本帧的**分段耗时**（core 侧：diff / 样式 / 脏区扫描 / 布局 / 绘制）。
    ///
    /// `None` = 宿主不提供（此后端没有 core 的帧缓存，或还没跑过一帧）。
    ///
    /// # 为什么放在 `stats` 而不是单开一条命令
    ///
    /// 「这一帧慢在哪」与「这一帧画了什么」是**同一个观察**的两个侧面：
    /// 排查「点了按钮之后卡了」时，要同时看 quads 涨了多少、diff 花了多久。
    /// 分成两条命令就要两次往返，而两次往返之间应用可能又变了。
    ///
    /// ⚠ 稳态帧（宿主走了 `Unchanged` 分支）只跑了前三段 ——
    /// `layout` 与 `paint` 是 **0**，意思是「那两段**没跑**」，不是「不花钱」。
    pub stages: Option<zephyra_core::frame::StageTimings>,
    /// 本帧的**离屏趟数** —— 每个带滤镜的命令一趟。
    ///
    /// # 为什么它是「GPU 干多少活」的主指标
    ///
    /// 一趟离屏 = 把内容**整帧**重画进离屏 RT、若干趟全屏模糊、一次全屏拷贝，
    /// 再合成回去。也就是说**一趟 ≈ 四趟全屏光栅化**，而 `quads` 只数画了几个
    /// 四边形 —— 它**完全看不出**这一趟。
    ///
    /// 实测（gallery，1400×800，D3D12，Intel UHD 630）：静止时 GPU 0%，
    /// 导航时峰值 18.7%；而单看 `quads`（98 ~ 115）根本猜不到代价在哪。
    ///
    /// ⚠ 它是**离屏趟数**，不是「模糊趟数」：一趟里可能含多趟模糊。
    /// 要看模糊与合成的分别计数，看 [`Self::backdrop_blur_passes`] 等字段。
    pub offscreen_passes: usize,
    /// 本帧的**背景模糊**趟数（磨砂玻璃）。
    pub backdrop_blur_passes: usize,
    /// 本帧的**透镜**趟数。
    pub lens_passes: usize,
    /// 本帧的**折射**趟数。
    pub refraction_passes: usize,
}

impl std::fmt::Display for FrameStats {
    /// 一行文本形态（与结构化字段同源）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "quads={} skipped={} dirty={} needs_layout={} redrawn={} hovered={}",
            self.quads,
            self.skipped,
            self.dirtied,
            self.needs_layout,
            self.redrawn,
            self.hovered.as_deref().unwrap_or("-"),
        )?;
        match &self.damage {
            Some(damage) => write!(f, " damage={damage}"),
            None => Ok(()),
        }?;
        match &self.stages {
            Some(s) => write!(
                f,
                " stages=diff:{}/style:{}/scan:{}/layout:{}/paint:{}/total:{}",
                s.diff_us, s.style_us, s.scan_us, s.layout_us, s.paint_us, s.total_us
            ),
            None => Ok(()),
        }?;
        write!(f, " frames={}", self.frames)?;
        match self.input_latency_us {
            Some(us) => write!(f, " latency={us}us"),
            None => Ok(()),
        }?;
        match self.render_submit_us {
            Some(us) => write!(f, " render_submit={us}us"),
            None => Ok(()),
        }?;
        write!(
            f,
            " passes=off:{}/blur:{}/lens:{}/refract:{}",
            self.offscreen_passes,
            self.backdrop_blur_passes,
            self.lens_passes,
            self.refraction_passes,
        )
    }
}

impl FrameStats {
    /// JSON 形态（一行一个对象）。
    ///
    /// 手写而不是引 JSON 依赖：库层调试桥的规矩是**纯 std 行协议**，
    /// 而这里只是把几个数与一个字符串排成对象形状。
    pub fn to_json(&self) -> String {
        let mut out = String::from("{");
        out.push_str(&format!("\"quads\":{}", self.quads));
        out.push_str(&format!(",\"skipped\":{}", self.skipped));
        out.push_str(&format!(",\"dirtied\":{}", self.dirtied));
        out.push_str(&format!(",\"needs_layout\":{}", self.needs_layout));
        out.push_str(&format!(",\"redrawn\":{}", self.redrawn));
        match &self.hovered {
            Some(h) => out.push_str(&format!(",\"hovered\":{}", json_string(h))),
            None => out.push_str(",\"hovered\":null"),
        }
        match &self.damage {
            Some(zephyra_core::frame::Damage::Full) => {
                out.push_str(",\"damage\":{\"kind\":\"full\"}");
            }
            Some(zephyra_core::frame::Damage::Rect(r)) => {
                out.push_str(&format!(
                    ",\"damage\":{{\"kind\":\"rect\",\"x\":{},\"y\":{},\"width\":{},\"height\":{}}}",
                    r.x, r.y, r.width, r.height
                ));
            }
            None => out.push_str(",\"damage\":null"),
        }
        match &self.stages {
            Some(s) => out.push_str(&format!(
                ",\"stages\":{{\"diff\":{},\"style\":{},\"scan\":{},\"layout\":{},\"paint\":{},\"total\":{}}}",
                s.diff_us, s.style_us, s.scan_us, s.layout_us, s.paint_us, s.total_us
            )),
            None => out.push_str(",\"stages\":null"),
        }
        out.push_str(&format!(",\"frames\":{}", self.frames));
        match self.input_latency_us {
            Some(us) => out.push_str(&format!(",\"latency_us\":{us}")),
            None => out.push_str(",\"latency_us\":null"),
        }
        match self.render_submit_us {
            Some(us) => out.push_str(&format!(",\"render_submit_us\":{us}")),
            None => out.push_str(",\"render_submit_us\":null"),
        }
        out.push_str(&format!(",\"offscreen_passes\":{}", self.offscreen_passes));
        out.push_str(&format!(
            ",\"backdrop_blur_passes\":{}",
            self.backdrop_blur_passes
        ));
        out.push_str(&format!(",\"lens_passes\":{}", self.lens_passes));
        out.push_str(&format!(
            ",\"refraction_passes\":{}",
            self.refraction_passes
        ));
        out.push('}');
        out
    }
}

/// JSON 字符串字面量（只处理 `\"` `\\` 与控制字符）。
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 「这一帧跳过了」⇒ 统计里该写什么。
///
/// # 为什么需要这条规则
///
/// 「画面上有什么」是「**上一次画的那帧**」，不是「这一帧画了几何」。
/// 而跳过的帧画了 **0** 个四边形 —— 报出去会被读成「屏幕上是空的」，
/// 而实际上应用只是跳过了这一帧（`redrawn=false` 已经说了这件事）。
///
/// 实测症状：`zephyra perf watch` 在静止的应用上那一列恒为 0，
/// 而画面上明明有 73 个节点的界面。
///
/// # 契约
///
/// - `drawn` = 上一次**画出来**的那帧的统计（宿主手里存着的那份）。
/// - `current` = 这一帧的新值（分段耗时、脏节点数、悬停项、帧计数……）。
/// - 返回值 = 写回宿主那份统计的内容。
///
/// ⚠ 只保留**画面相关**的三项（`quads` / `skipped` / `damage`）；
/// 其余（分段耗时、脏节点数）取这一帧的 —— 那些是「这一帧发生了什么」，
/// 而「画面上有什么」才是「上一次画的」。
pub fn stats_after_skipped(drawn: &FrameStats, current: &FrameStats) -> FrameStats {
    FrameStats {
        quads: drawn.quads,
        skipped: drawn.skipped,
        damage: drawn.damage,
        // 离屏趟数同理：**画出来的是上一次那帧**，所以趟数也取上一次 ——
        // 否则「这一帧跳过了」会把「画面是靠几趟离屏画出来的」抹成 0，
        // 而那正是 GPU 占用的来源。
        offscreen_passes: drawn.offscreen_passes,
        backdrop_blur_passes: drawn.backdrop_blur_passes,
        lens_passes: drawn.lens_passes,
        refraction_passes: drawn.refraction_passes,
        ..current.clone()
    }
}

/// 截图选项。
///
/// # 为什么有「沿用本帧脏区」这一档
///
/// 截图是**观测工具**，它不该改变被观测的对象。原先截图一律 `Damage::Full`
/// （整帧重画），于是「上一帧没被清掉的像素」在截图**之前**就被擦掉了 ——
/// 残影类缺陷因此**永远截不出来**，而截图看上去一切正常。
///
/// 沿用本帧脏区时：MSAA 目标纹理逐帧存活 ⇒ 脏区外保留上一帧的内容 ⇒ 读回
/// 到的就是「当前帧 + 脏区外的旧像素」= **残影本体**。
///
/// ⚠ 两个前提必须写清楚，否则下一个人会得出「工具测不出来」的错误结论：
///
/// - **MSAA 关时看不到残影**：flip model 下 present 之后后缓冲内容未定义，
///   宿主只能整帧重画（见 `zephyra-winit-d3d12` 的 `render_pass`）。
/// - **D3D11 侧同理**：那条路径没有持久 MSAA 目标，每帧都是整帧。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShotOptions {
    /// 强制整帧重画（默认 `false` = 沿用本帧脏区）。
    pub repaint: bool,
}

/// 单帧 CPU 生成阶段耗时；单位为微秒，不包含 GPU 完成等待。
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameTiming {
    pub update_us: u64,
    pub render_submit_us: u64,
    pub total_us: u64,
    /// core 侧的**分段**耗时（对账 / 样式 / 脏区扫描 / 布局 / 绘制）。
    ///
    /// `None` = 宿主不提供（没有 core 帧缓存的那种宿主）。
    ///
    /// # 为什么要有它
    ///
    /// `update_us` 是**一整段**（宿主更新视图 + 帧缓存），看不出「慢在哪」。
    /// 而 core 现在自己测分段（见 `zephyra_core::frame::StageTimings`）——
    /// 于是「这一帧的 300 µs 花在哪」第一次能被**读出来**，而不是靠猜。
    pub stages: Option<zephyra_core::frame::StageTimings>,
}

#[derive(Debug)]
pub struct PerformanceProfiler {
    deadline: Option<Instant>,
    started_at: Option<Instant>,
    samples: Vec<ProfileSample>,
    running: bool,
}

#[derive(Debug, Clone, Copy)]
struct ProfileSample {
    elapsed_ms: u64,
    timing: FrameTiming,
}

impl PerformanceProfiler {
    pub fn new() -> Self {
        Self {
            deadline: None,
            started_at: None,
            samples: Vec::new(),
            running: false,
        }
    }

    pub fn start(&mut self, duration_seconds: u64) -> Result<(), String> {
        const MAX_DURATION_SECONDS: u64 = 300;
        if !(1..=MAX_DURATION_SECONDS).contains(&duration_seconds) {
            return Err(format!("采集时长必须在 1..={MAX_DURATION_SECONDS} 秒内"));
        }
        let now = Instant::now();
        self.started_at = Some(now);
        self.deadline = Some(now + Duration::from_secs(duration_seconds));
        self.samples.clear();
        self.samples.reserve(3000);
        self.running = true;
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.running
            && self
                .deadline
                .is_some_and(|deadline| Instant::now() < deadline)
    }

    pub fn record(&mut self, timing: FrameTiming) {
        if !self.is_running() {
            self.running = false;
            return;
        }
        let elapsed_ms = self
            .started_at
            .map(|started| started.elapsed().as_millis() as u64)
            .unwrap_or_default();
        self.samples.push(ProfileSample { elapsed_ms, timing });
        if self.samples.len() >= 3000
            || self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.running = false;
        }
    }

    pub fn report(&mut self) -> Result<String, String> {
        if self.started_at.is_none() {
            return Err("尚未启动性能采样".into());
        }
        if self.is_running() {
            let remaining_ms = self
                .deadline
                .unwrap()
                .saturating_duration_since(Instant::now())
                .as_millis();
            return Ok(format!("PROFILE_STATE running remaining_ms={remaining_ms}"));
        }
        self.running = false;
        Ok(format_profile_report(
            self.started_at.unwrap(),
            &self.samples,
        ))
    }
}

impl Default for PerformanceProfiler {
    fn default() -> Self {
        Self::new()
    }
}

fn format_profile_report(started_at: Instant, samples: &[ProfileSample]) -> String {
    if samples.is_empty() {
        return concat!(
            "PROFILE_STATE complete\n",
            "frames=0\n",
            "采集期间没有完成任何一次重绘 —— 应用是静止的（那本身就是结论）。\n",
            "要看「静止时每帧花多少」，用 stats 的 stages 字段（宿主每帧都测）。"
        )
        .into();
    }
    let quantile = |mut values: Vec<u64>, percentile: usize| {
        values.sort_unstable();
        values[((values.len() - 1) * percentile).div_ceil(100)]
    };
    let column = |select: fn(&FrameTiming) -> u64| {
        let values: Vec<u64> = samples
            .iter()
            .map(|sample| select(&sample.timing))
            .collect();
        let average = values.iter().sum::<u64>() as f64 / values.len() as f64;
        format!(
            "min={}us avg={average:.1}us p50={}us p95={}us max={}us",
            values.iter().min().unwrap(),
            quantile(values.clone(), 50),
            quantile(values.clone(), 95),
            values.iter().max().unwrap()
        )
    };
    // 分段（core 侧）的百分位。
    //
    // ⚠ 只统计**报了分段**的那些帧 —— 稳态帧（宿主走了 `Unchanged` 分支）
    // 只跑了前三段，后两段是 0。把它们混进来算 p95 会**系统性偏低**：
    // 「layout 的 p95」会被一堆「layout 根本没跑」的 0 拉下来。
    //
    // 所以这里报的是「**画了**的帧」的分段 —— 那才是「重画时哪一段最贵」。
    let stage_column = |select: fn(&zephyra_core::frame::StageTimings) -> u64| {
        let values: Vec<u64> = samples
            .iter()
            .filter_map(|sample| sample.timing.stages.as_ref().map(select))
            .collect();
        if values.is_empty() {
            return "unavailable".to_string();
        }
        let average = values.iter().sum::<u64>() as f64 / values.len() as f64;
        format!(
            "min={}us avg={average:.1}us p50={}us p95={}us max={}us n={}",
            values.iter().min().unwrap(),
            quantile(values.clone(), 50),
            quantile(values.clone(), 95),
            values.iter().max().unwrap(),
            values.len()
        )
    };
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    // fps = 帧数 / 墙钟秒数。
    //
    // ⚠ 这是**宿主帧率**，不是「屏幕刷新率」：应用静止时不出帧，于是 fps 掉到 0 ——
    // 而那正是「应用没在动」的**事实**，不是测量错误。
    let fps = if elapsed_ms == 0 {
        0.0
    } else {
        samples.len() as f64 * 1000.0 / elapsed_ms as f64
    };
    let mut report = format!(
        concat!(
            "PROFILE_STATE complete\n",
            "frames={}\n",
            "elapsed_ms={}\n",
            "fps={:.1}\n",
            "frame_total {}\n",
            "update_layout {}\n",
            "render_submit {}\n",
            "stage_diff {}\n",
            "stage_style {}\n",
            "stage_scan {}\n",
            "stage_layout {}\n",
            "stage_paint {}\n",
            "frame_index elapsed_ms total_us update_layout_us render_submit_us\n"
        ),
        samples.len(),
        elapsed_ms,
        fps,
        column(|sample| sample.total_us),
        column(|sample| sample.update_us),
        column(|sample| sample.render_submit_us),
        stage_column(|s| u64::from(s.diff_us)),
        stage_column(|s| u64::from(s.style_us)),
        stage_column(|s| u64::from(s.scan_us)),
        stage_column(|s| u64::from(s.layout_us)),
        stage_column(|s| u64::from(s.paint_us)),
    );
    use std::fmt::Write as _;
    for (index, sample) in samples.iter().enumerate() {
        let _ = writeln!(
            report,
            "{} {} {} {} {}",
            index + 1,
            sample.elapsed_ms,
            sample.timing.total_us,
            sample.timing.update_us,
            sample.timing.render_submit_us
        );
    }
    report
}

/// 触摸相位（与 winit 的 `TouchPhase` 一一对应）。
///
/// # 为什么要有它
///
/// 触控的**取消**与**抬起**必须分开（官方触控指南把「指针取消」单列为一个
/// 事件）：取消时应用**不**该提交点击 —— 用户是被系统抢走的，不是松手了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    /// 手指落下。
    Down,
    /// 手指移动。
    Move,
    /// 手指抬起。
    Up,
    /// 交互被**异常取消**（系统接管手势 / 失焦）。
    Cancel,
}

/// 一条命令的响应（写回 TCP 的形式）。
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// 多行文本（每行一条，不带换行；写出时统一补 `\n`）。
    Text(Vec<String>),
    /// 二进制载荷（目前只有 BMP）+ 一行说明。
    Binary { note: String, data: Vec<u8> },
    /// 失败。错误文案要能**直接指认原因**（路径不对 / 后端不支持），
    /// 与本仓「报错质量」的一贯要求一致。
    Err(String),
}

/// 宿主实现的执行面。桥只认识这个 trait，不认识窗口与 GPU。
pub trait BridgeHost {
    /// 视口尺寸（物理像素，宽，高）。
    fn viewport(&self) -> (f32, f32);

    /// 布局树 dump。**绝对**坐标（点击注入用的就是同一坐标系）。
    fn tree_dump(&self) -> String;

    /// 指定路径节点的属性表；路径无效返回 `None`。
    fn props_dump(&self, path: &[usize]) -> Option<String>;

    /// 最近一帧的统计。
    ///
    /// 结构体而不是字符串：文本形态与 JSON 形态从同一份数据出（见 [`FrameStats`]）。
    fn stats(&self) -> FrameStats;

    /// 布局树根节点。
    ///
    /// 桥侧按 key / path 定位元素靠它。默认 `None` = 本后端不提供布局树
    /// （那种后端上按 key 点击不可用，命令会明确报错而不是静默无动作）。
    fn root(&self) -> Option<&zephyra_core::node::Node> {
        None
    }

    /// 按元素 key 找元素中心（物理像素）。
    ///
    /// 默认实现用 [`Self::root`] 与 [`nodes_by_key_name`]。
    fn center_by_key(&self, key: &str) -> Option<(f32, f32)> {
        let root = self.root()?;
        let node = nodes_by_key_name(root, key).into_iter().next()?;
        Some(node_center(node))
    }

    /// 按布局树路径找元素中心（物理像素）。
    ///
    /// 默认实现用 [`Self::root`] 与 [`node_at_path`]。
    fn center_by_path(&self, path: &[usize]) -> Option<(f32, f32)> {
        let root = self.root()?;
        let node = node_at_path(root, path)?;
        Some(node_center(node))
    }

    /// 进程与图形设备的资源占用；后端无法读取时返回可说明原因的错误。
    fn resources(&mut self) -> Result<String, String> {
        Err("此后端未提供资源遥测".into())
    }

    /// 注入指针移动。
    fn inject_hover(&mut self, x: f32, y: f32);

    /// 注入一次完整点击（move → down → up）。
    fn inject_click(&mut self, x: f32, y: f32);

    /// 按下主指针按钮。
    fn inject_pointer_down(&mut self);

    /// 抬起主指针按钮。
    fn inject_pointer_up(&mut self);

    /// 启动有限时长的性能采样。
    fn profile_start(&mut self, duration_seconds: u64) -> Result<(), String> {
        let _ = duration_seconds;
        Err("此后端未提供帧性能分析".into())
    }

    /// 返回采样状态；完成后返回逐帧报告。
    fn profile_report(&mut self) -> Result<String, String> {
        Err("此后端未提供帧性能分析".into())
    }

    /// 注入一个逻辑键的完整按下/抬起。
    fn inject_key(&mut self, key: &str);

    /// 注入滚轮事件，增量方向与 winit 一致（正 y 向上）。
    fn inject_scroll(&mut self, x: f32, y: f32, delta_x: f32, delta_y: f32);

    /// 注入一次**触摸**（多点触控的主入口）。
    ///
    /// # 为什么它是宿主的事
    ///
    /// 触控的「接触几何」与宿主有关（winit 给的是**物理**像素），而多点触控
    /// 的**手指身份**要跨事件保持 —— 那是宿主与 OS 之间的事。
    ///
    /// 默认：报错（此后端不支持触摸注入）。
    fn inject_touch(&mut self, phase: TouchPhase, id: u64, x: f32, y: f32) -> Result<(), String> {
        let _ = (phase, id, x, y);
        Err("此后端不支持触摸注入".into())
    }

    /// 注入一段**文字**（逐字符）。
    ///
    /// 与 [`Self::inject_key`] 的区别：按键是「按了某个键」（可能被快捷键吃掉），
    /// 文字是「文档里出现了这些字符」（文本框该收的那个）。
    fn inject_text(&mut self, text: &str);

    /// 应用**渲染过**的帧数（宿主自己的计数）。
    ///
    /// `wait_frames` 按它算「等到了没有」。默认 0 = 本后端不提供计数，
    /// 于是 `wait_frames` 只能等到客户端超时（10 秒）—— 那也是可接受的失败形态，
    /// 但两个宿主都实现了它，所以实际不会走到这条。
    fn rendered_frames(&self) -> u64 {
        0
    }

    /// 宿主身份——多实例时用来**认人**。
    ///
    /// 默认形态是「进程号 + 视口」：开发时常常同时开着两个实例（旧构建还活着、
    /// 新构建又起了一个），而端口是人为分配的 —— 光看端口认不出谁是谁，
    /// 进程号能。
    fn identity(&self) -> String {
        let (w, h) = self.viewport();
        format!("pid={} viewport={w:.0}x{h:.0}", std::process::id())
    }

    /// 改窗口的**客户区**尺寸（物理像素）。
    ///
    /// # 为什么 AI 需要它
    ///
    /// 「这个布局在窄窗口下还成立吗」是 UI 框架最常见的自查项，而它**只能**
    /// 真的改窗口大小来看 —— 靠读样式表猜不出来（弹性布局、换行、裁切都
    /// 要真排一遍）。
    ///
    /// 默认：报错（此后端不支持改窗口）。
    fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        let _ = (width, height);
        Err("此后端不支持改窗口尺寸".into())
    }

    /// **立即**渲一帧并读回像素，返回 BMP 字节。后端不支持读回时返回 `None`。
    ///
    /// 「立即」的含义：不能等下一帧——AI 发 `shot` 时界面往往静止
    /// （`dirty == false`），等帧循环自己跑就永远等不到。
    ///
    /// `opts.repaint` = 强制整帧重画；默认（`false`）沿用本帧脏区，
    /// 于是残影（脏区外保留的上一帧像素）**看得见**。见 [`ShotOptions`]。
    fn screenshot(&mut self, opts: ShotOptions) -> Option<Vec<u8>>;

    /// 桥客户端**消失之后**的一次性收尾。
    ///
    /// 为什么需要它：AI 在 `pointer_drag` 的按下与抬起之间把 MCP server 杀掉
    /// （超时/崩溃/会话取消都是常态），应用会永远卡在「指针按住」的状态——
    /// 后续真实用户的输入全部被当成拖动的一部分。桥检测到连接消失时调用
    /// 一次（下一帧起就不再调），宿主应当在此补一次抬起并标脏。
    ///
    /// 默认空实现：没有按下态概念的宿主什么都不用做。
    fn on_bridge_disconnect(&mut self) {}
}

/// 调试桥的显式配置（不依赖环境变量）。
///
/// `from_env()` 只是「从环境变量造一份配置」的快捷方式；IDE 直接启动、
/// 嵌入式宿主等场景可以用 [`DebugBridge::with_config`] 直接声明连入 MCP，
/// 包括 `hello` 帧里的应用身份。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BridgeConfig {
    /// 监听端口（旧模式：客户端连进来）。`None` = 不监听。
    pub listen_port: Option<u16>,
    /// 连入 MCP 的地址（连出模式：应用连过去）。`None` = 不连出。
    pub connect_to: Option<SocketAddr>,
    /// 应用名（hello 帧）。`None` = 取可执行文件名。
    pub app_name: Option<String>,
    /// 应用版本（hello 帧）。`None` = 取框架版本。
    pub app_version: Option<String>,
}

impl BridgeConfig {
    /// 从环境变量读配置：与历史 `from_env()` 完全相同的两条拓扑。
    pub fn from_env() -> Self {
        fn parse_port(raw: Option<String>) -> Option<u16> {
            raw.and_then(|s| {
                if s.is_empty() {
                    Some(DEFAULT_PORT)
                } else {
                    s.parse().ok()
                }
            })
        }
        let debug_raw = std::env::var(PORT_ENV).ok();
        let mcp_raw = std::env::var(MCP_PORT_ENV).ok();
        let listen_port = if debug_raw.is_some() || mcp_raw.is_some() {
            parse_port(debug_raw)
        } else {
            None
        };
        let connect_to = mcp_raw
            .as_ref()
            .and_then(|_| parse_port(mcp_raw.clone()))
            .map(|port| SocketAddr::from(([127, 0, 0, 1], port)));
        Self {
            listen_port,
            connect_to,
            app_name: None,
            app_version: None,
        }
    }
}

/// 调试桥的连接状态。
///
/// 旧模式（只监听 `PORT_ENV`）恒为 [`BridgeState::Inactive`] —— 没有「连出去」
/// 这回事。连出模式下三态对宿主有用：拨号在途要快唤醒回收连接线程的结果，
/// 退避等待 50 ms 就够，连上后按数据密度选 Poll / 10 ms。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeState {
    /// 未启用连出（旧模式，或桥未开启）。
    Inactive,
    /// 连出模式：连接线程在途。
    Connecting,
    /// 连出模式：至少一条流连着。
    Connected,
    /// 连出模式：连过但现在断了，等退避重试（MCP 重启是常态）。
    Reconnecting,
}

/// 调试桥。`from_env` 开启；每帧 [`Self::poll`]。
///
/// 两种拓扑共用同一套 `Connection` 协议（谁拨号不影响「读命令、回响应」）：
///
/// - **旧模式**：监听 `ZEPHYRA_DEBUG_PORT`，客户端连进来（`listener`）。
/// - **MCP 模式**：`ZEPHYRA_MCP_PORT` 由 CLI 注入，应用连进 MCP（`link`）。
pub struct DebugBridge {
    /// 旧模式的监听器。MCP 模式下仍可同时存在（`ZEPHYRA_DEBUG_PORT` 也设了），
    /// 这样 `zephyra bridge` / `perf` / `debug` 这些直连工具不受影响。
    listener: Option<TcpListener>,
    /// MCP 模式的连出状态。
    link: Option<OutboundLink>,
    /// 当前挂着的客户端连接。
    ///
    /// # 为什么是**多个**
    ///
    /// 原来只有一个槽（v1 的设计：「AI 客户端一次一个」）。那在「一个客户端
    /// 问、另一个客户端答」的场景下**直接堵死** —— 而调试器恰恰要那样用：
    /// 一边 `debug watch` 盯着某个属性，一边用 `bridge click` 去点。
    ///
    /// 实测症状：watcher 占着连接，于是点击那条命令**根本发不过去** ——
    /// 表现是「点了没反应」，而应用那边一切正常。
    ///
    /// 多连接之后，「盯着看」与「动手改」可以同时进行。
    conns: Vec<Connection>,
}

/// 同时挂着的客户端上限。
///
/// # 为什么有上限
///
/// 每个连接占一条 TCP 流 + 两个缓冲。调试设施不该被无限连接拖垮 ——
/// 而「连了 8 个还没断」几乎一定是有人跑了个脚本在重连。
const MAX_CLIENTS: usize = 8;

/// `wait_frames` 最多等这么久（应用不出帧时就到顶）。
///
/// ⚠ 比客户端的 10 秒读超时短：宁可**回一句实话**（「应用没画那么多帧」），
/// 也不要让客户端撞超时 —— 后者的错误信息看不出是「应用静止」还是「桥挂了」。
///
/// 2 秒的依据：排查动画时等的是 250~500ms 级的动画（开关把手、主题淡入）；
/// 而「应用静止」这件事几毫秒内就能看出来（没有新帧）—— 但**不能**立刻断定，
/// 因为应用可能只是慢。所以给 2 秒。
const WAIT_FRAMES_TIMEOUT: Duration = Duration::from_secs(2);

/// 「应用静止了」的判据：这么长的时间里**一帧都没出**。
///
/// # 为什么要有它
///
/// AI 的典型用法是「点一下 → 等画面稳定 → 截图」。而**静止**的应用
/// （点完就没事了）**永远出不了那么多帧** —— 于是每次 `wait_frames` 都要
/// 等满 [`WAIT_FRAMES_TIMEOUT`]（2 秒）。一个排查会话几十次调用，就是
/// 一分多钟纯等待。
///
/// 250 ms 的依据：60fps 的一帧是 16.7ms，250ms ≈ 15 帧。**慢**于这个帧率的
/// 应用（< 4fps）在交互排查里基本等于「卡住了」，而「卡住了」也正是
/// 「静止了」这个判断要表达的事。
const IDLE_WINDOW: Duration = Duration::from_millis(250);

/// `wait_frames` 的登记信息。
struct PendingWait {
    /// 目标渲染帧数。
    target: u64,
    /// 登记时的渲染帧数（用来报「等到了几帧」）。
    start: u64,
    /// 登记时刻。
    since: Instant,
    /// 最近一次**有进展**的时刻（帧数增加时更新）。
    last_progress: Instant,
    /// 最近一次看到的渲染帧数。
    last_rendered: u64,
}

struct Connection {
    stream: TcpStream,
    /// 半行缓冲：非阻塞读到的字节先攒在这，凑齐一行才解析。
    buf: Vec<u8>,
    /// 非阻塞写缓冲：大截图可能超过 TCP 发送缓冲，须跨帧续写。
    write_buf: Vec<u8>,
    write_offset: usize,
    /// `wait_frames` 的延迟应答。
    ///
    /// ⚠ **每个连接一份**：两个客户端各等各的帧数 —— 共用一份会让
    /// 「A 等 5 帧、B 等 1 帧」时，B 的应答被 A 的等待吞掉（或者反过来）。
    pending_wait: Option<PendingWait>,
}

/// 连出状态。连接放在一次性线程里做，绝不阻塞帧循环；结果经 channel 回收。
struct OutboundLink {
    addr: SocketAddr,
    rx: mpsc::Receiver<std::io::Result<TcpStream>>,
    /// 有连接线程在途 —— 避免每帧都起一个线程。
    connecting: bool,
    /// 下次发起连接的时刻。MCP 还没起来时退避重试，不要每帧试。
    next_try: Instant,
    /// 连上后第一帧：应用身份。MCP 据此区分「哪个应用 / 哪版框架」。
    hello: String,
}

/// 连出失败后的重试间隔。
const CONNECT_RETRY: Duration = Duration::from_millis(500);

/// `hello` 帧：应用连入 MCP 后主动报的身份。
///
/// 格式 `hello <app-name> <version>\n`。app-name 取可执行文件名，version 取
/// 框架自身版本 —— 无需应用额外配置就能区分「哪个应用 / 哪版框架」，也给
/// P2 的显式 builder API 留了升级路径（那时改成应用自己声明）。
fn hello_line(name: Option<&str>, version: Option<&str>) -> String {
    let name = name.map(str::to_string).unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "unknown".into())
    });
    let version = version
        .map(str::to_string)
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").into());
    format!("hello {name} {version}\n")
}

impl OutboundLink {
    fn new(addr: SocketAddr) -> Self {
        Self::with_hello(addr, hello_line(None, None))
    }

    fn with_hello(addr: SocketAddr, hello: String) -> Self {
        let mut this = Self {
            addr,
            // 立刻被 spawn 换掉；这里的 receiver 只为了满足字段初始化。
            rx: mpsc::channel::<std::io::Result<TcpStream>>().1,
            connecting: false,
            next_try: Instant::now(),
            hello,
        };
        this.spawn();
        this
    }

    fn spawn(&mut self) {
        let (tx, rx) = mpsc::channel();
        let addr = self.addr;
        std::thread::spawn(move || {
            // 本机回环 300ms 足够；超时就报失败，poll 会退避重试。
            let result =
                TcpStream::connect_timeout(&addr, Duration::from_millis(300)).and_then(|s| {
                    s.set_nodelay(true)?;
                    s.set_nonblocking(true)?;
                    Ok(s)
                });
            let _ = tx.send(result);
        });
        self.rx = rx;
        self.connecting = true;
    }

    /// 每帧一次：收连接结果；未在连接且过了退避时刻就起一次新连接。
    fn poll(&mut self, conns: &mut Vec<Connection>) {
        if self.connecting {
            match self.rx.try_recv() {
                Ok(Ok(stream)) => {
                    let before = conns.len();
                    push_connection(conns, stream);
                    if conns.len() > before {
                        // 真的挂上了：身份必须赶在任何命令之前发出，MCP 侧在
                        // 第一次 call 前先读掉它，否则响应解析会错位。
                        if let Some(conn) = conns.last_mut() {
                            conn.write_buf.extend_from_slice(self.hello.as_bytes());
                        }
                        self.connecting = false;
                    } else {
                        // 槽位满：push_connection 静默丢弃了这条流。不能当成连上 ——
                        // 否则 connecting 清空、next_try 还是创建时的过去时刻，
                        // 下一帧立刻再 spawn 一个 connect_timeout 线程，形成风暴。
                        self.connecting = false;
                        self.next_try = Instant::now() + CONNECT_RETRY;
                    }
                }
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                    self.connecting = false;
                    self.next_try = Instant::now() + CONNECT_RETRY;
                }
                Err(TryRecvError::Empty) => {}
            }
        } else if Instant::now() >= self.next_try {
            self.spawn();
        }
    }

    /// 已连上的流断了：安排重连（MCP 重启是 dev 工作流常态）。
    fn disconnected(&mut self) {
        if !self.connecting {
            self.next_try = Instant::now() + CONNECT_RETRY;
        }
    }
}

/// 把一条已建好的流挂成连接 —— 两种拓扑共用（谁拨号不影响协议方向）。
fn push_connection(conns: &mut Vec<Connection>, stream: TcpStream) {
    if conns.len() >= MAX_CLIENTS {
        return;
    }
    conns.push(Connection {
        stream,
        buf: Vec::new(),
        write_buf: Vec::new(),
        write_offset: 0,
        pending_wait: None,
    });
}

impl DebugBridge {
    /// 绑定 `port` 并置非阻塞。绑定失败（端口被占）由调用方决定怎么报——
    /// 桥是调试设施，**不该**让应用起不来。
    pub fn bind(port: u16) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener: Some(listener),
            link: None,
            conns: Vec::new(),
        })
    }

    /// 按显式配置开启桥。
    ///
    /// 两种拓扑可并存（监听 + 连出）；都没有 ⇒ `None`（「调试桥默认关闭」）。
    /// 端口绑定失败只报诊断，不让应用起不来 —— 与 `from_env()` 同款容错。
    pub fn with_config(config: BridgeConfig) -> Option<Self> {
        if config.listen_port.is_none() && config.connect_to.is_none() {
            return None;
        }

        let listener =
            config
                .listen_port
                .and_then(|port| match TcpListener::bind(("127.0.0.1", port)) {
                    Ok(l) => {
                        l.set_nonblocking(true).ok();
                        eprintln!("[zephyra] 调试桥已开启 127.0.0.1:{port}（显式配置）");
                        Some(l)
                    }
                    Err(e) => {
                        eprintln!("[zephyra] 调试桥绑定 {port} 失败（{e}）——无桥继续运行");
                        None
                    }
                });

        let link = config.connect_to.map(|addr| {
            eprintln!("[zephyra] 调试桥连入 MCP {addr}（显式配置）");
            OutboundLink::with_hello(
                addr,
                hello_line(config.app_name.as_deref(), config.app_version.as_deref()),
            )
        });

        if listener.is_none() && link.is_none() {
            return None;
        }
        Some(Self {
            listener,
            link,
            conns: Vec::new(),
        })
    }

    /// 按环境变量开启桥（见 [`BridgeConfig::from_env`]）。
    ///
    /// - `ZEPHYRA_MCP_PORT`（CLI 注入）⇒ 应用**连入 MCP**，agent 只经 stdio 与 MCP 说话；
    /// - `ZEPHYRA_DEBUG_PORT` ⇒ 旧模式，监听端口等客户端连进来。
    ///
    /// 两者可以同时设置：MCP 通道 + 直连的 `bridge` / `perf` / `debug` 工具并存。
    /// 都没设置返回 `None` ——「调试桥默认关闭」的唯一入口，不存在隐式监听。
    pub fn from_env() -> Option<Self> {
        Self::with_config(BridgeConfig::from_env())
    }

    /// 每帧一次：接受连接、读可用字节、把凑齐的行解析成命令并执行。
    ///
    /// 全程**非阻塞**：没有连接、没有数据、对端半行，都直接返回，
    /// 帧循环一微秒都不多等。
    pub fn poll(&mut self, host: &mut dyn BridgeHost) {
        let had_client = self.has_client();
        self.poll_inner(host);
        // 客户端**全部**消失：给宿主一次收尾机会（补抬指针之类）。
        //
        // ⚠ 判据是「**一个都不剩**」而不是「少了一个」：多个客户端时，
        // 「A 正拖着、B 断了」不该把 A 的按下态清掉 —— 那是 B 的锅，
        // 不是 A 的。真正需要收尾的是「调试器整个没人了」。
        if had_client && !self.has_client() {
            host.on_bridge_disconnect();
        }
    }

    /// `poll` 的实现体。拆出来只为让外层能在返回后统一判断连接是否消失
    /// （体内有多个 `return`，在体内加钩子必然漏分支）。
    fn poll_inner(&mut self, host: &mut dyn BridgeHost) {
        // 旧模式：接受新连接（最多 [`MAX_CLIENTS`] 个）。
        //
        // ⚠ 原来只在「没有客户端」时 accept —— 于是第二个客户端只能挂在
        // backlog 里干等，而它的命令**一条也进不来**。
        if let Some(listener) = &self.listener {
            while self.conns.len() < MAX_CLIENTS {
                let Ok((stream, _addr)) = listener.accept() else {
                    // WouldBlock = 没有新连接。
                    break;
                };
                let _ = stream.set_nonblocking(true);
                let _ = stream.set_nodelay(true);
                push_connection(&mut self.conns, stream);
            }
        }

        // MCP 模式：非阻塞回收连出线程的结果；断了会在下一帧退避重连。
        if let Some(link) = &mut self.link {
            link.poll(&mut self.conns);
        }

        // 逐个连接处理。死掉的连接记下来，最后**倒序**移除（否则下标会错位）。
        let mut dead: Vec<usize> = Vec::new();
        for (index, conn) in self.conns.iter_mut().enumerate() {
            if !poll_connection(conn, host) {
                dead.push(index);
            }
        }
        if !dead.is_empty() {
            if let Some(link) = &mut self.link {
                link.disconnected();
            }
        }
        for index in dead.into_iter().rev() {
            self.conns.remove(index);
        }
    }

    /// 当前是否挂着客户端。
    ///
    /// 事件驱动的宿主（D3D12）拿它决定控制流：有客户端切 Poll（TCP 数据
    /// 唤不醒 Wait 模式），无客户端回 Wait（静止零开销不动）。
    pub fn has_client(&self) -> bool {
        !self.conns.is_empty()
    }

    /// 当前连接状态，区分连出模式的三态。
    ///
    /// 旧模式（只有 `listener`）恒为 [`BridgeState::Inactive`]；只有连出模式才有
    /// Connecting / Connected / Reconnecting 之分。
    pub fn state(&self) -> BridgeState {
        let Some(link) = &self.link else {
            return BridgeState::Inactive;
        };
        if !self.conns.is_empty() {
            BridgeState::Connected
        } else if link.connecting {
            BridgeState::Connecting
        } else {
            BridgeState::Reconnecting
        }
    }

    /// 连出模式下是否已连上（有流）。旧模式恒为 `false`。
    pub fn is_connected(&self) -> bool {
        self.state() == BridgeState::Connected
    }

    /// 连出模式下连接线程是否在途。旧模式恒为 `false`。
    ///
    /// 宿主据此把「尚未连上」期间的唤醒档位从盲目的 50 ms 细化到 10 ms ——
    /// 连接线程的结果经 channel 回收，唤醒越快发现得越早。
    pub fn is_connecting(&self) -> bool {
        self.state() == BridgeState::Connecting
    }

    /// 桥上有**没读完的数据**或**没写完的响应**。
    ///
    /// # 为什么要有它（以及为什么不能只看 [`Self::has_client`]）
    ///
    /// 宿主的控制流只有两档：`Poll`（立刻返回，事件循环**全速空转**）与
    /// `WaitUntil(t)`（睡到 t）。TCP 数据**唤不醒** Wait —— 所以「有客户端」
    /// 就意味着「只能 Poll」，否则命令要等到下一次唤醒。
    ///
    /// 实测（D3D12 宿主 + 一个连着的 `zephyra perf profile`）：Poll 模式把
    /// 事件循环拉到 **2500 Hz**，而应用其实只画了 **9 帧** —— 白烧 CPU，
    /// 还让性能采样把「事件循环 tick」当成「帧」（见下）。
    ///
    /// 所以判据分成两档：
    ///
    /// - **有未处理的数据** ⇒ Poll（数据在流，全速才对）；
    /// - **有客户端但没数据** ⇒ `WaitUntil(now + 10 ms)`（命令最多晚 10 ms，
    ///   而 CPU 从「全速空转」降到 1% 量级）。
    ///
    /// ⚠ 判据只能是「**缓冲区里有没有字节**」—— 数据到达时主机**不知道**，
    /// 只能等下一次醒来去读。这就是那 10 ms 的来源。
    pub fn has_pending_work(&self) -> bool {
        self.conns
            .iter()
            .any(|c| !c.buf.is_empty() || !c.write_buf.is_empty())
    }
}

/// 处理**一个**连接的一帧工作。返回 `false` = 这个连接死了（该移除）。
///
/// # 为什么不写成 `DebugBridge` 的方法
///
/// 它只需要连接自己的状态（读缓冲、写缓冲、`wait_frames` 登记），不碰桥的
/// 其余部分 —— 写成自由函数就没有「同时可变借用 `self.conns` 与 `self`」
/// 的问题。多连接之后那个借用冲突是**真的**：方法体里 `self.conns.iter_mut()`
/// 期间不能再碰 `self` 的别的字段。
fn poll_connection(conn: &mut Connection, host: &mut dyn BridgeHost) -> bool {
    match flush_response(conn) {
        Ok(true) => {}
        // 还有没写完的（大截图）：下一帧继续。
        Ok(false) => return true,
        Err(e) => {
            eprintln!("[zephyra] 调试桥响应发送失败: {e}");
            return false;
        }
    }

    // 读可用字节（非阻塞：WouldBlock = 本帧没有新命令）。
    let mut chunk = [0u8; 4096];
    loop {
        match conn.stream.read(&mut chunk) {
            Ok(0) => {
                // 对端关了。丢弃缓冲（半行永远等不齐了）。
                return false;
            }
            Ok(n) => {
                conn.buf.extend_from_slice(&chunk[..n]);
                if conn.buf.len() > 64 * 1024 {
                    // 没有换行的洪泛：断开。调试桥不该被喂死。
                    return false;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => return false,
        }
    }

    // 逐条处理凑齐的行。半行留在缓冲里等下一帧。
    while let Some(pos) = conn.buf.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = conn.buf.drain(..=pos).collect();
        let line = String::from_utf8_lossy(&line[..line.len() - 1]); // 去 \n
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let resp = match parse_command(line) {
            // `wait_frames` 要**延迟应答**：等应用真的画够 N 帧再回话。
            // 这里只登记目标帧数，回应在下面的「pending」检查里发。
            Ok(Command::WaitFrames { frames }) => {
                let start = host.rendered_frames();
                let target = start + u64::from(frames);
                let now = Instant::now();
                conn.pending_wait = Some(PendingWait {
                    target,
                    start,
                    since: now,
                    last_progress: now,
                    last_rendered: start,
                });
                continue;
            }
            Ok(cmd) => execute(&cmd, host),
            Err(e) => Response::Err(e),
        };
        conn.write_buf.extend_from_slice(&response_bytes(&resp));
        match flush_response(conn) {
            Ok(true) => {}
            Ok(false) => return true,
            Err(e) => {
                eprintln!("[zephyra] 调试桥响应发送失败: {e}");
                return false;
            }
        }
    }

    // 延迟应答：`wait_frames` 的回应在这里发。
    //
    // 三种结局，每一种都说清楚实情：
    //
    // 1. **达标**（帧数到了）⇒ 等到了。
    // 2. **应用静止**（[`IDLE_WINDOW`] 内一帧都没出）⇒ 画面已经稳定，
    //    没等到更多帧是因为**没有动画在跑**。这是最常见的一种 ——
    //    AI 点完一个按钮就想截图，而点击**不产生动画**。
    // 3. **超时** ⇒ 应用在这段时间里既没画够、也不算静止（极慢或卡住）。
    //
    // 一直吊着不回应只会让客户端撞上 10 秒读超时，错误信息还更难看。
    if let Some(wait) = conn.pending_wait.as_mut() {
        let rendered = host.rendered_frames();
        let now = Instant::now();
        let done = if rendered >= wait.target {
            Some(format!(
                "waited {} rendered frames",
                rendered.saturating_sub(wait.start)
            ))
        } else if rendered > wait.last_rendered {
            // 有进展：刷新「静止」判据的起点，继续等。
            wait.last_rendered = rendered;
            wait.last_progress = now;
            None
        } else if now.duration_since(wait.last_progress) >= IDLE_WINDOW {
            Some(format!(
                "the app settled (rendered {} of the requested {} frames, then went idle)",
                rendered.saturating_sub(wait.start),
                wait.target.saturating_sub(wait.start)
            ))
        } else if wait.since.elapsed() > WAIT_FRAMES_TIMEOUT {
            Some(format!(
                "waited only {} of the requested frames (the app did not render that many in {}s)",
                rendered.saturating_sub(wait.start),
                WAIT_FRAMES_TIMEOUT.as_secs()
            ))
        } else {
            None
        };
        if let Some(text) = done {
            conn.pending_wait = None;
            let resp = Response::Text(vec![text]);
            conn.write_buf.extend_from_slice(&response_bytes(&resp));
            let _ = flush_response(conn);
        }
    }
    true
}

// ── 树 / 属性 dump：宿主侧共享实现 ─────────────────────────────────────────
//
// 两个后端（D3D11 / D3D12）都要回答 `tree` / `props`，而它们互不依赖——逻辑
// 只能有一份，落点就是这里（fx 依赖 core，`Node` 可见；两个宿主都依赖 fx）。
// 与 `zephyra_fx::quad` 的抽取同一论证：契约一致性靠「只有一份」保证，
// 不靠「记得同步两份」。

use std::fmt::Write as _;

/// 单节点属性的 JSON 形态（`props <path> --json`）。
pub fn props_json_of(node: &zephyra_core::node::Node) -> String {
    let r = node.arranged_rect();
    let mut out = String::from("{");
    out.push_str(&format!(
        "\"tag\":{}",
        json_string(&format!("{:?}", node.tag()))
    ));
    match node.key().and_then(|k| k.as_name()) {
        Some(name) => out.push_str(&format!(",\"key\":{}", json_string(name))),
        None => out.push_str(",\"key\":null"),
    }
    out.push_str(&format!(
        ",\"rect\":{{\"x\":{:.0},\"y\":{:.0},\"w\":{:.0},\"h\":{:.0}}}",
        r.x, r.y, r.width, r.height
    ));
    let classes: Vec<String> = node.style_names().iter().map(|c| json_string(c)).collect();
    out.push_str(&format!(",\"classes\":[{}]", classes.join(",")));
    out.push_str(",\"props\":{");
    let mut first = true;
    for (k, v) in node.props().iter_styled() {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!(
            "{}:{}",
            json_string(k.name()),
            json_string(&value_to_text(v))
        ));
    }
    for (k, v) in node.props().iter_local() {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!(
            "{}:{}",
            json_string(k.name()),
            json_string(&value_to_text(v))
        ));
    }
    out.push_str("}}");
    out
}

/// 布局树 dump（`tree` 命令的正文）。
///
/// 每行 `path tag key absx,absy w*h [styles...]`：
///
/// - **path**（根节点 `0` 加子下标链，如 `0.1.2`）是 `props` 命令的钥匙；
/// - 坐标已累成**绝对**值——`hover`/`click` 用的就是这一坐标系，
///   AI 从树里抄一个坐标就能点，不需要自己做相对/绝对换算；
/// - 缩进只为人读，解析靠 path 列。
pub fn tree_dump_of(root: &zephyra_core::node::Node) -> String {
    fn rec(node: &zephyra_core::node::Node, path: String, out: &mut String) {
        let r = node.arranged_rect();
        let key = match node.key() {
            Some(k) => format!("{k}"),
            None => "-".into(),
        };
        let _ = write!(
            out,
            "{path} {:?} {key} {:.0},{:.0} {:.0}x{:.0}",
            node.tag(),
            r.x,
            r.y,
            r.width,
            r.height
        );
        for name in node.style_names() {
            let _ = write!(out, " {name}");
        }
        // ⚠ 文字内容**必须**在树里：AI 看到的是「屏幕上写着什么」，而不是
        // 「这个节点的 key 叫什么」。没有它，AI 只能对每个文本节点逐个调
        // `props` 才看得见内容 —— 那是一次 O(n) 次往返的排查。
        //
        // 放在**最后**且带引号：前面的列（path / tag / key / 坐标 / 类名）
        // 形状不变，老的解析方式还能用。
        if let Some(text) = node_text(node) {
            if !text.is_empty() {
                let _ = write!(out, " {}", escape_inline(text));
            }
        }
        let _ = writeln!(out);
        for (i, child) in node.children().iter().enumerate() {
            let depth = path.matches('.').count() + 1;
            for _ in 0..depth {
                out.push_str("  ");
            }
            rec(child, format!("{path}.{i}"), out);
        }
    }
    let mut out = String::new();
    rec(root, "0".into(), &mut out);
    out
}

/// 节点的文字内容（`props::TEXT`）。
///
/// 没有就是 `None`（非文本节点，或文本为空）。
///
/// # 为什么本地值优先于样式值
///
/// 文本内容总是**本地**的（应用给的），样式表里不会有「这段文字写什么」——
/// 那不是外观，是内容。本地没有才回退到样式层（理论上不该发生，但读到的
/// 东西要完整）。
pub fn node_text(node: &zephyra_core::node::Node) -> Option<&str> {
    use zephyra_core::property::PropertyValue;
    let props = node.props();
    for (k, v) in props.iter_local().chain(props.iter_styled()) {
        // 比**地址**而不是名字：同名属性键可能有多个（`Text` 与 `Text.Size`
        // 是不同的键，但名字前缀相同）。
        if std::ptr::eq(k, &zephyra_core::props::TEXT) {
            return match v {
                PropertyValue::Str(s) => Some(s.as_ref()),
                PropertyValue::StaticStr(s) => Some(s),
                _ => None,
            };
        }
    }
    None
}

/// 文本 → 可安全放进**行协议**的形态（带引号，控制字符转义）。
///
/// # 为什么必须转义 `\n`
///
/// 协议是行文本：一条命令一行。一个**没转义**的换行会把一条命令劈成两条
/// —— 症状是「应用收到了莫名其妙的命令」，而没人知道为什么。
///
/// # 为什么带引号
///
/// 文字里有空格时（几乎总是有），引号是「这是一个整体」的信号；没有它，
/// 读的人只能靠「引号前是最后一个空格」来猜。
pub fn escape_inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 反解 [`escape_inline`]：去引号 + 还原转义。
///
/// 协议里「带引号的一串」是**一个**参数：`type_text "hello world"` 的参数是
/// `hello world`（含空格），不是两个词。
pub fn unescape_inline(s: &str) -> Result<String, String> {
    let s = s.trim();
    let Some(inner) = s.strip_prefix('"') else {
        // 没引号：整体就是参数（允许含空格 —— 调用方自己负责不把行拆开）。
        return Ok(s.to_string());
    };
    let Some(inner) = inner.strip_suffix('"') else {
        return Err("带引号的参数没有闭合引号".into());
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(esc) = chars.next() else {
            return Err("转义符后面没有字符".into());
        };
        match esc {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            'u' => {
                let hex: String = chars.by_ref().take(4).collect();
                if hex.len() != 4 {
                    return Err(format!("\\u 转义需要 4 位十六进制，实得 `{hex}`"));
                }
                let code = u32::from_str_radix(&hex, 16)
                    .map_err(|_| format!("\\u{hex} 不是合法十六进制"))?;
                let ch =
                    char::from_u32(code).ok_or_else(|| format!("\\u{hex} 不是合法字符码点"))?;
                out.push(ch);
            }
            other => return Err(format!("未知转义 `\\{other}`")),
        }
    }
    Ok(out)
}

/// 按布局树路径取节点。首段 `0` 表示根节点，后续段是逐层子下标。
pub fn node_at_path<'a>(
    root: &'a zephyra_core::node::Node,
    path: &[usize],
) -> Option<&'a zephyra_core::node::Node> {
    let (&root_index, children) = path.split_first()?;
    if root_index != 0 {
        return None;
    }
    children
        .iter()
        .try_fold(root, |node, &index| node.children().get(index))
}

/// 按**元素 key** 找节点：返回**全部**同名节点（文档序）。
///
/// # 为什么返回列表而不是一个
///
/// 重复 key 是**允许**的（`For` 的位置化 item 就可能同名），所以「按 key 定位」
/// 天然有歧义。返回全部让调用方能**说出**歧义，而不是悄悄取第一个 ——
/// 「点了另一个同名元素」这类症状最难查。
///
/// 只认 `ElementKey::Name` 形式：整数 id 的 key 没有可读的字符串形态，
/// 按名字找它没有意义（那种元素只能用路径定位）。
pub fn nodes_by_key_name<'a>(
    root: &'a zephyra_core::node::Node,
    name: &str,
) -> Vec<&'a zephyra_core::node::Node> {
    fn rec<'a>(
        node: &'a zephyra_core::node::Node,
        name: &str,
        out: &mut Vec<&'a zephyra_core::node::Node>,
    ) {
        if node.key().and_then(|k| k.as_name()) == Some(name) {
            out.push(node);
        }
        for child in node.children() {
            rec(child, name, out);
        }
    }
    let mut out = Vec::new();
    rec(root, name, &mut out);
    out
}

/// 节点中心（绝对坐标）——注入指针用的落点。
///
/// 用**布局盒**而不是视觉盒：视觉盒含绘制变换（translate / scale），
/// 而变换后的形状由渲染器自己算；落点只要在元素**里面**即可。
pub fn node_center(node: &zephyra_core::node::Node) -> (f32, f32) {
    let r = node.arranged_rect();
    (r.x + r.width * 0.5, r.y + r.height * 0.5)
}

/// 布局树的 JSON 形态（`tree --json`）。
///
/// # 为什么要 JSON 形态
///
/// 文本形态是给人读的；工具（脚本、另一个程序）要解析它就得写正则 ——
/// 而正则解析一个**格式会变**的文本，是那种「今天能用、改一行就悄悄坏掉」的东西。
/// JSON 形态从**同一棵树**直接生成，没有解析这一步。
///
/// 一行一个节点对象（不是数组）：行式 JSON 可以边读边处理，不必等全文。
pub fn tree_json_of(root: &zephyra_core::node::Node) -> String {
    fn rec(node: &zephyra_core::node::Node, path: String, out: &mut String) {
        let r = node.arranged_rect();
        let key = match node.key().and_then(|k| k.as_name()) {
            Some(name) => json_string(name),
            None => "null".into(),
        };
        let classes: Vec<String> = node.style_names().iter().map(|c| json_string(c)).collect();
        // 文字内容：与文本形态同源（`node_text`），不是另抄一份。
        let text = match node_text(node) {
            Some(t) if !t.is_empty() => json_string(t),
            _ => "null".into(),
        };
        let _ = writeln!(
            out,
            "{{\"path\":{},\"tag\":{},\"key\":{},\"text\":{},\"x\":{:.0},\"y\":{:.0},\"w\":{:.0},\"h\":{:.0},\"classes\":[{}]}}",
            json_string(&path),
            json_string(&format!("{:?}", node.tag())),
            key,
            text,
            r.x,
            r.y,
            r.width,
            r.height,
            classes.join(",")
        );
        for (i, child) in node.children().iter().enumerate() {
            rec(child, format!("{path}.{i}"), out);
        }
    }
    let mut out = String::new();
    rec(root, "0".into(), &mut out);
    out
}

/// 一处**按文字找到**的元素。
///
/// # 为什么带 `owner`
///
/// AI 想点的是「写着『快速开始』的那个导航项」，而树里那行是个 `Text` 节点
/// —— 点它多半没反应（真正可点的是它的祖先控件）。`owner` 就是**最近的可点
/// 祖先**（含自己）：带 key、且**不是**文本节点的那个。
#[derive(Debug, Clone, PartialEq)]
pub struct FoundText {
    /// 文本节点自己的路径。
    pub path: String,
    /// 命中的文字。
    pub text: String,
    /// 最近的可点祖先路径（含自身）；没有则 `None`。
    pub owner: Option<String>,
    /// 文本节点的矩形（绝对坐标）。
    pub rect: zephyra_core::core::Rect,
}

/// 按文字找元素（`find` 命令）。
///
/// # 判据
///
/// 文字**包含**查询串（不是等于）—— AI 常问「那个写着『确定』的按钮」，
/// 而按钮文字是「确定删除」。大小写不敏感（英文界面友好；中文无影响）。
///
/// # 为什么顺带给出 `owner`
///
/// 见 [`FoundText::owner`]：文字节点通常不是可点的那个。
pub fn find_text_of(root: &zephyra_core::node::Node, query: &str) -> Vec<FoundText> {
    let needle = query.to_lowercase();
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    fn rec(
        node: &zephyra_core::node::Node,
        path: String,
        owner: Option<&str>,
        needle: &str,
        out: &mut Vec<FoundText>,
    ) {
        // 自己可点（带 key 且不是纯文本节点）⇒ 自己就是 owner（给子节点用）。
        let keyed = node.key().is_some();
        let is_text = node.tag() == zephyra_core::id::NodeTag::Text;
        let owner = if keyed && !is_text {
            Some(path.as_str())
        } else {
            owner
        };
        if let Some(text) = node_text(node) {
            if !text.is_empty() && text.to_lowercase().contains(needle) {
                // 命中的那个节点自己可点时，owner 就是它自己。
                let own_owner = if keyed && !is_text {
                    Some(path.clone())
                } else {
                    owner.map(|s| s.to_string())
                };
                out.push(FoundText {
                    path: path.clone(),
                    text: text.to_string(),
                    owner: own_owner,
                    rect: node.arranged_rect(),
                });
            }
        }
        for (i, child) in node.children().iter().enumerate() {
            rec(child, format!("{path}.{i}"), owner, needle, out);
        }
    }
    rec(root, "0".into(), None, &needle, &mut out);
    out
}

/// 命中测试的结果：这个点上**最具体**的那个元素。
#[derive(Debug, Clone, PartialEq)]
pub struct HitResult {
    /// 命中节点的路径。
    pub path: String,
    /// 命中节点的类型标签。
    pub tag: String,
    /// 命中节点的 key（没有则 `None`）。
    pub key: Option<String>,
    /// 命中节点的文字（没有则 `None`）。
    pub text: Option<String>,
    /// **可点的那个**的路径：命中节点自己或祖先里最近的那个 `Interactive = true`。
    ///
    /// # 为什么单列一栏
    ///
    /// 命中 `Text` 节点时，AI 真正要点的是它的祖先控件 —— 而那个祖先的 key
    /// 往往**不唯一**（所有导航项的标签都叫 `label`），所以按 key 点会点错。
    /// 给**路径**才是精确的。
    pub clickable: Option<String>,
}

/// 命中测试：这个点上最具体的元素。
///
/// # 为什么需要它
///
/// AI 手里有一张截图，于是会问「(500, 300) 那里是什么」。没有这条命令，
/// 只能自己扫树 + 算矩形 —— 而那是**猜**（哪个在上？哪个能被点？）。
///
/// # 判据
///
/// 取**最深**的、矩形包含该点的节点；同深度取**文档序靠后**的（后画的在上）。
/// 这是「视觉上最靠前的那层」。
pub fn hit_at(root: &zephyra_core::node::Node, x: f32, y: f32) -> Option<HitResult> {
    fn rec(
        node: &zephyra_core::node::Node,
        path: String,
        depth: usize,
        x: f32,
        y: f32,
        clickable: Option<&str>,
        best: &mut Option<(usize, HitResult)>,
    ) {
        let r = node.arranged_rect();
        let inside = x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
        // 自己可交互 ⇒ 自己就是「可点的那个」（给子节点用）。
        let clickable = if node_bool_prop(node, &zephyra_core::props::INTERACTIVE) {
            Some(path.as_str())
        } else {
            clickable
        };
        if inside {
            let result = HitResult {
                path: path.clone(),
                tag: format!("{:?}", node.tag()),
                key: node.key().and_then(|k| k.as_name()).map(|s| s.to_string()),
                text: node_text(node).map(|s| s.to_string()),
                clickable: clickable.map(|s| s.to_string()),
            };
            // 更深 ⇒ 更具体；同深度时后来者（文档序靠后）盖住先来的。
            let replace = match best {
                None => true,
                Some((best_depth, _)) => depth >= *best_depth,
            };
            if replace {
                *best = Some((depth, result));
            }
        }
        for (i, child) in node.children().iter().enumerate() {
            rec(
                child,
                format!("{path}.{i}"),
                depth + 1,
                x,
                y,
                clickable,
                best,
            );
        }
    }
    let mut best = None;
    rec(root, "0".into(), 0, x, y, None, &mut best);
    best.map(|(_, r)| r)
}

/// 读一个**布尔**属性（本地优先）。
fn node_bool_prop(
    node: &zephyra_core::node::Node,
    key: &zephyra_core::property::PropertyKey,
) -> bool {
    use zephyra_core::property::PropertyValue;
    let props = node.props();
    for (k, v) in props.iter_local().chain(props.iter_styled()) {
        if std::ptr::eq(k, key) {
            return matches!(v, PropertyValue::Bool(true));
        }
    }
    false
}

/// 单节点属性表 dump（`props` 命令的正文）。
///
/// 先样式后本地——与优先级顺序相反（本地赢），读的人「后面盖前面」。
pub fn props_dump_of(node: &zephyra_core::node::Node) -> String {
    let r = node.arranged_rect();
    let mut s = String::new();
    let _ = writeln!(s, "tag {:?}", node.tag());
    if let Some(k) = node.key() {
        let _ = writeln!(s, "key {k}");
    }
    let _ = writeln!(
        s,
        "rect {:.0},{:.0} {:.0}x{:.0} (absolute)",
        r.x, r.y, r.width, r.height
    );
    for name in node.style_names() {
        let _ = writeln!(s, "style {name}");
    }
    for (k, v) in node.props().iter_styled() {
        let _ = writeln!(s, "{} = {} (styled)", k.name(), value_to_text(v));
    }
    for (k, v) in node.props().iter_local() {
        let _ = writeln!(s, "{} = {}", k.name(), value_to_text(v));
    }
    s
}

/// 解析一行命令。纯函数，单测钉在这里。
///
/// 协议（一行的形状）：
///
/// ```text
/// ping
/// tree
/// layout
/// props 0.1.2
/// stats
/// viewport
/// hover 120.5 48
/// click 120.5 48
/// pointer_down
/// pointer_up
/// press_key Enter
/// scroll 120 48 0 -3
/// find 快速开始
/// type_text "hello world"
/// resources
/// profile_start 30
/// profile_report
/// shot
/// ```
pub fn parse_command(line: &str) -> Result<Command, String> {
    let mut parts = line.split_whitespace();
    let cmd = parts.next().ok_or("空命令")?;
    // 余下的**原样**（不按空白重新拼）：引号里的**连续空格**必须保住 ——
    // `type_text "a  b"` 重新拼成 `type_text "a b"` 就会少输一个空格，
    // 而症状是「输入框里少了个空格」，极难归因。
    let rest_raw = line[cmd.len()..].trim_start();
    let rest: Vec<&str> = parts.collect();
    match cmd {
        "ping" => Ok(Command::Ping),
        "tree" => {
            let json = rest.contains(&"--json");
            if rest.iter().any(|a| *a != "--json") {
                return Err("tree 只接受可选的 --json 开关".into());
            }
            Ok(Command::Tree { json })
        }
        "layout" => Ok(Command::Layout),
        "stats" => {
            let json = rest.contains(&"--json");
            if rest.iter().any(|a| *a != "--json") {
                return Err("stats 只接受可选的 --json 开关".into());
            }
            Ok(Command::Stats { json })
        }
        "resources" => Ok(Command::Resources),
        "profile_report" if rest.is_empty() => Ok(Command::ProfileReport),
        "profile_start" => {
            if rest.len() != 1 {
                return Err("profile_start 需要一个采集秒数，范围 1..=300".into());
            }
            let duration_seconds = rest[0]
                .parse::<u64>()
                .map_err(|_| "采集秒数必须是 1..=300 的整数")?;
            if !(1..=300).contains(&duration_seconds) {
                return Err("采集秒数必须在 1..=300 范围内".into());
            }
            Ok(Command::ProfileStart { duration_seconds })
        }
        "viewport" => Ok(Command::Viewport),
        "wait_frames" => {            let Some(n) = rest.first() else {
                return Err("wait_frames 需要一个帧数，如 `wait_frames 2`".into());
            };
            if rest.len() > 1 {
                return Err(format!("wait_frames 只接受一个帧数，收到 {} 个参数", rest.len()));
            }
            let frames: u32 = n
                .parse()
                .map_err(|_| format!("帧数 `{n}` 不是 0..=10000 的整数"))?;
            if frames > 10_000 {
                return Err("帧数不能超过 10000（否则客户端会先超时）".into());
            }
            Ok(Command::WaitFrames { frames })
        }
        "find" => {
            // 查询串是**整行余下部分**（可以含空格）。
            let mut raw = rest_raw;
            let mut json = false;
            // 末尾的 `--json` 是开关，不是查询内容。
            if let Some(stripped) = raw.strip_suffix("--json") {
                json = true;
                raw = stripped.trim_end();
            }
            let query = unescape_inline(raw)?;
            if query.trim().is_empty() {
                return Err("find 需要一个查询串，如 `find 确定`".into());
            }
            Ok(Command::Find { query, json })
        }
        "type_text" => {
            // 参数是**整行余下部分**：带引号时按引号内的内容算（可以含空格）。
            if rest_raw.is_empty() {
                return Err("type_text 需要一段文字，如 `type_text \"hello\"`".into());
            }
            let text = unescape_inline(rest_raw)?;
            if text.is_empty() {
                return Err("type_text 的文字不能是空的".into());
            }
            Ok(Command::TypeText { text })
        }
        "shot" => {
            // `shot [--repaint]`：默认沿用本帧脏区（残影可见）；`--repaint`
            // 强制整帧（要一张干净参考图时用）。
            let repaint = rest.contains(&"--repaint");
            if rest.iter().any(|a| *a != "--repaint") {
                return Err("shot 只接受可选的 --repaint 开关".into());
            }
            Ok(Command::Shot { repaint })
        }
        "props" => {
            let Some(p) = rest.first() else {
                return Err("props 需要一个路径参数，如 `props 0.1.2`".into());
            };
            let json = rest.contains(&"--json");
            if rest.iter().any(|a| *a != "--json" && *a != *p) {
                return Err("props 只接受一个路径与可选的 --json 开关".into());
            }
            parse_path(p).map(|path| Command::Props { path, json })
        }
        "hover" | "click" => {
            if rest.len() != 2 {
                return Err(format!("{cmd} 需要 x y 两个坐标参数，如 `{cmd} 120 48`"));
            }
            let x: f32 = rest[0]
                .parse()
                .map_err(|_| format!("x 坐标 `{}` 不是数字", rest[0]))?;
            let y: f32 = rest[1]
                .parse()
                .map_err(|_| format!("y 坐标 `{}` 不是数字", rest[1]))?;
            if !(x.is_finite() && y.is_finite()) {
                return Err("坐标必须是有限数".into());
            }
            if cmd == "hover" {
                Ok(Command::Hover { x, y })
            } else {
                Ok(Command::Click { x, y })
            }
        }
        "hit" => {
            if rest.len() != 2 {
                return Err("hit 需要 x y 两个坐标参数，如 `hit 500 300`".into());
            }
            let x: f32 = rest[0]
                .parse()
                .map_err(|_| format!("x 坐标 `{}` 不是数字", rest[0]))?;
            let y: f32 = rest[1]
                .parse()
                .map_err(|_| format!("y 坐标 `{}` 不是数字", rest[1]))?;
            if !(x.is_finite() && y.is_finite()) {
                return Err("坐标必须是有限数".into());
            }
            Ok(Command::Hit { x, y })
        }
        "touch" => {
            // `touch <相位> <id> <x> <y>` —— 与真实 `WindowEvent::Touch` 同形。
            if rest.len() != 4 {
                return Err(
                    "touch 需要四个参数：相位（down/move/up/cancel）、指针 id、x、y".into(),
                );
            }
            let phase = match rest[0] {
                "down" => TouchPhase::Down,
                "move" => TouchPhase::Move,
                "up" => TouchPhase::Up,
                "cancel" => TouchPhase::Cancel,
                other => {
                    return Err(format!(
                        "touch 的相位只能是 down/move/up/cancel，实得 `{other}`"
                    ))
                }
            };
            let id: u64 = rest[1]
                .parse()
                .map_err(|_| format!("touch 的指针 id `{}` 不是整数", rest[1]))?;
            let x: f32 = rest[2]
                .parse()
                .map_err(|_| format!("touch 的 x 坐标 `{}` 不是数字", rest[2]))?;
            let y: f32 = rest[3]
                .parse()
                .map_err(|_| format!("touch 的 y 坐标 `{}` 不是数字", rest[3]))?;
            if !(x.is_finite() && y.is_finite()) {
                return Err("坐标必须是有限数".into());
            }
            Ok(Command::Touch { phase, id, x, y })
        }
        "resize" => {            if rest.len() != 2 {
                return Err("resize 需要宽 高 两个数字参数，如 `resize 800 600`".into());
            }
            let parse = |i: usize, name: &str| {
                rest[i]
                    .parse::<u32>()
                    .map_err(|_| format!("{name} `{}` 不是整数", rest[i]))
            };
            let (width, height) = (parse(0, "宽")?, parse(1, "高")?);
            if width == 0 || height == 0 {
                return Err("宽高不能为 0".into());
            }
            if width > 16_384 || height > 16_384 {
                return Err("宽高不能超过 16384".into());
            }
            Ok(Command::Resize { width, height })
        }
        "pointer_down" if rest.is_empty() => Ok(Command::PointerDown),
        "pointer_up" if rest.is_empty() => Ok(Command::PointerUp),
        "click_key" | "hover_key" => {
            let Some(key) = rest.first() else {
                return Err(format!("{cmd} 需要一个元素 key，如 `{cmd} switch:solid`"));
            };
            if rest.len() > 1 {
                return Err(format!("{cmd} 只接受一个 key，收到 {} 个参数", rest.len()));
            }
            let key = key.to_string();
            if cmd == "click_key" {
                Ok(Command::ClickKey { key })
            } else {
                Ok(Command::HoverKey { key })
            }
        }
        "click_path" | "hover_path" => {
            let Some(p) = rest.first() else {
                return Err(format!("{cmd} 需要一个路径参数，如 `{cmd} 0.1.1`"));
            };
            if rest.len() > 1 {
                return Err(format!("{cmd} 只接受一个路径，收到 {} 个参数", rest.len()));
            }
            let path = parse_path(p).map_err(|e| format!("{cmd}: {e}"))?;
            if cmd == "click_path" {
                Ok(Command::ClickPath { path })
            } else {
                Ok(Command::HoverPath { path })
            }
        }
        "press_key" => {
            if rest.len() != 1 || rest[0].chars().any(char::is_control) {
                return Err("press_key 需要一个逻辑键名，如 `press_key Enter` 或 `press_key a`".into());
            }
            Ok(Command::PressKey {
                key: rest[0].to_string(),
            })
        }
        "scroll" => {
            if rest.len() != 4 {
                return Err("scroll 需要 x y delta_x delta_y 四个数字参数".into());
            }
            let parse = |i: usize, name: &str| {
                rest[i]
                    .parse::<f32>()
                    .map_err(|_| format!("{name} `{}` 不是数字", rest[i]))
            };
            let (x, y, delta_x, delta_y) = (
                parse(0, "x 坐标")?,
                parse(1, "y 坐标")?,
                parse(2, "delta_x")?,
                parse(3, "delta_y")?,
            );
            if ![x, y, delta_x, delta_y].into_iter().all(f32::is_finite) {
                return Err("滚轮坐标与增量必须是有限数".into());
            }
            Ok(Command::Scroll {
                x,
                y,
                delta_x,
                delta_y,
            })
        }
        other => Err(format!(
            "未知命令 `{other}`；可用：ping tree layout props stats resources profile_start profile_report viewport hover click click_key hover_key click_path hover_path wait_frames find hit resize touch type_text pointer_down pointer_up press_key scroll shot"
        )),
    }
}

/// 解析树路径 `0.1.2` → `[0, 1, 2]`。
fn parse_path(s: &str) -> Result<Vec<usize>, String> {
    s.split('.')
        .map(|seg| {
            seg.parse::<usize>()
                .map_err(|_| format!("路径段 `{seg}` 不是下标（形如 `0.1.2`）"))
        })
        .collect()
}

/// 执行命令。宿主方法各管各的，这里只做搬运。
fn execute(cmd: &Command, host: &mut dyn BridgeHost) -> Response {
    match cmd {
        Command::Ping => Response::Text(vec![format!("pong {}", host.identity())]),
        Command::Tree { json } => {
            if *json {
                match host.root() {
                    Some(root) => Response::Text(
                        tree_json_of(root)
                            .lines()
                            .map(str::to_string)
                            .collect::<Vec<_>>(),
                    ),
                    None => Response::Text(vec![String::new()]),
                }
            } else {
                Response::Text(host.tree_dump().lines().map(str::to_string).collect())
            }
        }
        Command::Layout => {
            let (w, h) = host.viewport();
            let mut lines = vec![
                format!("viewport {w:.0}x{h:.0} physical-pixels"),
                format!("stats {}", host.stats()),
                "layout-tree path tag key absolute-x,absolute-y widthxheight [classes]".into(),
            ];
            lines.extend(host.tree_dump().lines().map(str::to_string));
            Response::Text(lines)
        }
        Command::Props { path, json } => {
            if *json {
                // JSON 形态从**节点**直接生成（不经过文本）—— 文本形态是给人读的。
                match host.root().and_then(|root| node_at_path(root, path)) {
                    Some(node) => Response::Text(vec![props_json_of(node)]),
                    None => Response::Err(format!(
                        "路径 {} 不存在（用 tree 查看当前形状；路径是子下标链，树里每行第一列）",
                        path.iter()
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>()
                            .join(".")
                    )),
                }
            } else {
                match host.props_dump(path) {
                    Some(s) => Response::Text(s.lines().map(str::to_string).collect()),
                    None => Response::Err(format!(
                        "路径 {} 不存在（用 tree 查看当前形状；路径是子下标链，树里每行第一列）",
                        path.iter()
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>()
                            .join(".")
                    )),
                }
            }
        }
        Command::Stats { json } => {
            let stats = host.stats();
            if *json {
                Response::Text(vec![stats.to_json()])
            } else {
                Response::Text(vec![stats.to_string()])
            }
        }
        Command::Resources => match host.resources() {
            Ok(resources) => Response::Text(resources.lines().map(str::to_string).collect()),
            Err(error) => Response::Err(error),
        },
        Command::Viewport => {
            let (w, h) = host.viewport();
            Response::Text(vec![format!("{w} {h}")])
        }
        Command::Hover { x, y } => {
            host.inject_hover(*x, *y);
            Response::Text(vec!["hovered".into()])
        }
        Command::Click { x, y } => {
            host.inject_click(*x, *y);
            Response::Text(vec!["clicked".into()])
        }
        Command::PointerDown => {
            host.inject_pointer_down();
            Response::Text(vec!["pointer_down".into()])
        }
        Command::PointerUp => {
            host.inject_pointer_up();
            Response::Text(vec!["pointer_up".into()])
        }
        Command::ClickKey { key } => match host.center_by_key(key) {
            Some((x, y)) => {
                host.inject_click(x, y);
                Response::Text(vec![format!("clicked key `{key}` at {x:.0},{y:.0}")])
            }
            None => Response::Err(format!(
                "key `{key}` 不存在（用 tree 看当前形状；key 列就是元素身份）"
            )),
        },
        Command::HoverKey { key } => match host.center_by_key(key) {
            Some((x, y)) => {
                host.inject_hover(x, y);
                Response::Text(vec![format!("hovered key `{key}` at {x:.0},{y:.0}")])
            }
            None => Response::Err(format!(
                "key `{key}` 不存在（用 tree 看当前形状；key 列就是元素身份）"
            )),
        },
        Command::ClickPath { path } => match host.center_by_path(path) {
            Some((x, y)) => {
                host.inject_click(x, y);
                Response::Text(vec![format!(
                    "clicked path {} at {x:.0},{y:.0}",
                    path.iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(".")
                )])
            }
            None => Response::Err(format!(
                "路径 {} 不存在（用 tree 看当前形状；路径是子下标链，树里每行第一列）",
                path.iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(".")
            )),
        },
        Command::HoverPath { path } => match host.center_by_path(path) {
            Some((x, y)) => {
                host.inject_hover(x, y);
                Response::Text(vec![format!(
                    "hovered path {} at {x:.0},{y:.0}",
                    path.iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(".")
                )])
            }
            None => Response::Err(format!(
                "路径 {} 不存在（用 tree 看当前形状；路径是子下标链，树里每行第一列）",
                path.iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(".")
            )),
        },
        Command::ProfileStart { duration_seconds } => match host.profile_start(*duration_seconds) {
            Ok(()) => Response::Text(vec![format!(
                "performance profiling started for {duration_seconds}s"
            )]),
            Err(error) => Response::Err(error),
        },
        Command::ProfileReport => match host.profile_report() {
            Ok(report) => Response::Text(report.lines().map(str::to_string).collect()),
            Err(error) => Response::Err(error),
        },
        Command::PressKey { key } => {
            host.inject_key(key);
            Response::Text(vec![format!("pressed {key}")])
        }
        Command::Scroll {
            x,
            y,
            delta_x,
            delta_y,
        } => {
            host.inject_scroll(*x, *y, *delta_x, *delta_y);
            Response::Text(vec!["scrolled".into()])
        }
        Command::Shot { repaint } => match host.screenshot(ShotOptions { repaint: *repaint }) {
            Some(data) => Response::Binary {
                note: "bmp 24bpp".into(),
                data,
            },
            None => Response::Err("此后端不支持像素读回".into()),
        },
        Command::WaitFrames { .. } => {
            // 正常路径下走不到这里：`wait_frames` 在桥的 poll 里被**延迟应答**
            // （要等应用真的画够 N 帧）。这一支只为穷尽匹配。
            Response::Err("wait_frames 由桥延迟应答（这条分支不该被走到）".into())
        }
        Command::Find { query, json } => match host.root() {
            Some(root) => {
                let hits = find_text_of(root, query);
                if *json {
                    let mut lines = Vec::with_capacity(hits.len());
                    for hit in &hits {
                        lines.push(format!(
                            "{{\"path\":{},\"text\":{},\"owner\":{},\"x\":{:.0},\"y\":{:.0},\"w\":{:.0},\"h\":{:.0}}}",
                            json_string(&hit.path),
                            json_string(&hit.text),
                            match &hit.owner {
                                Some(o) => json_string(o),
                                None => "null".into(),
                            },
                            hit.rect.x,
                            hit.rect.y,
                            hit.rect.width,
                            hit.rect.height
                        ));
                    }
                    Response::Text(lines)
                } else {
                    let mut lines = Vec::with_capacity(hits.len());
                    for hit in &hits {
                        lines.push(format!(
                            "{}  {}  at {:.0},{:.0} {:.0}x{:.0}  owner={}",
                            hit.path,
                            escape_inline(&hit.text),
                            hit.rect.x,
                            hit.rect.y,
                            hit.rect.width,
                            hit.rect.height,
                            hit.owner.as_deref().unwrap_or("-")
                        ));
                    }
                    Response::Text(lines)
                }
            }
            None => Response::Err("此后端不提供布局树，无法按文字查找".into()),
        },
        Command::TypeText { text } => {
            host.inject_text(text);
            Response::Text(vec![format!("typed {} characters", text.chars().count())])
        }
        Command::Hit { x, y } => match host.root() {
            Some(root) => match hit_at(root, *x, *y) {
                Some(hit) => {
                    let mut line = format!(
                        "{} {} key={} text={}",
                        hit.path,
                        hit.tag,
                        hit.key.as_deref().unwrap_or("-"),
                        match &hit.text {
                            Some(t) => escape_inline(t),
                            None => "-".into(),
                        }
                    );
                    // 可点的那个用**路径**给：key 常常不唯一（所有导航项的
                    // 标签都叫 `label`），按 key 点会点错那一个。
                    match &hit.clickable {
                        Some(p) => line.push_str(&format!("  clickable={p} (click_path {p})")),
                        None => line.push_str("  clickable=-"),
                    }
                    Response::Text(vec![line])
                }
                None => Response::Text(vec![format!(
                    "nothing at {x:.0},{y:.0} (the point is outside every element's box)"
                )]),
            },
            None => Response::Err("此后端不提供布局树，无法命中测试".into()),
        },
        Command::Resize { width, height } => match host.resize(*width, *height) {
            Ok(()) => Response::Text(vec![format!(
                "requested {width}x{height} physical pixels (the app re-lays out on the next frame; read `viewport` for the actual size)"
            )]),
            Err(e) => Response::Err(e),
        },
        Command::Touch { phase, id, x, y } => match host.inject_touch(*phase, *id, *x, *y) {
            Ok(()) => Response::Text(vec![format!(
                "touch {:?} id={id} at {x:.0},{y:.0}",
                phase
            )]),
            Err(e) => Response::Err(e),
        },
    }
}

/// 把响应写回 TCP。
///
/// 文本响应：每行一条 + 终止行 `OK`。二进制：`BIN <len> <note>\n` + 原始字节
/// + `\nOK`。客户端读到 `OK` / 消费完 `BIN` 才算一条响应结束。
fn response_bytes(resp: &Response) -> Vec<u8> {
    let mut head = String::new();
    match resp {
        Response::Text(lines) => {
            for l in lines {
                head.push_str(l);
                head.push('\n');
            }
            head.push_str("OK\n");
        }
        Response::Err(e) => {
            head.push_str("ERR ");
            head.push_str(e);
            head.push('\n');
        }
        Response::Binary { note, data } => {
            head.push_str(&format!("BIN {} {note}\n", data.len()));
        }
    }
    let mut out = head.into_bytes();
    if let Response::Binary { data, .. } = resp {
        out.extend_from_slice(data);
        out.extend_from_slice(b"\nOK\n");
    }
    out
}

/// Writes as much of a queued response as the non-blocking socket currently accepts.
/// Returns `false` while bytes remain for a later poll.
fn flush_response(conn: &mut Connection) -> Result<bool, std::io::Error> {
    while conn.write_offset < conn.write_buf.len() {
        match conn.stream.write(&conn.write_buf[conn.write_offset..]) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(n) => conn.write_offset += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) => return Err(e),
        }
    }
    conn.write_buf.clear();
    conn.write_offset = 0;
    Ok(true)
}

/// 属性值 → 人读文本（`props` 命令用；后端 host 实现也复用它 dump 属性表）。
///
/// 放在桥里而不是给 [`zephyra_core::property::PropertyValue`] 实现 `Display`：
/// 这是**调试口径**（如颜色出十六进制、方向出 Debug 形式），不是值的规范文本，
/// 让它渗进核心 API 会让「这个格式是不是契约」变得模糊——它不是。
pub fn value_to_text(v: &zephyra_core::property::PropertyValue) -> String {
    use zephyra_core::property::PropertyValue;
    match v {
        PropertyValue::Float(f) => format!("{f}"),
        PropertyValue::Int(i) => format!("{i}"),
        PropertyValue::Bool(b) => format!("{b}"),
        PropertyValue::Str(s) => s.to_string(),
        PropertyValue::StaticStr(s) => (*s).to_string(),
        PropertyValue::Color(c) => {
            // 十六进制 `#RRGGBBAA`——AI 与人都能直接对到设计稿。通道走
            // `srgb_bytes`（诊断口径与 BMP 截图一致），alpha 直乘。
            let [r, g, b] = crate::pixels::srgb_bytes(*c);
            let a = (c.a * 255.0).round().clamp(0.0, 255.0) as u8;
            format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
        }
        PropertyValue::Resource(k) => format!("${k}"),
        // 结构性枚举走 Debug——它们的名字本身就是词汇表（`Direction::Column`）。
        PropertyValue::Direction(v) => format!("{v:?}"),
        PropertyValue::CrossAlign(v) => format!("{v:?}"),
        PropertyValue::MainAlign(v) => format!("{v:?}"),
        PropertyValue::AlignContent(v) => format!("{v:?}"),
        PropertyValue::AlignSelf(v) => format!("{v:?}"),
        PropertyValue::FlexWrap(v) => format!("{v:?}"),
        PropertyValue::FlexDirection(v) => format!("{v:?}"),
        PropertyValue::Position(v) => format!("{v:?}"),
        PropertyValue::Overflow(v) => format!("{v:?}"),
        PropertyValue::Anchor(v) => format!("{v:?}"),
        PropertyValue::Layer(v) => format!("{v:?}"),
        PropertyValue::Filter(v) => format!("{v:?}"),
        PropertyValue::BlendMode(v) => format!("{v:?}"),
        PropertyValue::Material(v) => format!("{v:?}"),
        PropertyValue::MaterialParams(v) => format!("{v:?}"),
        PropertyValue::Decorations(v) => format!("{v:?}"),
        PropertyValue::UvRect(v) => format!("{v:?}"),
        PropertyValue::Rect(v) => format!("{v:?}"),
        // 类型擦除载荷没有稳定文本：只标存在性。真正要读它的宿主应该
        // 注册成字符串属性，而不是指望调试器看穿 `dyn Any`。
        PropertyValue::CornerShape(v) => {
            let name = if v.is_round() {
                "round"
            } else if v.is_squircle() {
                "squircle"
            } else if (v.k - 0.0).abs() < 1e-6 {
                "bevel"
            } else if (v.k - -1.0).abs() < 1e-6 {
                "scoop"
            } else if v.k.is_infinite() && v.k.is_sign_positive() {
                "square"
            } else if v.k.is_infinite() && v.k.is_sign_negative() {
                "notch"
            } else {
                return format!("superellipse({})", v.k);
            };
            name.into()
        }
        PropertyValue::Custom { .. } => "<custom>".into(),
    }
}

#[cfg(test)]
#[path = "../tests/unit/bridge.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/unit/bridge_tree_dump_tests.rs"]
mod tree_dump_tests;

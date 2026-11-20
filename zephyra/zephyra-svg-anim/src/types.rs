//! SVG 动画引擎的数据类型。
//!
//! 设计取舍见 crate 根文档。这里只放数据结构，不掺解析/求值逻辑。

use zephyra_bezier::Bezier;
pub use zephyra_vector::icon::{FillRule, PaintMode, Segment};

/// SMIL 动画元素解析错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgAnimError {
    /// 文本里找不到 `<svg` 根。
    NoSvgRoot,
    /// 根元素缺 `viewBox`。
    NoViewBox,
    /// `viewBox` 不是四个数。
    BadViewBox,
    /// 没有可动画的 `<path>`。
    NoPath,
    /// `<animate>` 缺 `attributeName`。
    MissingAttributeName,
    /// `dur` / `begin` 等时间值非法。
    BadTiming,
    /// `values` / `from` / `to` 内容非法。
    BadValues,
}

impl core::fmt::Display for SvgAnimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            Self::NoSvgRoot => "找不到 <svg> 根元素",
            Self::NoViewBox => "缺少 viewBox",
            Self::BadViewBox => "viewBox 必须是 4 个数",
            Self::NoPath => "没有可动画的 <path>",
            Self::MissingAttributeName => "<animate> 缺少 attributeName",
            Self::BadTiming => "时间属性非法",
            Self::BadValues => "values/from/to 非法",
        };
        write!(f, "{s}")
    }
}

/// 重复次数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Repeat {
    /// 固定次数（0 = 不播，1 = 播一次）。
    Count(u32),
    /// `repeatCount="indefinite"` / `repeatDur="indefinite"`。
    Indefinite,
    /// `repeatDur="2500ms"`：重复到总时长达到该值。
    Duration(f32),
    /// `repeatCount` 与 `repeatDur` 同时给：取**先到者**（SMIL 规定）。
    Bounded { count: u32, total_ms: f32 },
}

/// `fill` 属性：动画结束后停在终值还是回到基值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillMode {
    /// 回到动画前的基值（默认）。
    Remove,
    /// 冻结在最后一次求值的结果。
    Freeze,
}

/// `calcMode` 属性：关键帧之间的插值方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcMode {
    /// 线性插值（默认）。
    Linear,
    /// 阶跃：在 keyTimes 边界才跳到下一值。
    Discrete,
    /// 按路径长度匀速（用于 `animateMotion`）。
    Paced,
    /// `keySplines` 指定的三次贝塞尔缓动。
    Spline,
}

/// `begin` / `end` 的时间规格。
#[derive(Debug, Clone, PartialEq)]
pub enum Begin {
    /// 相对文档开始的时钟偏移（毫秒），可为负（动画从中间开始）。
    Offset(f32),
    /// `begin="indefinite"`：只能由宿主 `beginElement()` 触发。
    Indefinite,
    /// 事件基：`begin="click"` / `begin="id.click"`。
    Event {
        target: Option<String>,
        event: String,
    },
    /// syncbase：`begin="other.end+500ms"`。
    Syncbase {
        id: String,
        event: String,
        offset_ms: f32,
    },
    /// `accessKey="k"`：按键触发。
    AccessKey(char),
}

/// `restart` 属性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    Always,
    WhenNotActive,
    Never,
}

/// `stroke` 颜色动画。
#[derive(Debug, Clone, PartialEq)]
pub struct StrokeColorAnim {
    pub timing: Timing,
    pub values: Values<[f32; 4]>,
}

/// `fill-opacity` / `stroke` / `stroke-width` / `stroke-opacity` 动画。
#[derive(Debug, Clone, PartialEq)]
pub struct StyleAnim {
    pub timing: Timing,
    pub attribute_name: String,
    pub values: Values<f32>,
}

/// 一个动画的时间参数（SMIL 共通属性）。
#[derive(Debug, Clone, PartialEq)]
pub struct Timing {
    /// `begin`：相对文档开始的延迟（毫秒）。
    /// `begin` 时间规格（偏移 / indefinite / 事件 / syncbase）。
    pub begin: Begin,
    /// `dur`：单次时长（毫秒）。
    /// `dur`：毫秒。`None` = `dur="indefinite"`。
    pub dur_ms: Option<f32>,
    /// 重复策略。
    pub repeat: Repeat,
    /// 结束后是否冻结。
    pub fill: FillMode,
    /// 插值方式。
    pub calc_mode: CalcMode,
    /// `keyTimes`：每个值的时间比例（0..1，升序，与 values 等长）。
    pub key_times: Vec<f32>,
    /// `keySplines`：每段的贝塞尔控制点（4 个数一段）。
    pub key_splines: Vec<Bezier>,
    /// `end`：绝对结束时刻（毫秒）。`None` = 不提前结束。
    pub end_ms: Option<f32>,
    /// `additive="sum"`：动画值叠加到基值之上，而不是替换。
    pub additive_sum: bool,
    /// `accumulate="sum"`：每次重复把上一次的终值作为下一次的起点。
    pub accumulate_sum: bool,
    /// `restart` 策略。
    pub restart: Restart,
    /// `min`：活跃期最短时长（毫秒）。
    pub min_ms: f32,
    /// `max`：活跃期最长时长（毫秒）。
    pub max_ms: Option<f32>,
    /// `accessKey`：按键触发。
    pub access_key: Option<char>,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            begin: Begin::Offset(0.0),
            dur_ms: None,
            repeat: Repeat::Count(1),
            fill: FillMode::Remove,
            calc_mode: CalcMode::Linear,
            key_times: Vec::new(),
            key_splines: Vec::new(),
            end_ms: None,
            additive_sum: false,
            accumulate_sum: false,
            restart: Restart::Always,
            min_ms: 0.0,
            max_ms: None,
            access_key: None,
        }
    }
}

/// 动画目标值序列。
#[derive(Debug, Clone, PartialEq)]
pub enum Values<T> {
    /// `from="a" to="b"`：两端。
    FromTo(T, T),
    /// `by="d"`：从基值加增量。
    By(T),
    /// `values="a;b;c"`：多关键帧。
    List(Vec<T>),
}

/// `opacity` 动画（0..1）。
#[derive(Debug, Clone, PartialEq)]
pub struct OpacityAnim {
    pub timing: Timing,
    pub values: Values<f32>,
}

/// `fill` 动画（RGBA，0..1）。
#[derive(Debug, Clone, PartialEq)]
pub struct FillAnim {
    pub timing: Timing,
    pub values: Values<[f32; 4]>,
}

/// `d` 动画（路径形变）。
#[derive(Debug, Clone, PartialEq)]
pub struct PathAnim {
    pub timing: Timing,
    pub values: Values<Vec<Segment>>,
}

/// `<animateTransform>` 的变换种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformKind {
    Translate,
    Scale,
    Rotate,
    SkewX,
    SkewY,
}

/// `<animateTransform>`：三个分量（平移 x/y、缩放 x/y、旋转 angle/cx/cy）。
#[derive(Debug, Clone, PartialEq)]
pub struct TransformAnim {
    pub timing: Timing,
    pub kind: TransformKind,
    pub values: Values<[f32; 3]>,
}

/// `<animateMotion>`：沿路径运动。
#[derive(Debug, Clone, PartialEq)]
pub struct MotionAnim {
    pub timing: Timing,
    pub path: Vec<Segment>,
    /// 运动旋转模式：`auto` / `auto-reverse` / 无（0）。
    pub rotate_mode: MotionRotate,
    /// `keyPoints`：沿路径的关键点比例（0..1），与 keyTimes 对应。
    pub key_points: Vec<f32>,
    /// `<mpath xlink:href="#id">` 引用的路径 id（解析时只记录，不跨文档解析）。
    pub mpath_href: Option<String>,
    /// `origin`：运动路径的起点偏移 (x, y)，默认 (0,0)。
    pub origin: (f32, f32),
}

/// `<animateMotion rotate="...">` 的模式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotionRotate {
    /// 不旋转（默认）。
    None,
    /// 朝向运动方向。
    Auto,
    /// 朝向运动方向 + 180°（反向）。
    AutoReverse,
    /// 固定角度（度）。
    FixedAngle(f32),
}

/// 几何属性动画：x / y / width / height / r / cx / cy。
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryAnim {
    pub timing: Timing,
    /// 属性名（x/y/width/height/r/cx/cy）。
    pub attribute_name: String,
    pub values: Values<f32>,
}

/// `<set>`：在一段时间内把属性设成固定值（离散，无插值）。
#[derive(Debug, Clone, PartialEq)]
pub struct SetAnim {
    pub timing: Timing,
    /// 属性名（目前支持 opacity / fill / d）。
    pub attribute_name: String,
    /// 目标值（按属性名解释）。
    pub to: String,
}

/// 一个元素上挂的全部动画。
#[derive(Debug, Clone, PartialEq)]
pub enum Animation {
    /// `<set>` 离散赋值。
    Set(SetAnim),
    Opacity(OpacityAnim),
    Fill(FillAnim),
    /// 几何属性动画。
    Geometry(GeometryAnim),
    /// 描边 / 不透明度样式动画。
    Style(StyleAnim),
    /// stroke 颜色动画。
    StrokeColor(StrokeColorAnim),
    Path(PathAnim),
    Transform(TransformAnim),
    Motion(MotionAnim),
}

/// 解析后的可动画 SVG（图标级子集 + SMIL 动画）。
#[derive(Debug, Clone, PartialEq)]
pub struct AnimatedSvg {
    /// (min_x, min_y, width, height)
    pub view_box: (f32, f32, f32, f32),
    pub paths: Vec<AnimatedPath>,
}

/// 一条带动画的 `<path>`。
#[derive(Debug, Clone, PartialEq)]
pub struct AnimatedPath {
    /// 基础路径段（无动画时的形态）。
    pub base: Vec<Segment>,
    /// 基础填充：模式 + 规则。
    pub paint_mode: PaintMode,
    pub fill_rule: FillRule,
    pub animations: Vec<Animation>,
}

/// 一帧的求值结果：一条路径的当前形态与外观。
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedPath {
    pub segments: Vec<Segment>,
    pub paint_mode: PaintMode,
    pub fill_rule: FillRule,
    /// 最终不透明度（基值 1.0 × opacity 动画）。
    pub opacity: f32,
    /// 累积的 2D 仿射变换：[a, b, c, d, e, f]
    /// （x' = a·x + c·y + e；y' = b·x + d·y + f）。
    pub transform: [f32; 6],
    /// 运动路径带来的偏移（已并进 transform 的 e/f，单独留一份便于调试/HUD）。
    pub motion_offset: (f32, f32),
    /// 几何属性动画的结果：[x, y, width, height, r]（未设置的属性为 0）。
    pub geometry: [f32; 6],
    /// fill-opacity（默认 1.0）。
    pub fill_opacity: f32,
    /// stroke-opacity（默认 1.0）。
    pub stroke_opacity: f32,
    /// stroke-width（默认 0）。
    pub stroke_width: f32,
    /// 描边色（None = currentColor）。
    pub stroke: Option<[f32; 4]>,
}

/// 一帧的完整结果。
#[derive(Debug, Clone, PartialEq)]
pub struct SvgFrame {
    pub paths: Vec<RenderedPath>,
    /// 当前时间（毫秒）。
    pub time_ms: f32,
}

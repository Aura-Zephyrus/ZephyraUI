#![forbid(unsafe_code)]
#![deny(rustdoc::broken_intra_doc_links)]
//! SVG SMIL 动画引擎。
//!
//! # 做什么
//!
//! 解析 SVG 原生的动画元素（**只做 SMIL，不做 CSS**）：
//!
//! - `<animate attributeName="opacity|fill|d">`
//! - `<animateTransform type="translate|scale|rotate">`
//! - `<animateMotion path="...">`
//!
//! 然后按时间求值成 [`SvgFrame`]：每条路径在某一时刻的形态、颜色、不透明度与
//! 仿射变换。`SvgTimeline::tick` 推进时钟，`to_dynamic_path` 把一帧三角化成
//! `DynamicPath` 直接送进 `PATH_DATA`。
//!
//! # 为什么是独立 crate
//!
//! 动画的**时间线与插值**与绘制后端无关，且要能在 CI 里逐点断言 —— 所以本 crate
//! 只依赖 `zephyra-vector`（解析 / 几何 / 三角化）与零依赖的 `zephyra-bezier`，
//! **不依赖 `zephyra-core`**。产物是纯数据，应用层自己决定怎么送进 DisplayList。
//!
//! # 与现有动画体系的关系
//!
//! 控件层的补间（`zephyra_ui::motion`）还是"进度在代码、外观在 zstyle"那一套。
//! 本引擎是**另一条路**：给需要真正 SVG 时间线动画的场景（图标内部形变、沿路径
//! 运动、多关键帧颜色过渡）提供完整能力，两者不互相替代。

pub mod eval;
pub mod smil;
pub mod types;

pub use eval::{to_dynamic_path, SvgTimeline};

/// 一条 SVG 动画的总时长（毫秒）：所有动画 `dur + begin` 的最大值。
/// 供接入层把外部 progress 0..1 映射到 `SvgTimeline::seek()` 的绝对时间。
pub fn duration_ms(svg: &AnimatedSvg) -> f32 {
    let mut max = 0.0f32;
    for path in &svg.paths {
        for anim in &path.animations {
            let t = anim.timing();
            let begin = match t.begin {
                Begin::Offset(v) => v.max(0.0),
                _ => 0.0,
            };
            let dur = t.dur_ms.unwrap_or(0.0);
            let total = match t.repeat {
                Repeat::Count(n) => dur * n as f32,
                Repeat::Duration(d) => d,
                Repeat::Indefinite | Repeat::Bounded { .. } => dur,
            };
            max = max.max(begin + total);
        }
    }
    max.max(1.0)
}
pub use smil::parse_animated_svg;
pub use types::{
    AnimatedPath, AnimatedSvg, Animation, Begin, CalcMode, Direction, FillAnim, FillMode,
    MotionAnim, OpacityAnim, PathAnim, RenderedPath, Repeat, SvgAnimError, SvgFrame, Timing,
    TransformAnim, TransformKind, Values,
};

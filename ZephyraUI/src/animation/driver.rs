//! 动画驱动：外部 progress → `SvgTimeline::seek()` → 节点的 paint 属性。
//!
//! # 时钟归应用
//!
//! 这里**不推进时钟**：`update()` 只读节点上的 `animation-progress`（应用绑定写入），
//! 把它映射到时间线的绝对时刻。暂停就是不再改 progress —— core 的逐字节 diff
//! 自然不重绘，`FrameCache` 无需改动（见 docs/developer/syntax.md §5）。
//!
//! # 只写 paint 属性
//!
//! 只写 `PATH_DATA` 与 `OPACITY`，两者 `affects_layout = false`，所以动画永远
//! 改不了布局盒。颜色不在引擎里写：预制 SVG 一律不动画 fill/stroke 色，颜色
//! 继续由 `.zstyle` 的 `@token` 决定。

use std::collections::HashMap;

use zephyra_core::node::Node;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_svg_anim::{
    duration_ms, parse_animated_svg, to_dynamic_path, AnimatedSvg, SvgFrame, SvgTimeline,
};

/// 预制动画 id → SVG 源码。
pub const PREFADE_SOURCES: &[(&str, &str)] = &[
    ("spinner", include_str!("../../ui/animations/spinner.svg")),
    ("pulse", include_str!("../../ui/animations/pulse.svg")),
    ("ripple", include_str!("../../ui/animations/ripple.svg")),
    (
        "check-draw",
        include_str!("../../ui/animations/check-draw.svg"),
    ),
    (
        "hamburger-x",
        include_str!("../../ui/animations/hamburger-x.svg"),
    ),
    (
        "progress-arc",
        include_str!("../../ui/animations/progress-arc.svg"),
    ),
];

/// 动画驱动器：持有解析好的动画资源，按外部 progress 取帧并写到节点上。
pub struct AnimDriver {
    svgs: HashMap<String, AnimatedSvg>,
}

impl Default for AnimDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl AnimDriver {
    /// 注册全部预制动画。解析失败的预制直接 panic —— 资源随二进制分发，
    /// 坏资源不该等到运行时才发现。
    pub fn new() -> Self {
        let mut svgs = HashMap::new();
        for (id, src) in PREFADE_SOURCES {
            let svg = parse_animated_svg(src)
                .unwrap_or_else(|e| panic!("预制动画 `{id}` 解析失败: {e:?}"));
            svgs.insert((*id).to_string(), svg);
        }
        Self { svgs }
    }

    /// 注册应用自己的动画（id / SVG 源码）。旧 id 被覆盖。
    pub fn register(&mut self, id: impl Into<String>, svg_src: &str) -> Result<(), String> {
        let svg = parse_animated_svg(svg_src).map_err(|e| format!("{e:?}"))?;
        self.svgs.insert(id.into(), svg);
        Ok(())
    }

    /// 按 id 取一帧。`progress`  clamp 到 0..1 后映射到整条时间线。
    pub fn frame(&self, id: &str, progress: f32) -> Option<SvgFrame> {
        let svg = self.svgs.get(id)?;
        let p = progress.clamp(0.0, 1.0);
        let mut tl = SvgTimeline::new(svg.clone());
        Some(tl.seek(p * duration_ms(svg)))
    }

    /// 把一帧写进节点。只写 paint 属性；多路径预制只取第一条（接入约定：
    /// 一个动画节点对应一条 `<path>`）。
    pub fn apply(node: &mut Node, frame: &SvgFrame) {
        let Some(first) = frame.paths.first() else {
            return;
        };
        // 必须写 styled 层，不能写 local 层：diff 每帧用 retain_local_props
        // 拆掉「声明树没声明的本地属性」。模板只声明 animation/progress，
        // 所以写 local 的 PATH_DATA 下一帧必被拆 —— 暂停时画面直接空白。
        // styled 层既不被拆，set_styled_if_changed 还保证同值不重复标脏。
        node.set_styled_prop(&props::OPACITY, PropertyValue::float(first.opacity));
        if let Some(path) = to_dynamic_path(first) {
            node.set_styled_prop(&props::PATH_DATA, PropertyValue::custom(path));
        }
    }

    /// 遍历整棵树，对带 `animation` 属性的节点套用当前 progress。
    /// `animation-play-state = false` 时跳过（保持上一帧写入的值）。
    pub fn update(&self, root: &mut Node) {
        self.update_node(root);
    }

    fn update_node(&self, node: &mut Node) {
        let id = match node.props().lookup(&props::ANIMATION).map(|(v, _)| v) {
            Some(PropertyValue::StaticStr(s)) => Some((*s).to_string()),
            Some(PropertyValue::Str(s)) => Some(s.to_string()),
            _ => None,
        };
        let playing = match node.props().lookup(&props::ANIMATION_PLAY_STATE) {
            Some((PropertyValue::Bool(b), _)) => *b,
            _ => true,
        };
        if playing {
            if let Some(id) = id {
                let progress = match node.props().lookup(&props::ANIMATION_PROGRESS) {
                    Some((PropertyValue::Float(p), _)) => *p,
                    _ => 0.0,
                };
                if let Some(frame) = self.frame(&id, progress) {
                    Self::apply(node, &frame);
                }
            }
        }
        for child in node.children_mut() {
            self.update_node(child);
        }
    }
}

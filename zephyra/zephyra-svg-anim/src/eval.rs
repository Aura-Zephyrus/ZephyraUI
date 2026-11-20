//! SMIL 求值：把解析出的动画按时间采样成 [`SvgFrame`]。
//!
//! # 时间模型
//!
//! 全局时间 `time_ms` 单调前进。每个动画有自己的 `begin` / `dur` / `repeat`：
//!
//! ```text
//! local = time_ms - begin
//! 若 local < 0            ⇒ 尚未开始，用基值
//! 若 repeat = Count(n) 且已播完 n 次 ⇒ 用基值（fill=remove）或末值（fill=freeze）
//! 否则 cycle = local % dur，iteration = local / dur
//! ```
//!
//! 多动画叠加顺序固定：`opacity` → `fill` → `path` → `transform` → `motion`，
//! 后写的覆盖前写的同类属性。这个顺序写在下面一个函数里，不散落在各分支。

use zephyra_vector::geom::{flatten, triangulate, Contour, FillRule as GeomFillRule, TOLERANCE};
use zephyra_vector::icon::Segment;
use zephyra_vector::paint::DynamicPath;

use crate::types::*;

/// 动画时间线：持有解析结果和全局时钟。
#[derive(Debug, Clone)]
pub struct SvgTimeline {
    svg: AnimatedSvg,
    time_ms: f32,
    /// 由 beginElement() 显式触发的动画在当前 iteration 的开始时刻
    /// （key = 路径下标+动画下标）。
    triggered: std::collections::HashMap<(usize, usize), f32>,
    /// endElement() 显式结束的时刻。
    ended: std::collections::HashMap<(usize, usize), f32>,
}

impl SvgTimeline {
    pub fn new(svg: AnimatedSvg) -> Self {
        Self {
            svg,
            time_ms: 0.0,
            triggered: std::collections::HashMap::new(),
            ended: std::collections::HashMap::new(),
        }
    }

    /// 推进 `dt_ms`（毫秒）并返回这一时刻的帧。
    pub fn tick(&mut self, dt_ms: f32) -> SvgFrame {
        if dt_ms.is_finite() && dt_ms > 0.0 {
            self.time_ms += dt_ms;
        }
        self.frame()
    }

    /// 跳到绝对时间（毫秒），返回那一时刻的帧。
    pub fn seek(&mut self, time_ms: f32) -> SvgFrame {
        self.time_ms = if time_ms.is_finite() && time_ms >= 0.0 {
            time_ms
        } else {
            0.0
        };
        self.frame()
    }

    pub fn time_ms(&self) -> f32 {
        self.time_ms
    }

    pub fn svg(&self) -> &AnimatedSvg {
        &self.svg
    }

    /// 显式触发 `begin="indefinite"` / 事件基的动画（SMIL `beginElement()`）。
    /// `path_index` / `anim_index` 定位到具体动画。
    pub fn begin_element(&mut self, path_index: usize, anim_index: usize) -> bool {
        let exists = self
            .svg
            .paths
            .get(path_index)
            .and_then(|p| p.animations.get(anim_index))
            .is_some();
        if exists {
            self.triggered
                .insert((path_index, anim_index), self.time_ms);
            self.ended.remove(&(path_index, anim_index));
        }
        exists
    }

    /// 显式结束动画（SMIL `endElement()`）。
    pub fn end_element(&mut self, path_index: usize, anim_index: usize) -> bool {
        let exists = self
            .svg
            .paths
            .get(path_index)
            .and_then(|p| p.animations.get(anim_index))
            .is_some();
        if exists {
            self.ended.insert((path_index, anim_index), self.time_ms);
        }
        exists
    }

    /// 按 id / 事件名触发所有匹配的 begin 动画（syncbase / 事件基的宿主侧入口）。
    pub fn fire_event(&mut self, target: Option<&str>, event: &str) -> usize {
        let mut count = 0;
        for (pi, path) in self.svg.paths.iter().enumerate() {
            for (ai, anim) in path.animations.iter().enumerate() {
                let begin = match anim {
                    Animation::Set(a) => &a.timing.begin,
                    Animation::Opacity(a) => &a.timing.begin,
                    Animation::Fill(a) => &a.timing.begin,
                    Animation::Geometry(a) => &a.timing.begin,
                    Animation::Style(a) => &a.timing.begin,
                    Animation::Path(a) => &a.timing.begin,
                    Animation::Transform(a) => &a.timing.begin,
                    Animation::Motion(a) => &a.timing.begin,
                    Animation::StrokeColor(a) => &a.timing.begin,
                };
                let matches = match begin {
                    Begin::Event {
                        target: t,
                        event: e,
                    } => {
                        e == event
                            && match (t, target) {
                                (None, _) => true,
                                (Some(x), Some(y)) => x == y,
                                _ => false,
                            }
                    }
                    Begin::Syncbase { event: e, .. } => e == event,
                    _ => false,
                };
                if matches {
                    self.triggered.insert((pi, ai), self.time_ms);
                    self.ended.remove(&(pi, ai));
                    count += 1;
                }
            }
        }
        count
    }

    /// 求当前帧。纯函数式的：不改时间，只按 `time_ms` 采样。
    pub fn frame(&self) -> SvgFrame {
        let paths = self
            .svg
            .paths
            .iter()
            .enumerate()
            .map(|(pi, p)| eval_path(p, self.time_ms, pi, &self.triggered, &self.ended))
            .collect();
        SvgFrame {
            paths,
            time_ms: self.time_ms,
        }
    }
}

/// `#rgb` / `#rrggbb` / `#rrggbbaa` → RGBA(0..1)，失败返回 None。
fn parse_hex_rgba(s: &str) -> Option<[f32; 4]> {
    let hex = s.strip_prefix('#')?;
    let ch = |i: usize, n: usize| u8::from_str_radix(&hex[i..i + n], 16).ok();
    match hex.len() {
        3 => Some([
            ch(0, 1)? as f32 / 15.0,
            ch(1, 1)? as f32 / 15.0,
            ch(2, 1)? as f32 / 15.0,
            1.0,
        ]),
        6 => Some([
            ch(0, 2)? as f32 / 255.0,
            ch(2, 2)? as f32 / 255.0,
            ch(4, 2)? as f32 / 255.0,
            1.0,
        ]),
        8 => Some([
            ch(0, 2)? as f32 / 255.0,
            ch(2, 2)? as f32 / 255.0,
            ch(4, 2)? as f32 / 255.0,
            ch(6, 2)? as f32 / 255.0,
        ]),
        _ => None,
    }
}

/// 求单条路径在 `time_ms` 的形态。
fn eval_path(
    path: &AnimatedPath,
    time_ms: f32,
    path_index: usize,
    triggered: &std::collections::HashMap<(usize, usize), f32>,
    ended: &std::collections::HashMap<(usize, usize), f32>,
) -> RenderedPath {
    let mut segments = path.base.clone();
    let mut paint_mode = path.paint_mode;
    let mut opacity = 1.0f32;
    let mut transform = IDENTITY;
    let mut motion_offset = (0.0f32, 0.0f32);
    let mut geometry: [f32; 6] = [0.0; 6]; // x, y, width, height, r, (unused)
    let mut fill_opacity = 1.0f32;
    let mut stroke_opacity = 1.0f32;
    let mut stroke_width = 0.0f32;
    let mut stroke: Option<[f32; 4]> = None;

    for (anim_index, anim) in path.animations.iter().enumerate() {
        // 事件/indefinite 动画：由 beginElement/fire_event 触发，用触发时刻作虚拟 begin。
        let virtual_begin = triggered.get(&(path_index, anim_index)).copied();
        let virtual_end = ended.get(&(path_index, anim_index)).copied();
        match anim {
            Animation::Set(set) => {
                if let Some((_, active, _)) = phase(
                    &effective_timing(&set.timing, virtual_begin, virtual_end),
                    time_ms,
                ) {
                    // set 是离散的：活跃期内取 to，非活跃期保持基值。
                    if active {
                        match set.attribute_name.as_str() {
                            "opacity" => {
                                if let Ok(v) = set.to.parse::<f32>() {
                                    opacity = v.clamp(0.0, 1.0);
                                }
                            }
                            "fill" => {
                                if let Some(c) = parse_hex_rgba(&set.to) {
                                    paint_mode = PaintMode::Solid(c);
                                }
                            }
                            "d" => {
                                if let Ok(v) = zephyra_vector::icon::parse_path_data(&set.to) {
                                    segments = v;
                                }
                            }
                            "fill-opacity" => {
                                if let Ok(v) = set.to.parse::<f32>() {
                                    fill_opacity = v.clamp(0.0, 1.0);
                                }
                            }
                            "stroke-opacity" => {
                                if let Ok(v) = set.to.parse::<f32>() {
                                    stroke_opacity = v.clamp(0.0, 1.0);
                                }
                            }
                            "stroke-width" => {
                                if let Ok(v) = set.to.parse::<f32>() {
                                    stroke_width = v.max(0.0);
                                }
                            }
                            "stroke" => {
                                stroke = parse_hex_rgba(&set.to);
                            }
                            "x" | "y" | "width" | "height" | "r" => {
                                if let Ok(v) = set.to.parse::<f32>() {
                                    let idx = match set.attribute_name.as_str() {
                                        "x" => 0,
                                        "y" => 1,
                                        "width" => 2,
                                        "height" => 3,
                                        "r" => 4,
                                        _ => 5,
                                    };
                                    if idx < 5 {
                                        geometry[idx] = v;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Animation::Opacity(a) => {
                if let Some((v, _)) = sample_f32(
                    &effective_timing(&a.timing, virtual_begin, virtual_end),
                    &a.values,
                    time_ms,
                    1.0,
                ) {
                    opacity = if a.timing.additive_sum {
                        (opacity + v).clamp(0.0, 1.0)
                    } else {
                        v.clamp(0.0, 1.0)
                    };
                }
            }
            Animation::StrokeColor(a) => {
                if let Some((v, _)) = sample_color(
                    &effective_timing(&a.timing, virtual_begin, virtual_end),
                    &a.values,
                    time_ms,
                    [0.0, 0.0, 0.0, 0.0],
                ) {
                    stroke = Some(v);
                }
            }
            Animation::Style(a) => {
                if let Some((v, _)) = sample_f32(
                    &effective_timing(&a.timing, virtual_begin, virtual_end),
                    &a.values,
                    time_ms,
                    1.0,
                ) {
                    match a.attribute_name.as_str() {
                        "fill-opacity" => fill_opacity = v.clamp(0.0, 1.0),
                        "stroke-opacity" => stroke_opacity = v.clamp(0.0, 1.0),
                        "stroke-width" => stroke_width = v.max(0.0),
                        _ => {}
                    }
                }
            }
            Animation::Geometry(a) => {
                if let Some((v, _)) = sample_f32(
                    &effective_timing(&a.timing, virtual_begin, virtual_end),
                    &a.values,
                    time_ms,
                    0.0,
                ) {
                    let idx = match a.attribute_name.as_str() {
                        "x" => 0,
                        "y" => 1,
                        "width" => 2,
                        "height" => 3,
                        "r" => 4,
                        "cx" => 0,
                        "cy" => 1,
                        _ => 5,
                    };
                    if idx < 5 {
                        geometry[idx] = v;
                    }
                }
            }
            Animation::Fill(a) => {
                if let Some((v, _)) = sample_color(
                    &effective_timing(&a.timing, virtual_begin, virtual_end),
                    &a.values,
                    time_ms,
                    [1.0, 1.0, 1.0, 1.0],
                ) {
                    let base = match paint_mode {
                        PaintMode::Solid(c) => c,
                        _ => [0.0, 0.0, 0.0, 0.0],
                    };
                    let c = if a.timing.additive_sum {
                        [
                            (base[0] + v[0]).min(1.0),
                            (base[1] + v[1]).min(1.0),
                            (base[2] + v[2]).min(1.0),
                            (base[3] + v[3]).min(1.0),
                        ]
                    } else {
                        v
                    };
                    paint_mode = PaintMode::Solid(c);
                }
            }
            Animation::Path(a) => {
                if let Some((v, _)) = sample_path(
                    &effective_timing(&a.timing, virtual_begin, virtual_end),
                    &a.values,
                    time_ms,
                    &path.base,
                ) {
                    segments = v;
                }
            }
            Animation::Transform(a) => {
                if let Some((v, _)) = sample_f32x3(
                    &effective_timing(&a.timing, virtual_begin, virtual_end),
                    &a.values,
                    time_ms,
                    [0.0, 0.0, 0.0],
                ) {
                    // 先算出**本动画自身的增量**（基于单位矩阵），再按 additive 决定
                    // 是叠加到已有 transform 还是替换它。
                    let delta = match a.kind {
                        TransformKind::Translate => [1.0, 0.0, 0.0, 1.0, v[0], v[1]],
                        TransformKind::Scale => [v[0], 0.0, 0.0, v[1], 0.0, 0.0],
                        TransformKind::SkewX => [1.0, 0.0, v[0].tan(), 1.0, 0.0, 0.0],
                        TransformKind::SkewY => [1.0, v[0].tan(), 0.0, 1.0, 0.0, 0.0],
                        TransformKind::Rotate => {
                            let (r, cx, cy) = (v[0].to_radians(), v[1], v[2]);
                            let (s, c) = (r.sin(), r.cos());
                            let rot = [c, s, -s, c, 0.0, 0.0];
                            mul(
                                mul([1.0, 0.0, 0.0, 1.0, cx, cy], rot),
                                [1.0, 0.0, 0.0, 1.0, -cx, -cy],
                            )
                        }
                    };
                    transform = if a.timing.additive_sum {
                        mul(transform, delta)
                    } else {
                        delta
                    };
                }
            }
            Animation::Motion(a) => {
                if let Some((p, angle, _)) = sample_motion(a, time_ms) {
                    motion_offset = p;
                    transform = mul(transform, [1.0, 0.0, 0.0, 1.0, p.0, p.1]);
                    if a.rotate_mode != MotionRotate::None {
                        let angle = if a.rotate_mode == MotionRotate::AutoReverse {
                            angle + std::f32::consts::PI
                        } else {
                            angle
                        };
                        let (s, c) = (angle.sin(), angle.cos());
                        transform = mul(transform, [c, s, -s, c, 0.0, 0.0]);
                    }
                }
            }
        }
    }

    RenderedPath {
        segments,
        paint_mode,
        fill_rule: path.fill_rule,
        opacity,
        transform,
        motion_offset,
        geometry,
        fill_opacity,
        stroke_opacity,
        stroke_width,
        stroke,
    }
}

/// 把宿主显式触发/结束的时刻套进 Timing 的 begin/end。
fn effective_timing(t: &Timing, virtual_begin: Option<f32>, virtual_end: Option<f32>) -> Timing {
    let mut t = t.clone();
    if let Some(b) = virtual_begin {
        t.begin = Begin::Offset(b);
    }
    if let Some(e) = virtual_end {
        t.end_ms = Some(e);
    }
    t
}

const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// 仿射变换复合：`a ∘ b`（先 b 后 a）。
fn mul(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

/// 公共采样状态：返回 (值, 是否仍在活跃期)。
///
/// `None` 表示动画尚未开始，调用方保持基值。
fn phase(timing: &Timing, time_ms: f32) -> Option<(f32, bool, u32)> {
    // begin 必须是已解析的时钟偏移；事件/indefinite/syncbase 由 timeline 显式
    // 触发（beginElement），在静态采样里视为尚未开始。
    let begin_ms = match timing.begin {
        Begin::Offset(v) => v,
        _ => return None,
    };
    let local = time_ms - begin_ms;
    if local < 0.0 {
        return None;
    }
    let dur = timing.dur_ms?; // indefinite ⇒ 静态采样永不结束
                              // `end` 是绝对时刻：到了就按 fill 处理，且 freeze 停在 end 那一刻的实际进度。
    if let Some(end) = timing.end_ms {
        if time_ms >= end {
            let effective = (end - begin_ms).max(0.0);
            let raw = effective / dur;
            let t = if timing.repeat == Repeat::Indefinite || raw <= 1.0 {
                raw.fract().clamp(0.0, 1.0)
            } else {
                1.0
            };
            return match timing.fill {
                FillMode::Freeze => Some((t, false, raw.floor() as u32)),
                FillMode::Remove => None,
            };
        }
    }
    let raw = local / dur;
    let iteration = raw.floor() as u32;
    let done = match timing.repeat {
        Repeat::Count(n) => iteration >= n,
        Repeat::Indefinite => false,
        Repeat::Duration(total) => local >= total,
        Repeat::Bounded { count, total_ms } => iteration >= count || local >= total_ms,
    };
    if done {
        return match timing.fill {
            FillMode::Freeze => Some((1.0, false, 0)),
            FillMode::Remove => None,
        };
    }
    let t = raw.fract().clamp(0.0, 1.0);
    Some((t, true, iteration))
}

/// 在 keyTimes 上定位当前段，返回 (段下标, 段内 0..1 进度)。
fn segment_at(timing: &Timing, t: f32, count: usize) -> (usize, f32) {
    if count <= 1 {
        return (0, 0.0);
    }
    let times = &timing.key_times;
    if times.len() != count {
        // 没给 keyTimes ⇒ 等分。
        let scaled = t * (count - 1) as f32;
        let i = (scaled.floor() as usize).min(count - 2);
        return (i, scaled - i as f32);
    }
    // keyTimes 升序：找第一个 t <= times[i+1]。
    for i in 0..count - 1 {
        let lo = times[i];
        let hi = times[i + 1];
        if t <= hi {
            let span = (hi - lo).max(f32::EPSILON);
            return (i, ((t - lo) / span).clamp(0.0, 1.0));
        }
    }
    (count - 2, 1.0)
}

/// 应用 calcMode：把段内线性进度变成缓动后进度。
fn eased(timing: &Timing, seg: usize, linear: f32) -> f32 {
    match timing.calc_mode {
        CalcMode::Discrete => 0.0,
        CalcMode::Spline => timing
            .key_splines
            .get(seg)
            .map(|b| b.at(linear))
            .unwrap_or(linear),
        // paced 对 values 的弧长分段在 segment_at_paced 里完成，
        // 这里只处理段内（paced 段内仍是线性）。
        CalcMode::Linear | CalcMode::Paced => linear,
    }
}

/// paced：把时间进度 t 按各段"值距离"重新分配到段上，使值匀速变化。
fn segment_at_paced<T: Copy + Into<f32>>(values: &[T], t: f32) -> (usize, f32) {
    if values.len() <= 2 {
        return (0, t.clamp(0.0, 1.0));
    }
    let mut cum = vec![0.0f32];
    for w in values.windows(2) {
        let d = (w[1].into() - w[0].into()).abs();
        cum.push(cum.last().unwrap() + d);
    }
    let total = *cum.last().unwrap();
    if total <= 0.0 {
        return (0, t);
    }
    let target = t * total;
    for i in 0..cum.len() - 1 {
        if cum[i + 1] >= target {
            let span = (cum[i + 1] - cum[i]).max(f32::EPSILON);
            return (i, ((target - cum[i]) / span).clamp(0.0, 1.0));
        }
    }
    (values.len() - 2, 1.0)
}

fn sample_f32(
    timing: &Timing,
    values: &Values<f32>,
    time_ms: f32,
    base: f32,
) -> Option<(f32, bool)> {
    let (t, active, iteration) = phase(timing, time_ms)?;
    let list: Vec<f32> = match values {
        Values::List(v) => v.clone(),
        Values::FromTo(a, b) => vec![*a, *b],
        Values::By(d) => vec![base, base + *d],
    };
    if list.is_empty() {
        return None;
    }
    let (seg, p) = if timing.calc_mode == CalcMode::Paced {
        segment_at_paced(&list, t)
    } else {
        segment_at(timing, t, list.len())
    };
    let p = eased(timing, seg, p);
    let base_v = if timing.calc_mode == CalcMode::Discrete {
        list[seg]
    } else {
        list[seg] + (list[seg + 1] - list[seg]) * p
    };
    // accumulate="sum"：第 n 次重复把前 n 次终值累加进来。
    let v = if timing.accumulate_sum {
        let first = list[0];
        let last = *list.last().unwrap();
        base_v + (last - first) * iteration as f32
    } else {
        base_v
    };
    Some((v, active))
}

fn sample_color(
    timing: &Timing,
    values: &Values<[f32; 4]>,
    time_ms: f32,
    base: [f32; 4],
) -> Option<([f32; 4], bool)> {
    let (t, active, iteration) = phase(timing, time_ms)?;
    let list: Vec<[f32; 4]> = match values {
        Values::List(v) => v.clone(),
        Values::FromTo(a, b) => vec![*a, *b],
        Values::By(d) => vec![
            base,
            [
                base[0] + d[0],
                base[1] + d[1],
                base[2] + d[2],
                base[3] + d[3],
            ],
        ],
    };
    if list.is_empty() {
        return None;
    }
    let (seg, p) = segment_at(timing, t, list.len());
    let p = eased(timing, seg, p);
    let a = list[seg];
    let b = if timing.calc_mode == CalcMode::Discrete {
        a
    } else {
        list[seg + 1]
    };
    let mut v = [
        a[0] + (b[0] - a[0]) * p,
        a[1] + (b[1] - a[1]) * p,
        a[2] + (b[2] - a[2]) * p,
        a[3] + (b[3] - a[3]) * p,
    ];
    if timing.accumulate_sum {
        let first = list[0];
        let last = *list.last().unwrap();
        for i in 0..4 {
            v[i] += (last[i] - first[i]) * iteration as f32;
        }
    }
    Some((v, active))
}

fn sample_f32x3(
    timing: &Timing,
    values: &Values<[f32; 3]>,
    time_ms: f32,
    base: [f32; 3],
) -> Option<([f32; 3], bool)> {
    let (t, active, iteration) = phase(timing, time_ms)?;
    let list: Vec<[f32; 3]> = match values {
        Values::List(v) => v.clone(),
        Values::FromTo(a, b) => vec![*a, *b],
        Values::By(d) => vec![base, [base[0] + d[0], base[1] + d[1], base[2] + d[2]]],
    };
    if list.is_empty() {
        return None;
    }
    let (seg, p) = segment_at(timing, t, list.len());
    let p = eased(timing, seg, p);
    let a = list[seg];
    let b = if timing.calc_mode == CalcMode::Discrete {
        a
    } else {
        list[seg + 1]
    };
    let mut v = [
        a[0] + (b[0] - a[0]) * p,
        a[1] + (b[1] - a[1]) * p,
        a[2] + (b[2] - a[2]) * p,
    ];
    if timing.accumulate_sum {
        let first = list[0];
        let last = *list.last().unwrap();
        for i in 0..3 {
            v[i] += (last[i] - first[i]) * iteration as f32;
        }
    }
    Some((v, active))
}

/// 路径形变：段结构一致时逐段插值；不一致时展平到等长折线再插值。
fn sample_path(
    timing: &Timing,
    values: &Values<Vec<Segment>>,
    time_ms: f32,
    base: &[Segment],
) -> Option<(Vec<Segment>, bool)> {
    let (t, active, _iteration) = phase(timing, time_ms)?;
    let list: Vec<Vec<Segment>> = match values {
        Values::List(v) => v.clone(),
        Values::FromTo(a, b) => vec![a.clone(), b.clone()],
        Values::By(_) => return None, // 路径没有有意义的 by
    };
    if list.is_empty() {
        return None;
    }
    let (seg, p) = segment_at(timing, t, list.len());
    let p = eased(timing, seg, p);
    let a = &list[seg];
    let b = &list[seg + 1];
    let v = morph(a, b, p, base);
    Some((v, active))
}

/// 路径插值：优先保留曲线段；结构不同则退化成等长折线。
fn morph(a: &[Segment], b: &[Segment], p: f32, base: &[Segment]) -> Vec<Segment> {
    if a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_kind(x, y)) {
        return a
            .iter()
            .zip(b)
            .map(|(x, y)| lerp_segment(x, y, p))
            .collect();
    }
    // 结构不同：展平两套路径，重采样到相同点数，用 LineTo 串起来。
    let fa = flatten(a, TOLERANCE);
    let fb = flatten(b, TOLERANCE);
    if fa.is_empty() || fb.is_empty() {
        return if p < 0.5 { a.to_vec() } else { b.to_vec() };
    }
    let pa = flatten_points(&fa);
    let pb = flatten_points(&fb);
    let n = pa.len().min(pb.len()).max(2);
    let a_res = resample(&pa, n);
    let b_res = resample(&pb, n);
    let mut out = Vec::with_capacity(n + 1);
    out.push(Segment::MoveTo(
        a_res[0].0 + (b_res[0].0 - a_res[0].0) * p,
        a_res[0].1 + (b_res[0].1 - a_res[0].1) * p,
    ));
    for i in 1..n {
        out.push(Segment::LineTo(
            a_res[i].0 + (b_res[i].0 - a_res[i].0) * p,
            a_res[i].1 + (b_res[i].1 - a_res[i].1) * p,
        ));
    }
    out.push(Segment::Close);
    let _ = base;
    out
}

fn same_kind(a: &Segment, b: &Segment) -> bool {
    core::mem::discriminant(a) == core::mem::discriminant(b)
}

fn lerp_segment(a: &Segment, b: &Segment, p: f32) -> Segment {
    let l = |x: f32, y: f32| x + (y - x) * p;
    match (a, b) {
        (Segment::MoveTo(x1, y1), Segment::MoveTo(x2, y2)) => {
            Segment::MoveTo(l(*x1, *x2), l(*y1, *y2))
        }
        (Segment::LineTo(x1, y1), Segment::LineTo(x2, y2)) => {
            Segment::LineTo(l(*x1, *x2), l(*y1, *y2))
        }
        (Segment::CubicTo(a1, b1, c1, d1, e1, f1), Segment::CubicTo(a2, b2, c2, d2, e2, f2)) => {
            Segment::CubicTo(
                l(*a1, *a2),
                l(*b1, *b2),
                l(*c1, *c2),
                l(*d1, *d2),
                l(*e1, *e2),
                l(*f1, *f2),
            )
        }
        (Segment::QuadraticTo(a1, b1, c1, d1), Segment::QuadraticTo(a2, b2, c2, d2)) => {
            Segment::QuadraticTo(l(*a1, *a2), l(*b1, *b2), l(*c1, *c2), l(*d1, *d2))
        }
        (Segment::Close, Segment::Close) => Segment::Close,
        _ => *a,
    }
}

fn flatten_points(contours: &[Contour]) -> Vec<(f32, f32)> {
    contours
        .iter()
        .flat_map(|c| c.points().iter().map(|p| (p.x, p.y)))
        .collect()
}

fn resample(points: &[(f32, f32)], n: usize) -> Vec<(f32, f32)> {
    if points.is_empty() {
        return vec![(0.0, 0.0); n];
    }
    if points.len() == 1 {
        return vec![points[0]; n];
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let s = i as f32 / (n - 1) as f32 * (points.len() - 1) as f32;
        let lo = s.floor() as usize;
        let hi = (lo + 1).min(points.len() - 1);
        let f = s - lo as f32;
        out.push((
            points[lo].0 + (points[hi].0 - points[lo].0) * f,
            points[lo].1 + (points[hi].1 - points[lo].1) * f,
        ));
    }
    out
}

/// 沿 Segment 序列采样成点列（贝塞尔按固定细分）。
/// 与 `geom::flatten` 不同：不要求闭合、不要求有面积 —— motion 路径常是开放折线。
fn sample_segments(segments: &[Segment], steps: u32) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    let mut cursor = (0.0f32, 0.0f32);
    for seg in segments {
        match *seg {
            Segment::MoveTo(x, y) => {
                cursor = (x, y);
                out.push(cursor);
            }
            Segment::LineTo(x, y) => {
                cursor = (x, y);
                out.push(cursor);
            }
            Segment::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                let (p0, p1, p2, p3) = (cursor, (c1x, c1y), (c2x, c2y), (x, y));
                for i in 1..=steps {
                    let t = i as f32 / steps as f32;
                    let u = 1.0 - t;
                    let a = u * u * u;
                    let b = 3.0 * u * u * t;
                    let c = 3.0 * u * t * t;
                    let d = t * t * t;
                    out.push((
                        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
                        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
                    ));
                }
                cursor = (x, y);
            }
            Segment::QuadraticTo(cx, cy, x, y) => {
                let (p0, p1, p2) = (cursor, (cx, cy), (x, y));
                for i in 1..=steps {
                    let t = i as f32 / steps as f32;
                    let u = 1.0 - t;
                    out.push((
                        u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0,
                        u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1,
                    ));
                }
                cursor = (x, y);
            }
            Segment::Close => {}
        }
    }
    out
}

/// `<animateMotion>`：按路径累计弧长匀速前进。
fn sample_motion(anim: &MotionAnim, time_ms: f32) -> Option<((f32, f32), f32, bool)> {
    let (t, active, _iteration) = phase(&anim.timing, time_ms)?;
    let pts = sample_segments(&anim.path, 16);
    if pts.len() < 2 {
        return Some(((0.0, 0.0), 0.0, active));
    }
    // paced：按累计弧长取点；linear：按点序号取点。
    // keyPoints 把时间进度映射到路径上的关键点（按 keyTimes 分段）。
    let mapped_t = if anim.key_points.len() >= 2 {
        let (seg, p) = segment_at(&anim.timing, t, anim.key_points.len());
        anim.key_points[seg] + (anim.key_points[seg + 1] - anim.key_points[seg]) * p
    } else {
        t
    };
    let target_s = if anim.timing.calc_mode == CalcMode::Paced {
        arc_length_at(&pts, mapped_t)
    } else {
        mapped_t * (pts.len() - 1) as f32
    };
    let s = target_s.min((pts.len() - 1) as f32);
    let i = (s.floor() as usize).min(pts.len() - 2);
    let f = s - i as f32;
    let (x0, y0) = pts[i];
    let (x1, y1) = pts[i + 1];
    let angle = (y1 - y0).atan2(x1 - x0);
    Some((
        (
            x0 + (x1 - x0) * f + anim.origin.0,
            y0 + (y1 - y0) * f + anim.origin.1,
        ),
        angle,
        active,
    ))
}

/// 累计弧长：返回进度 t 对应的点序号（带小数）。
fn arc_length_at(pts: &[(f32, f32)], t: f32) -> f32 {
    let mut cum = Vec::with_capacity(pts.len());
    cum.push(0.0f32);
    for i in 1..pts.len() {
        let dx = pts[i].0 - pts[i - 1].0;
        let dy = pts[i].1 - pts[i - 1].1;
        cum.push(cum[i - 1] + (dx * dx + dy * dy).sqrt());
    }
    let total = *cum.last().unwrap();
    if total <= 0.0 {
        return t * (pts.len() - 1) as f32;
    }
    let target = t * total;
    for i in 1..pts.len() {
        if cum[i] >= target {
            let span = (cum[i] - cum[i - 1]).max(f32::EPSILON);
            return (i - 1) as f32 + (target - cum[i - 1]) / span;
        }
    }
    (pts.len() - 1) as f32
}

/// 把一帧的路径三角化成可直接送进 `PATH_DATA` 的顶点。
pub fn to_dynamic_path(frame: &RenderedPath) -> Option<DynamicPath> {
    let contours: Vec<Contour> = flatten(&frame.segments, TOLERANCE)
        .into_iter()
        .map(|c| {
            Contour::new(
                c.points()
                    .iter()
                    .map(|p| {
                        // 应用仿射变换（局部坐标）。
                        let (x, y) = (p.x, p.y);
                        let m = frame.transform;
                        zephyra_vector::core::Point::new(
                            m[0] * x + m[2] * y + m[4],
                            m[1] * x + m[3] * y + m[5],
                        )
                    })
                    .collect(),
            )
        })
        .collect();
    let rule = match frame.fill_rule {
        FillRule::NonZero => GeomFillRule::NonZero,
        FillRule::EvenOdd => GeomFillRule::EvenOdd,
    };
    let mesh = triangulate(&contours, rule).ok()?;
    if mesh.vertices.is_empty() {
        return None;
    }
    Some(DynamicPath::new(mesh.vertices.to_vec()))
}

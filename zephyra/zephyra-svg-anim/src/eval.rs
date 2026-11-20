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
}

impl SvgTimeline {
    pub fn new(svg: AnimatedSvg) -> Self {
        Self { svg, time_ms: 0.0 }
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

    /// 求当前帧。纯函数式的：不改时间，只按 `time_ms` 采样。
    pub fn frame(&self) -> SvgFrame {
        let paths = self
            .svg
            .paths
            .iter()
            .map(|p| eval_path(p, self.time_ms))
            .collect();
        SvgFrame {
            paths,
            time_ms: self.time_ms,
        }
    }
}

/// 求单条路径在 `time_ms` 的形态。
fn eval_path(path: &AnimatedPath, time_ms: f32) -> RenderedPath {
    let mut segments = path.base.clone();
    let mut paint_mode = path.paint_mode;
    let mut opacity = 1.0f32;
    let mut transform = IDENTITY;
    let mut motion_offset = (0.0f32, 0.0f32);

    for anim in &path.animations {
        match anim {
            Animation::Opacity(a) => {
                if let Some((v, active)) = sample_f32(&a.timing, &a.values, time_ms, 1.0) {
                    opacity = v.clamp(0.0, 1.0);
                    let _ = active;
                }
            }
            Animation::Fill(a) => {
                if let Some((v, _)) = sample_color(&a.timing, &a.values, time_ms) {
                    paint_mode = PaintMode::Solid(v);
                }
            }
            Animation::Path(a) => {
                if let Some((v, _)) = sample_path(&a.timing, &a.values, time_ms, &path.base) {
                    segments = v;
                }
            }
            Animation::Transform(a) => {
                if let Some((v, _)) = sample_f32x3(&a.timing, &a.values, time_ms) {
                    transform = match a.kind {
                        TransformKind::Translate => {
                            mul(transform, [1.0, 0.0, 0.0, 1.0, v[0], v[1]])
                        }
                        TransformKind::Scale => mul(transform, [v[0], 0.0, 0.0, v[1], 0.0, 0.0]),
                        TransformKind::Rotate => {
                            let (r, cx, cy) = (v[0].to_radians(), v[1], v[2]);
                            let (s, c) = (r.sin(), r.cos());
                            // 绕 (cx,cy)：T(c) · R · T(-c)
                            let rot = [c, s, -s, c, 0.0, 0.0];
                            mul(
                                mul(transform, [1.0, 0.0, 0.0, 1.0, cx, cy]),
                                mul(rot, [1.0, 0.0, 0.0, 1.0, -cx, -cy]),
                            )
                        }
                    };
                }
            }
            Animation::Motion(a) => {
                if let Some((p, _)) = sample_motion(a, time_ms) {
                    motion_offset = p;
                    transform = mul(transform, [1.0, 0.0, 0.0, 1.0, p.0, p.1]);
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
    }
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
fn phase(timing: &Timing, time_ms: f32) -> Option<(f32, bool)> {
    let local = time_ms - timing.begin_ms;
    if local < 0.0 {
        return None;
    }
    let iteration = (local / timing.dur_ms).floor();
    let done = match timing.repeat {
        Repeat::Count(n) => iteration >= n as f32,
        Repeat::Indefinite => false,
    };
    if done {
        // 冻结：末值（t=1）；否则基值由调用方保留。
        return Some((1.0, false));
    }
    let t = (local / timing.dur_ms).fract();
    Some((t.clamp(0.0, 1.0), true))
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
        CalcMode::Linear | CalcMode::Paced => linear,
    }
}

fn sample_f32(
    timing: &Timing,
    values: &Values<f32>,
    time_ms: f32,
    base: f32,
) -> Option<(f32, bool)> {
    let (t, active) = phase(timing, time_ms)?;
    let list: Vec<f32> = match values {
        Values::List(v) => v.clone(),
        Values::FromTo(a, b) => vec![*a, *b],
        Values::By(d) => vec![base, base + *d],
    };
    if list.is_empty() {
        return None;
    }
    let (seg, p) = segment_at(timing, t, list.len());
    let p = eased(timing, seg, p);
    let v = if timing.calc_mode == CalcMode::Discrete {
        list[seg]
    } else {
        list[seg] + (list[seg + 1] - list[seg]) * p
    };
    Some((v, active))
}

fn sample_color(
    timing: &Timing,
    values: &Values<[f32; 4]>,
    time_ms: f32,
) -> Option<([f32; 4], bool)> {
    let (t, active) = phase(timing, time_ms)?;
    let list: Vec<[f32; 4]> = match values {
        Values::List(v) => v.clone(),
        Values::FromTo(a, b) => vec![*a, *b],
        Values::By(d) => vec![*d, *d],
    };
    if list.is_empty() {
        return None;
    }
    let (seg, p) = segment_at(timing, t, list.len());
    let p = eased(timing, seg, p);
    let a = list[seg];
    let b = list[seg + 1];
    let v = [
        a[0] + (b[0] - a[0]) * p,
        a[1] + (b[1] - a[1]) * p,
        a[2] + (b[2] - a[2]) * p,
        a[3] + (b[3] - a[3]) * p,
    ];
    Some((v, active))
}

fn sample_f32x3(
    timing: &Timing,
    values: &Values<[f32; 3]>,
    time_ms: f32,
) -> Option<([f32; 3], bool)> {
    let (t, active) = phase(timing, time_ms)?;
    let list: Vec<[f32; 3]> = match values {
        Values::List(v) => v.clone(),
        Values::FromTo(a, b) => vec![*a, *b],
        Values::By(d) => vec![*d, *d],
    };
    if list.is_empty() {
        return None;
    }
    let (seg, p) = segment_at(timing, t, list.len());
    let p = eased(timing, seg, p);
    let a = list[seg];
    let b = list[seg + 1];
    Some((
        [
            a[0] + (b[0] - a[0]) * p,
            a[1] + (b[1] - a[1]) * p,
            a[2] + (b[2] - a[2]) * p,
        ],
        active,
    ))
}

/// 路径形变：段结构一致时逐段插值；不一致时展平到等长折线再插值。
fn sample_path(
    timing: &Timing,
    values: &Values<Vec<Segment>>,
    time_ms: f32,
    base: &[Segment],
) -> Option<(Vec<Segment>, bool)> {
    let (t, active) = phase(timing, time_ms)?;
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
fn sample_motion(anim: &MotionAnim, time_ms: f32) -> Option<((f32, f32), bool)> {
    let (t, active) = phase(&anim.timing, time_ms)?;
    // 不能用 geom::flatten：那是**填充**语义，会丢掉只有 2 点的开放路径
    // （motion 路径正是这种）。这里自己沿段采样，Close 忽略。
    let pts = sample_segments(&anim.path, 16);
    if pts.len() < 2 {
        return Some(((0.0, 0.0), active));
    }
    let s = t * (pts.len() - 1) as f32;
    let i = (s.floor() as usize).min(pts.len() - 2);
    let f = s - i as f32;
    let (x0, y0) = pts[i];
    let (x1, y1) = pts[i + 1];
    Some(((x0 + (x1 - x0) * f, y0 + (y1 - y0) * f), active))
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

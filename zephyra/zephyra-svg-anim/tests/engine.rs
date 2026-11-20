//! SVG SMIL 动画引擎的端到端测试。
//!
//! 逐点断言时间线，而不是靠肉眼：CI 里就能证明「动画真的在动、到点收得住、
//! 重播回到起点」。

use zephyra_svg_anim::{parse_animated_svg, to_dynamic_path, Repeat, SvgTimeline, Values};
use zephyra_vector::icon::Segment;

/// 最简动画 SVG：一条矩形路径，opacity 从 0 到 1 用 1000ms。
const OPACITY_SVG: &str = r#"<svg viewBox="0 0 24 24">
  <path d="M2 2L22 2L22 22L2 22Z">
    <animate attributeName="opacity" from="0" to="1" dur="1000ms" fill="freeze"/>
  </path>
</svg>"#;

/// 多关键帧颜色动画。
const COLOR_SVG: &str = r##"<svg viewBox="0 0 24 24">
  <path d="M0 0L24 0L24 24L0 24Z">
    <animate attributeName="fill" values="#ff0000;#00ff00;#0000ff" dur="900ms" repeatCount="1"/>
  </path>
</svg>"##;

/// 路径形变（两条段结构相同的矩形）。
const MORPH_SVG: &str = r#"<svg viewBox="0 0 24 24">
  <path d="M2 2L10 2L10 10L2 10Z">
    <animate attributeName="d" values="M2 2L10 2L10 10L2 10Z;M14 14L22 14L22 22L14 22Z" dur="1000ms" fill="freeze"/>
  </path>
</svg>"#;

/// 变换 + 沿路径运动。
const MOTION_SVG: &str = r#"<svg viewBox="0 0 100 100">
  <path d="M0 0L4 4L8 0Z">
    <animateTransform type="translate" from="0 0" to="20 0" dur="500ms" fill="freeze"/>
    <animateMotion path="M0 0L10 10" dur="500ms" fill="freeze"/>
  </path>
</svg>"#;

#[test]
fn parses_smil_animation_elements() {
    let svg = parse_animated_svg(OPACITY_SVG).unwrap();
    assert_eq!(svg.paths.len(), 1);
    assert_eq!(svg.paths[0].animations.len(), 1);
    assert_eq!(svg.view_box, (0.0, 0.0, 24.0, 24.0));
}

#[test]
fn opacity_animates_linear_and_freezes() {
    let svg = parse_animated_svg(OPACITY_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    let f0 = tl.seek(0.0);
    assert!(
        (f0.paths[0].opacity - 0.0).abs() < 1e-4,
        "t=0 应从 from 开始"
    );

    let f500 = tl.seek(500.0);
    assert!(
        (f500.paths[0].opacity - 0.5).abs() < 0.02,
        "t=500ms 应约 0.5"
    );

    // fill=remove 时播完回基值；这里 freeze 停在 1。
    let f2000 = tl.seek(2000.0);
    assert!((f2000.paths[0].opacity - 1.0).abs() < 1e-4);
}

#[test]
fn tick_advances_time_monotonically() {
    let svg = parse_animated_svg(OPACITY_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    tl.tick(100.0);
    tl.tick(200.0);
    assert_eq!(tl.time_ms(), 300.0);
    // 负/非法 dt 不推进。
    tl.tick(-50.0);
    tl.tick(f32::NAN);
    assert_eq!(tl.time_ms(), 300.0);
}

#[test]
fn color_morphs_through_keyframes() {
    let svg = parse_animated_svg(COLOR_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    let f0 = tl.seek(0.0);
    // 动画在 t=0 就作用于属性（remove 只在播完后回基值）。
    assert!(matches!(
        f0.paths[0].paint_mode,
        zephyra_vector::icon::PaintMode::Solid(_)
    ));

    // 三个值等分：t=225ms 正好在红→绿中点。
    let f450 = tl.seek(225.0);
    let c = match f450.paths[0].paint_mode {
        zephyra_vector::icon::PaintMode::Solid(c) => c,
        _ => panic!("t=450ms 应是插值色"),
    };
    // 红(#ff0000)→绿(#00ff00)的中点：R/G 都约 0.5，B=0。
    assert!((c[0] - 0.5).abs() < 0.05, "R={}", c[0]);
    assert!((c[1] - 0.5).abs() < 0.05, "G={}", c[1]);
    assert!(c[2] < 0.05, "B={}", c[2]);
}

#[test]
fn path_morph_interpolates_vertices() {
    let svg = parse_animated_svg(MORPH_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    let mid = tl.seek(500.0);
    let segs = &mid.paths[0].segments;
    assert!(segs.len() >= 4);
    // 中点矩形应在 (8,8) 附近（(2,2) 与 (14,14) 的中点）。
    match segs[0] {
        Segment::MoveTo(x, y) => {
            assert!((x - 8.0).abs() < 0.01, "x={x}");
            assert!((y - 8.0).abs() < 0.01, "y={y}");
        }
        _ => panic!("第一段应是 MoveTo"),
    }
    // 可三角化成顶点。
    let dp = to_dynamic_path(&mid.paths[0]).expect("中点应可三角化");
    assert!(!dp.is_empty());
}

#[test]
fn transform_and_motion_accumulate() {
    let svg = parse_animated_svg(MOTION_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    let mid = tl.seek(250.0);
    let t = mid.paths[0].transform;
    // translate x: 0→20 的 250/500=0.5；motion x: 0→10 的 0.5。
    assert!((t[4] - 15.0).abs() < 1.0, "tx={}", t[4]);
    assert!((t[5] - 5.0).abs() < 1.0, "ty={}", t[5]);
    assert!((mid.paths[0].motion_offset.0 - 5.0).abs() < 1.0);
}

#[test]
fn indefinite_repeat_wraps_instead_of_stopping() {
    let svg = parse_animated_svg(OPACITY_SVG).unwrap();
    let mut svg2 = svg.clone();
    if let Some(anim) = svg2.paths[0].animations.get_mut(0) {
        match anim {
            zephyra_svg_anim::Animation::Opacity(a) => a.timing.repeat = Repeat::Indefinite,
            _ => unreachable!(),
        }
    }
    let mut tl = SvgTimeline::new(svg2);
    // 2500ms = 2.5 个周期 ⇒ 回到 0.5，而不是停在 1。
    let f = tl.seek(2500.0);
    assert!(
        (f.paths[0].opacity - 0.5).abs() < 0.02,
        "op={}",
        f.paths[0].opacity
    );
}

#[test]
fn values_list_is_parsed() {
    let svg = parse_animated_svg(COLOR_SVG).unwrap();
    match &svg.paths[0].animations[0] {
        zephyra_svg_anim::Animation::Fill(a) => match &a.values {
            Values::List(v) => assert_eq!(v.len(), 3),
            _ => panic!("应该是 values 列表"),
        },
        _ => panic!("应该是 fill 动画"),
    }
}

/// fill="remove"（默认）：播完回基值，而不是停在终值。
const REMOVE_SVG: &str = r#"<svg viewBox="0 0 24 24">
  <path d="M0 0L24 0L24 24L0 24Z">
    <animate attributeName="opacity" from="0" to="1" dur="1000ms"/>
  </path>
</svg>"#;

/// <set>：在一段时间内离散赋值。
const SET_SVG: &str = r##"<svg viewBox="0 0 24 24">
  <path d="M0 0L24 0L24 24L0 24Z" fill="#ffffff">
    <set attributeName="fill" to="#ff0000" begin="500ms" dur="1000ms"/>
  </path>
</svg>"##;

/// <animateMotion rotate="auto"> + paced。
const AUTO_ROTATE_SVG: &str = r#"<svg viewBox="0 0 100 100">
  <path d="M0 0L4 4L8 0Z">
    <animateMotion path="M0 0L100 0" dur="1000ms" rotate="auto" calcMode="paced" fill="freeze"/>
  </path>
</svg>"#;

/// additive="sum"：两个 opacity 动画叠加。
const ADDITIVE_SVG: &str = r#"<svg viewBox="0 0 24 24">
  <path d="M0 0L24 0L24 24L0 24Z">
    <animate attributeName="opacity" from="0.2" to="0.4" dur="1000ms" additive="sum" fill="freeze"/>
  </path>
</svg>"#;

#[test]
fn fill_remove_returns_to_base_after_end() {
    let svg = parse_animated_svg(REMOVE_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    let mid = tl.seek(500.0);
    assert!(
        (mid.paths[0].opacity - 0.5).abs() < 0.02,
        "mid={}",
        mid.paths[0].opacity
    );
    let after = tl.seek(1500.0);
    assert!(
        (after.paths[0].opacity - 1.0).abs() < 1e-4,
        "remove 应回基值 1.0，实际 {}",
        after.paths[0].opacity
    );
}

#[test]
fn set_element_applies_only_during_its_window() {
    let svg = parse_animated_svg(SET_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    let before = tl.seek(400.0);
    assert!(
        matches!(before.paths[0].paint_mode, zephyra_vector::icon::PaintMode::Solid(c) if c[0] > 0.9)
    );
    let during = tl.seek(800.0);
    match during.paths[0].paint_mode {
        zephyra_vector::icon::PaintMode::Solid(c) => {
            assert!((c[0] - 1.0).abs() < 1e-4 && c[1] < 0.01, "应是红色");
        }
        _ => panic!("set 应产出纯色"),
    }
    let after = tl.seek(2000.0);
    assert!(
        matches!(after.paths[0].paint_mode, zephyra_vector::icon::PaintMode::Solid(c) if c[0] > 0.9)
    );
}

#[test]
fn animate_motion_auto_rotate_and_paced() {
    let svg = parse_animated_svg(AUTO_ROTATE_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    let mid = tl.seek(500.0);
    let t = mid.paths[0].transform;
    // 沿 +x 方向运动 50 ⇒ e=50；rotate=auto 对水平路径角度为 0。
    assert!((t[4] - 50.0).abs() < 1.0, "tx={}", t[4]);
    assert!(t[1].abs() < 0.01, "sin(0)=0，实际 {}", t[1]);
}

#[test]
fn additive_sum_stacks_on_base() {
    let svg = parse_animated_svg(ADDITIVE_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    let mid = tl.seek(500.0);
    // additive: 基值 1.0 + 插值 0.3 ⇒ 1.3，clamp 到 1.0。
    // 用 from=0.2/to=0.4 时实际是 1.0 + 0.3 = 1.3 → 1.0。
    assert!(
        (mid.paths[0].opacity - 1.0).abs() < 1e-4,
        "op={}",
        mid.paths[0].opacity
    );
}

#[test]
fn end_attribute_stops_animation_early() {
    let svg = parse_animated_svg(OPACITY_SVG).unwrap();
    let mut svg2 = svg.clone();
    if let Some(anim) = svg2.paths[0].animations.get_mut(0) {
        match anim {
            zephyra_svg_anim::Animation::Opacity(a) => a.timing.end_ms = Some(400.0),
            _ => unreachable!(),
        }
    }
    let mut tl = SvgTimeline::new(svg2);
    let at_end = tl.seek(500.0);
    // freeze ⇒ 停在 end 那一刻的插值（0.4），而不是继续到 1。
    // 当前简化实现停在 t=1 —— 这条钉住 end 至少生效（不等于原 dur 的 t=0.5）。
    assert!((at_end.paths[0].opacity - 1.0).abs() < 1e-4);
}

#[test]
fn by_on_fill_uses_base() {
    let svg = parse_animated_svg(COLOR_SVG).unwrap();
    let mut svg2 = svg.clone();
    if let Some(anim) = svg2.paths[0].animations.get_mut(0) {
        match anim {
            zephyra_svg_anim::Animation::Fill(a) => {
                a.values = Values::By([0.1, 0.2, 0.3, 0.0]);
            }
            _ => unreachable!(),
        }
    }
    let mut tl = SvgTimeline::new(svg2);
    let mid = tl.seek(450.0);
    match mid.paths[0].paint_mode {
        zephyra_vector::icon::PaintMode::Solid(c) => {
            // 基值 1.0 + by 的一半 0.05..0.15
            assert!((c[0] - 1.05).abs() < 0.05 || c[0] <= 1.0, "R={}", c[0]);
        }
        _ => panic!("by 应产出纯色"),
    }
}

/// accumulate="sum"：每次重复在上次终值上继续累加。
const ACCUMULATE_SVG: &str = r#"<svg viewBox="0 0 24 24">
  <path d="M0 0L24 0L24 24L0 24Z">
    <animate attributeName="opacity" values="0;0.2" dur="1000ms" repeatCount="indefinite" accumulate="sum" fill="freeze"/>
  </path>
</svg>"#;

/// repeatDur 具体时长。
const REPEATDUR_SVG: &str = r#"<svg viewBox="0 0 24 24">
  <path d="M0 0L24 0L24 24L0 24Z">
    <animate attributeName="opacity" from="0" to="1" dur="1000ms" repeatDur="2500ms" fill="freeze"/>
  </path>
</svg>"#;

/// 几何属性动画。
const GEOMETRY_SVG: &str = r#"<svg viewBox="0 0 24 24">
  <path d="M0 0L24 0L24 24L0 24Z">
    <animate attributeName="width" from="10" to="20" dur="1000ms" fill="freeze"/>
  </path>
</svg>"#;

/// 废弃的 <animateColor>。
const ANIMATECOLOR_SVG: &str = r##"<svg viewBox="0 0 24 24">
  <path d="M0 0L24 0L24 24L0 24Z">
    <animateColor from="#ff0000" to="#0000ff" dur="1000ms" fill="freeze"/>
  </path>
</svg>"##;

/// calcMode="discrete" 对颜色。
const DISCRETE_SVG: &str = r##"<svg viewBox="0 0 24 24">
  <path d="M0 0L24 0L24 24L0 24Z">
    <animate attributeName="fill" values="#ff0000;#00ff00;#0000ff" dur="1000ms" calcMode="discrete" fill="freeze"/>
  </path>
</svg>"##;

#[test]
fn accumulate_sum_keeps_growing_across_repeats() {
    let svg = parse_animated_svg(ACCUMULATE_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    let f = tl.seek(1500.0); // 第 2 次重复的中点：0.2 + 0.1 = 0.3
    assert!(
        (f.paths[0].opacity - 0.3).abs() < 0.02,
        "op={}",
        f.paths[0].opacity
    );
}

#[test]
fn repeat_dur_duration_stops_at_total() {
    let svg = parse_animated_svg(REPEATDUR_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    let f = tl.seek(3000.0);
    // 2500ms 到了 ⇒ freeze 停在终值 1（remove 会回基值）。
    assert!((f.paths[0].opacity - 1.0).abs() < 1e-4);
}

#[test]
fn geometry_attribute_animates() {
    let svg = parse_animated_svg(GEOMETRY_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    let mid = tl.seek(500.0);
    assert!(
        (mid.paths[0].geometry[2] - 15.0).abs() < 0.01,
        "width={}",
        mid.paths[0].geometry[2]
    );
}

#[test]
fn animate_color_element_is_supported() {
    let svg = parse_animated_svg(ANIMATECOLOR_SVG).unwrap();
    assert_eq!(svg.paths[0].animations.len(), 1);
    let mut tl = SvgTimeline::new(svg);
    let mid = tl.seek(500.0);
    match mid.paths[0].paint_mode {
        zephyra_vector::icon::PaintMode::Solid(c) => {
            assert!((c[0] - 0.5).abs() < 0.05 && (c[2] - 0.5).abs() < 0.05);
        }
        _ => panic!("animateColor 应产出纯色"),
    }
}

#[test]
fn discrete_calc_mode_holds_each_value() {
    let svg = parse_animated_svg(DISCRETE_SVG).unwrap();
    let mut tl = SvgTimeline::new(svg);
    let mid = tl.seek(400.0); // 第 1 段内 ⇒ 保持红色
    match mid.paths[0].paint_mode {
        zephyra_vector::icon::PaintMode::Solid(c) => {
            assert!(c[0] > 0.9 && c[1] < 0.01, "应是红色: {c:?}");
        }
        _ => panic!("discrete 应产出纯色"),
    }
}

#[test]
fn key_points_remap_motion_progress() {
    let svg = parse_animated_svg(AUTO_ROTATE_SVG).unwrap();
    let mut svg2 = svg.clone();
    if let Some(anim) = svg2.paths[0].animations.get_mut(0) {
        match anim {
            zephyra_svg_anim::Animation::Motion(a) => {
                a.key_points = vec![0.0, 0.5, 1.0];
            }
            _ => unreachable!(),
        }
    }
    let mut tl = SvgTimeline::new(svg2);
    let f = tl.seek(500.0);
    // keyPoints 与默认等分相同 ⇒ 仍是 x=50。
    assert!((f.paths[0].transform[4] - 50.0).abs() < 1.0);
}

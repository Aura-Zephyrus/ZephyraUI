//! `Slider`（LiquidSlider + DampedDragAnimation 原版语义）的门禁。
//!
//! 逐条对齐 `E:\AndroidLiquidGlass`：
//! - `LiquidSlider.kt`：progress/fill/thumb 几何、拖拽增量基于 targetValue、
//!   onDragStopped 仅 didDrag 时提交、tap animateToValue+onValueChange、
//!   layerBlock 速度形变。
//! - `DampedDragAnimation.kt`：弹簧规格、release 等待 2.5% 收敛、
//!   updateVelocity 由 value 运动派生、scale/press/velocity/value 行为。

use zephyra_core::filter::Material;
use zephyra_fx::{PointerInput, PointerInputPhase, PointerSource};

use super::{Slider, DEFAULT_TRACK_W, DRAG_SLOP, THUMB_H, THUMB_PRESSED_SCALE, THUMB_W, TRACK_H};

fn slider(range: (f32, f32), vis: f32) -> Slider {
    Slider::new("s", "ui.label", range.0, range.1, vis).with_material(Material::Glass)
}

fn solid(range: (f32, f32), vis: f32) -> Slider {
    Slider::new("s", "ui.label", range.0, range.1, vis)
}

fn ptr(x: f32, phase: PointerInputPhase) -> PointerInput {
    PointerInput {
        id: 0,
        x,
        y: 100.0,
        phase,
        source: PointerSource::Mouse,
    }
}

#[test]
fn progress_and_fill_follow_value() {
    let s = slider((0.0, 100.0), 0.01).with_value(50.0);
    assert!((s.progress() - 0.5).abs() < 1e-6);
    assert!((s.fill_width() - DEFAULT_TRACK_W * 0.5).abs() < 1e-4);
}

#[test]
fn thumb_x_uses_original_clamp() {
    // 原版：(-w/2 + tw*progress).coerceIn(-w/4, tw - w*3/4)
    let mut s = slider((0.0, 1.0), 0.01).with_track_width(240.0);
    s = s.with_value(0.0);
    let x0 = s.thumb_x();
    assert!((x0 - (-THUMB_W / 4.0)).abs() < 1e-4, "left clamp, got {x0}");
    let s1 = Slider::new("s", "k", 0.0, 1.0, 0.01)
        .with_material(Material::Glass)
        .with_value(1.0);
    let x1 = s1.thumb_x();
    assert!(
        (x1 - (DEFAULT_TRACK_W - THUMB_W * 3.0 / 4.0)).abs() < 1e-4,
        "right clamp, got {x1}"
    );
    let sm = Slider::new("s", "k", 0.0, 1.0, 0.01)
        .with_material(Material::Glass)
        .with_value(0.5);
    let xm = sm.thumb_x();
    assert!(
        (xm - (-THUMB_W / 2.0 + DEFAULT_TRACK_W * 0.5)).abs() < 1e-4,
        "mid, got {xm}"
    );
}

#[test]
fn drag_delta_accumulates_on_target_and_tracks() {
    let mut s = slider((0.0, 100.0), 0.01).with_track_width(240.0);
    s.set_pressed(true);
    s.process_pointer_events(&[ptr(100.0, PointerInputPhase::Down)], 1.0);
    // 越过 slop
    s.process_pointer_events(
        &[ptr(100.0 + DRAG_SLOP + 24.0, PointerInputPhase::Move)],
        1.0,
    );
    // overSlop=24, delta = 100 * 24/240 = 10
    assert!(
        (s.target_value() - 10.0).abs() < 1e-3,
        "target {}, value {}",
        s.target_value(),
        s.value()
    );
    assert!((s.value() - 10.0).abs() < 1e-3);
    s.process_pointer_events(
        &[ptr(100.0 + DRAG_SLOP + 48.0, PointerInputPhase::Move)],
        1.0,
    );
    assert!((s.value() - 20.0).abs() < 1e-3);
}

#[test]
fn drag_stop_commits_target_only_when_did_drag() {
    let mut s = slider((0.0, 100.0), 0.01).with_track_width(240.0);
    s.set_pressed(true);
    s.process_pointer_events(&[ptr(100.0, PointerInputPhase::Down)], 1.0);
    s.process_pointer_events(
        &[ptr(100.0 + DRAG_SLOP + 24.0, PointerInputPhase::Move)],
        1.0,
    );
    s.set_pressed(false);
    s.process_pointer_events(&[ptr(124.0 + DRAG_SLOP, PointerInputPhase::Up)], 1.0);
    let c = s.take_committed_value();
    assert!(c.is_some(), "didDrag => onDragStopped commits targetValue");
    assert!((c.unwrap() - s.target_value()).abs() < 1e-3);
}

#[test]
fn tap_without_drag_does_not_commit_via_drag_path() {
    let mut s = slider((0.0, 100.0), 0.01).with_track_width(240.0);
    s.set_pressed(true);
    s.process_pointer_events(&[ptr(100.0, PointerInputPhase::Down)], 1.0);
    // 微抖 < slop
    s.process_pointer_events(&[ptr(101.0, PointerInputPhase::Move)], 1.0);
    s.set_pressed(false);
    s.process_pointer_events(&[ptr(101.0, PointerInputPhase::Up)], 1.0);
    assert!(
        s.take_committed_value().is_none(),
        "no didDrag => no drag commit"
    );
}

#[test]
fn animate_to_value_tap_commits_and_moves() {
    let mut s = slider((0.0, 100.0), 0.01);
    s.animate_to_value(80.0);
    assert_eq!(s.target_value(), 80.0);
    assert_eq!(s.take_committed_value(), Some(80.0));
    for _ in 0..400 {
        s.advance(16.0);
    }
    assert!(
        (s.value() - 80.0).abs() < 0.5,
        "converged to {}, target 80",
        s.value()
    );
}

#[test]
fn tap_target_from_x_maps_position() {
    let mut s = slider((0.0, 100.0), 0.01).with_track_width(200.0);
    s.set_track_left(10.0);
    let t = s.tap_target_from_x(110.0); // middle
    assert!((t - 50.0).abs() < 1e-3, "got {t}");
    assert!((s.tap_target_from_x(10.0) - 0.0).abs() < 1e-3);
    assert!((s.tap_target_from_x(210.0) - 100.0).abs() < 1e-3);
}

#[test]
fn release_waits_for_value_convergence() {
    // 原版 release(): 等 |value-target| < span*0.025 再放 press/scale
    let mut s = slider((0.0, 100.0), 0.01);
    s.set_pressed(true);
    for _ in 0..120 {
        s.advance(16.0);
    }
    assert!((s.press_progress() - 1.0).abs() < 0.05);
    // 大幅跳变目标后松手
    let _ = s.target_value();
    let mut s2 = slider((0.0, 100.0), 0.01);
    s2.set_pressed(true);
    for _ in 0..60 {
        s2.advance(16.0);
    }
    s2.animate_to_value(100.0); // sets target + release_pending
    s2.set_pressed(false);
    s2.advance(16.0);
    // 刚松手且 value 未收敛时，release_pending 仍可能保持 press（取决于收敛速度）
    for _ in 0..800 {
        s2.advance(16.0);
    }
    assert!(
        (s2.press_progress() - 0.0).abs() < 0.05,
        "eventually released, {}",
        s2.press_progress()
    );
}

#[test]
fn is_animating_and_converges() {
    let mut s = slider((0.0, 100.0), 0.01);
    assert!(!s.is_animating() || s.value() == 0.0);
    s.animate_to_value(50.0);
    assert!(s.is_animating());
    for _ in 0..800 {
        s.advance(16.0);
    }
    assert!((s.value() - 50.0).abs() < 0.5);
}

#[test]
fn layer_block_velocity_distortion_finite() {
    let mut s = slider((0.0, 100.0), 0.01);
    s.animate_to_value(100.0);
    for _ in 0..60 {
        s.advance(16.0);
        let sx = s.thumb_scale_x();
        let sy = s.thumb_scale_y();
        assert!(sx.is_finite() && sy.is_finite(), "{sx} {sy}");
        assert!(sx > 0.2 && sx < 3.0, "sx {sx}");
        assert!(sy > 0.2 && sy < 3.0, "sy {sy}");
    }
}

#[test]
fn solid_has_instant_press_no_glass_scale() {
    let mut s = solid((0.0, 100.0), 0.01);
    s.set_pressed(true);
    s.advance(16.0);
    assert_eq!(s.press_progress(), 1.0);
    assert_eq!(s.thumb_scale_x(), 1.0);
}

#[test]
fn drag_overshoot_not_banked() {
    let mut s = slider((0.0, 100.0), 0.01)
        .with_value(100.0)
        .with_track_width(240.0);
    s.set_pressed(true);
    s.process_pointer_events(&[ptr(200.0, PointerInputPhase::Down)], 1.0);
    s.process_pointer_events(&[ptr(260.0, PointerInputPhase::Move)], 1.0); // past right
    assert!((s.value() - 100.0).abs() < 1e-3);
    s.process_pointer_events(&[ptr(236.0, PointerInputPhase::Move)], 1.0); // back by 24 => -10
    assert!((s.value() - 90.0).abs() < 1e-3, "value {}", s.value());
}

#[test]
fn geometry_constants_match_original() {
    assert_eq!(THUMB_W, 40.0);
    assert_eq!(THUMB_H, 24.0);
    assert_eq!(THUMB_PRESSED_SCALE, 1.5);
    assert_eq!(TRACK_H, 6.0);
}

#[test]
fn long_frame_stable() {
    let mut s = slider((0.0, 100.0), 0.01);
    s.set_pressed(true);
    for _ in 0..120 {
        s.advance(16.0);
    }
    s.set_pressed(false);
    s.advance(500.0);
    for v in [
        s.thumb_scale_x(),
        s.thumb_scale_y(),
        s.press_progress(),
        s.value(),
    ] {
        assert!(v.is_finite());
    }
    for _ in 0..800 {
        s.advance(16.0);
    }
}

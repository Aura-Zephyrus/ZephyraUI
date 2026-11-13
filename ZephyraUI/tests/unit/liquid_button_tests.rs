//! `LiquidButton`（LiquidButton + InteractiveHighlight 原版语义）的门禁。

use zephyra_core::filter::Material;
use zephyra_fx::{PointerInput, PointerInputPhase, PointerSource};

use super::{LiquidButton, LIQUID_BUTTON_H, LIQUID_BUTTON_W};

fn ptr(x: f32, y: f32, phase: PointerInputPhase) -> PointerInput {
    PointerInput {
        id: 0,
        x,
        y,
        phase,
        source: PointerSource::Mouse,
    }
}

#[test]
fn constants_match_original() {
    assert_eq!(LIQUID_BUTTON_H, 48.0);
}

#[test]
fn press_spring_animates_and_converges_glass() {
    let mut b = LiquidButton::new("b").with_material(Material::Glass);
    b.set_pressed(true);
    for _ in 0..120 {
        b.advance(16.0);
    }
    assert!(
        (b.press_progress() - 1.0).abs() < 0.02,
        "{}",
        b.press_progress()
    );
    b.set_pressed(false);
    for _ in 0..400 {
        b.advance(16.0);
    }
    assert!(b.press_progress().abs() < 0.02, "{}", b.press_progress());
    assert!(!b.is_animating());
}

#[test]
fn solid_is_instant() {
    let mut b = LiquidButton::new("b");
    b.set_pressed(true);
    b.advance(16.0);
    assert_eq!(b.press_progress(), 1.0);
    b.set_pressed(false);
    b.advance(16.0);
    assert_eq!(b.press_progress(), 0.0);
    assert!(!b.is_animating());
}

#[test]
fn drag_offset_drives_translation_tanh() {
    let mut b = LiquidButton::new("b")
        .with_material(Material::Glass)
        .with_size(120.0, 48.0);
    b.set_pressed(true);
    b.process_pointer_events(&[ptr(60.0, 24.0, PointerInputPhase::Down)], 1.0);
    b.process_pointer_events(&[ptr(80.0, 30.0, PointerInputPhase::Move)], 1.0);
    let (tx, ty) = b.translation();
    assert!(tx > 0.0 && tx.is_finite(), "{tx}");
    assert!(ty > 0.0 && ty.is_finite(), "{ty}");
    let max_offset = 48.0f32;
    assert!(tx.abs() <= max_offset && ty.abs() <= max_offset);
}

#[test]
fn release_returns_offset_to_zero() {
    let mut b = LiquidButton::new("b")
        .with_material(Material::Glass)
        .with_size(120.0, 48.0);
    b.set_pressed(true);
    b.process_pointer_events(&[ptr(60.0, 24.0, PointerInputPhase::Down)], 1.0);
    b.process_pointer_events(&[ptr(90.0, 40.0, PointerInputPhase::Move)], 1.0);
    b.set_pressed(false);
    b.process_pointer_events(&[ptr(90.0, 40.0, PointerInputPhase::Up)], 1.0);
    for _ in 0..600 {
        b.advance(16.0);
    }
    let (ox, oy) = b.offset();
    assert!(ox.abs() < 0.05 && oy.abs() < 0.05, "{ox} {oy}");
    let (tx, ty) = b.translation();
    assert!(tx.abs() < 0.1 && ty.abs() < 0.1, "{tx} {ty}");
}

#[test]
fn layer_block_scale_changes_on_press_glass() {
    let mut b = LiquidButton::new("b")
        .with_material(Material::Glass)
        .with_size(LIQUID_BUTTON_W, LIQUID_BUTTON_H);
    let (sx0, sy0) = b.scale();
    assert!((sx0 - 1.0).abs() < 1e-6 && (sy0 - 1.0).abs() < 1e-6);
    b.set_pressed(true);
    for _ in 0..120 {
        b.advance(16.0);
    }
    let (sx, sy) = b.scale();
    // baseScale = 1 + 4/48 * progress ≈ 1.0833
    assert!(sx > 1.05 && sx < 1.2, "{sx}");
    assert!(sy > 1.05 && sy < 1.2, "{sy}");
}

#[test]
fn directional_scale_responds_to_drag() {
    let mut b = LiquidButton::new("b")
        .with_material(Material::Glass)
        .with_size(120.0, 48.0);
    b.set_pressed(true);
    b.process_pointer_events(&[ptr(60.0, 24.0, PointerInputPhase::Down)], 1.0);
    for _ in 0..60 {
        b.advance(16.0);
    }
    b.process_pointer_events(&[ptr(100.0, 24.0, PointerInputPhase::Move)], 1.0);
    let (sx, _sy) = b.scale();
    assert!(sx.is_finite() && sx > 1.0);
}

#[test]
fn long_frame_stable() {
    let mut b = LiquidButton::new("b")
        .with_material(Material::Glass)
        .with_size(120.0, 48.0);
    b.set_pressed(true);
    b.process_pointer_events(&[ptr(60.0, 24.0, PointerInputPhase::Down)], 1.0);
    b.process_pointer_events(&[ptr(90.0, 34.0, PointerInputPhase::Move)], 1.0);
    b.advance(500.0);
    for v in [
        b.press_progress(),
        b.translation().0,
        b.translation().1,
        b.scale().0,
        b.scale().1,
    ] {
        assert!(v.is_finite(), "{v}");
    }
}

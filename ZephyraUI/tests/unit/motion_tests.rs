//! 下拉开合补间的单测（算法住在控件层，不由每个应用各写一份）。

use super::{
    morph_class, morph_height, reveal_items, visible, DropdownMotion, CLOSE_MS, MORPH_CLASSES,
    MORPH_STEPS, OPEN_MS,
};

#[test]
fn defaults_are_collapsed_and_quiet() {
    let m = DropdownMotion::default();
    assert_eq!(m.value(), 0.0);
    assert!(!m.is_animating());
    assert!(!visible(m.value()));
}

#[test]
fn opening_reaches_one_and_stops_asking_for_frames() {
    let mut m = DropdownMotion::default();
    for _ in 0..40 {
        m.tick(true, 16.0);
    }
    assert_eq!(m.value(), 1.0);
    assert!(!m.is_animating(), "到端点后不该让宿主继续空转");
}

// 时长是字面量，编译期就能判，不必等到测试期。
const _: () = assert!(CLOSE_MS < OPEN_MS);

#[test]
fn interrupted_flip_restarts_from_current_value() {
    let mut m = DropdownMotion::default();
    for _ in 0..6 {
        m.tick(true, 16.0);
    }
    let mid = m.value();
    assert!(mid > 0.0 && mid < 1.0);
    m.tick(false, 16.0);
    assert!(
        m.value() <= mid,
        "反向起跑必须从当前值继续，不能先跳回 0 再降"
    );
}

#[test]
fn non_finite_or_nonpositive_dt_does_not_advance() {
    let mut m = DropdownMotion::default();
    m.tick(true, f32::NAN);
    assert_eq!(m.value(), 0.0);
    m.tick(true, 0.0);
    assert_eq!(m.value(), 0.0);
    m.tick(true, -16.0);
    assert_eq!(m.value(), 0.0);
}

#[test]
fn morph_steps_cover_the_full_range() {
    assert_eq!(MORPH_CLASSES.len(), MORPH_STEPS as usize);
    assert_eq!(morph_class(0.0), "");
    assert_eq!(morph_class(1.0), MORPH_CLASSES[MORPH_STEPS as usize - 1]);
    assert_eq!(morph_height(0.0), super::BTN_H);
    assert_eq!(morph_height(1.0), super::MENU_H);
}

#[test]
fn reveal_is_symmetric_and_after_the_threshold() {
    assert!(!reveal_items(0.5, true));
    assert!(reveal_items(0.6, true));
    assert!(!reveal_items(0.5, false));
    assert!(reveal_items(0.6, false));
}

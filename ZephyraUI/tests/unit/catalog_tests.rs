//! 补充控件的纯状态单测（无窗口、无 GPU）。

use crate::{
    AvatarState, BusyIndicatorState, CardState, ColorPickerState, ExpanderState, HyperlinkState,
    InfoBadgeState, InfoBarState, PageIndicatorState, Progress, SelectorBarState,
    SettingExpanderState, SettingItemState, SpinBoxState, ToggleState, ToolButtonState,
    ToolTipState,
};

#[test]
fn progress_clamps_and_reports_percent() {
    let mut p = Progress::new(1.5);
    assert_eq!(p.value01(), 1.0);
    p.set_value(-1.0);
    assert_eq!(p.value01(), 0.0);
    p.set_value(0.456);
    assert_eq!(p.percent(), 46);
    p.set_state("error");
    assert_eq!(p.state_word(), "error");
}

#[test]
fn info_bar_close_is_idempotent_and_reopenable() {
    let mut b = InfoBarState::new("caution");
    assert!(b.is_open());
    b.close();
    b.close();
    assert!(!b.is_open());
    b.reopen();
    assert!(b.is_open());
    assert_eq!(b.severity(), "caution");
}

#[test]
fn color_picker_select_ignores_out_of_range() {
    let mut c = ColorPickerState::new(99);
    assert_eq!(c.selected(), 15);
    c.select(7);
    assert_eq!(c.selected(), 7);
    c.select(16);
    assert_eq!(c.selected(), 7);
}

#[test]
fn spin_box_stops_at_bounds() {
    let mut s = SpinBoxState::new(5, 1, 10);
    for _ in 0..20 {
        s.increment();
    }
    assert_eq!(s.value(), 10);
    for _ in 0..20 {
        s.decrement();
    }
    assert_eq!(s.value(), 1);
}

#[test]
fn busy_indicator_advances_one_arc_frame_per_step() {
    use crate::catalog::{BUSY_FRAME_COUNT, BUSY_FRAME_MS};
    let mut b = BusyIndicatorState::new();
    assert_eq!(b.icon_name(), "busy-arc-0");
    b.advance(BUSY_FRAME_MS - 1.0);
    assert_eq!(b.frame(), 0);
    b.advance(1.0);
    assert_eq!(b.frame(), 1);
    assert_eq!(b.icon_name(), "busy-arc-1");
    // 走满一整圈回到第 0 帧（从帧 1 再走 23 帧 = 24 帧，整圈回绕）。
    let mut full = BusyIndicatorState::new();
    // +1ms 抵消 f32 累加误差（否则 24 帧只走 23 帧）。
    full.advance(BUSY_FRAME_MS * BUSY_FRAME_COUNT as f32 + 1.0);
    assert_eq!(full.frame(), 0);
    assert!(b.is_animating());
}

#[test]
fn expander_toggles_both_ways() {
    let mut e = ExpanderState::new(false);
    e.toggle();
    assert!(e.is_expanded());
    e.toggle();
    assert!(!e.is_expanded());
}

#[test]
fn selector_bar_clamps_and_yields_id() {
    let mut s = SelectorBarState::new(9, 3);
    assert_eq!(s.selected(), 2);
    assert_eq!(s.selected_id(), "sel:2");
    s.select(1);
    assert_eq!(s.selected_id(), "sel:1");
    s.select(5);
    assert_eq!(s.selected(), 1);
}

#[test]
fn dynamic_arc_annulus_triangulates_to_vertices() {
    use zephyra_core::core::Point;
    use zephyra_core::geom::{triangulate, Contour, FillRule};
    // 与 gallery 的 catalog_busy_path 同一套几何：中线半径 25、线宽 6、圆端。
    let start = 45.0_f32.to_radians();
    let sweep = 120.0_f32.to_radians();
    let steps = 20usize;
    let (ro, ri, cap) = (28.0f32, 22.0f32, 3.0f32);
    let mut ring: Vec<Point> = Vec::new();
    for i in 0..=steps {
        let a = start + sweep * (i as f32 / steps as f32);
        ring.push(Point::new(28.0 + ro * a.cos(), 28.0 + ro * a.sin()));
    }
    let end = start + sweep;
    let ec = Point::new(28.0 + 25.0 * end.cos(), 28.0 + 25.0 * end.sin());
    for i in 1..8 {
        let t = end + std::f32::consts::PI * i as f32 / 8.0;
        ring.push(Point::new(ec.x + cap * t.cos(), ec.y + cap * t.sin()));
    }
    for i in (0..=steps).rev() {
        let a = start + sweep * (i as f32 / steps as f32);
        ring.push(Point::new(28.0 + ri * a.cos(), 28.0 + ri * a.sin()));
    }
    let sc = Point::new(28.0 + 25.0 * start.cos(), 28.0 + 25.0 * start.sin());
    for i in 1..8 {
        let t = start + std::f32::consts::PI + std::f32::consts::PI * i as f32 / 8.0;
        ring.push(Point::new(sc.x + cap * t.cos(), sc.y + cap * t.sin()));
    }
    let mesh = triangulate(&[Contour::new(ring)], FillRule::NonZero).expect("弧线应能三角化");
    assert!(
        mesh.vertices.len() > 6,
        "顶点数 {} 太少",
        mesh.vertices.len()
    );
}

#[test]
fn avatar_carries_initials_and_accent() {
    let a = AvatarState::new("ZL", true);
    assert_eq!(a.initials(), "ZL");
    assert!(a.is_accent());
}

#[test]
fn toggle_states_flip_both_ways() {
    let mut t = ToggleState::new(true);
    assert_eq!(t.class_word(), "checked");
    t.toggle();
    assert!(!t.is_checked());
    assert_eq!(t.class_word(), "");
    let mut p = ToggleState::new(false);
    p.toggle();
    assert!(p.is_checked());
}

#[test]
fn tool_button_and_link_record_interaction() {
    let mut tool = ToolButtonState::new(false);
    tool.toggle();
    assert!(tool.is_highlighted());
    let mut link = HyperlinkState::new();
    assert!(!link.is_visited());
    link.visit();
    assert!(link.is_visited());
}

#[test]
fn info_badge_formats_count_and_severity() {
    let mut b = InfoBadgeState::new(5, "success", true);
    assert_eq!(b.text(), "5");
    b.set_count(100);
    assert_eq!(b.text(), "99+");
    b.set_count(-1);
    assert_eq!(b.text(), "");
    assert_eq!(b.severity(), "success");
    assert!(b.is_solid());
}

#[test]
fn tooltip_setting_card_and_card_toggle() {
    let mut tip = ToolTipState::new(false);
    tip.toggle();
    assert!(tip.is_visible());
    let mut setting = SettingItemState::new(false);
    setting.toggle();
    assert!(setting.is_clickable());
    let mut exp = SettingExpanderState::new(false);
    exp.toggle();
    assert!(exp.is_expanded());
    let mut card = CardState::new(false);
    card.toggle();
    assert!(card.is_selected());
}

#[test]
fn page_indicator_wraps_and_clamps() {
    let mut p = PageIndicatorState::new(3, 0);
    p.next();
    p.next();
    p.next();
    assert_eq!(p.current(), 0, "到末页后应回到第 0 页");
    p.set_current(9);
    assert_eq!(p.current(), 0, "越界设置应被忽略");
    p.set_current(2);
    assert_eq!(p.current(), 2);
}

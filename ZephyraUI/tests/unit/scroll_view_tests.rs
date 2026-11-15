//! `ScrollView` 的行为断言：钳位、滚轮、拖动（含 slop）。
//!
//! 这批断言盯的是**滚动容器的数值契约** —— 画面上的效果由 core 的绘制趟
//! 保证（见 `zephyra-core/tests/unit/paint_tests.rs`），而"能不能滚到那一头"
//! 完全由这里的钳位决定。

use zephyra_core::core::Point;

use super::{ScrollView, SCROLL_DRAG_SLOP, SCROLL_WHEEL_DELTA};

/// 一个内容 300×300、视口 100×100 的滚动容器。
fn view() -> ScrollView {
    let mut v = ScrollView::new("scroll");
    v.set_content_size(Point::new(300.0, 300.0));
    v.set_viewport_size(Point::new(100.0, 100.0));
    v
}

#[test]
fn the_scroll_range_is_content_minus_viewport() {
    let v = view();
    assert!((v.max_scroll_y() - 200.0).abs() < 1e-6);
    assert!((v.max_scroll_x() - 200.0).abs() < 1e-6);
    assert!(v.can_scroll_y() && v.can_scroll_x());
}

#[test]
fn content_that_fits_does_not_scroll() {
    let mut v = ScrollView::new("scroll");
    v.set_content_size(Point::new(50.0, 50.0));
    v.set_viewport_size(Point::new(100.0, 100.0));
    assert!(!v.can_scroll_y());
    assert_eq!(v.max_scroll_y(), 0.0);
}

#[test]
fn the_offset_is_clamped_to_the_scroll_range() {
    let mut v = view();
    v.set_scroll_offset(Point::new(999.0, -50.0));
    assert_eq!(v.scroll_offset().x, 200.0);
    assert_eq!(v.scroll_offset().y, 0.0);
}

#[test]
fn the_wheel_scrolls_by_a_fixed_step_and_stops_at_the_ends() {
    let mut v = view();
    assert!(v.scroll_wheel(1.0));
    assert!((v.scroll_offset().y - SCROLL_WHEEL_DELTA).abs() < 1e-6);
    // 滚到底：不再变化。
    for _ in 0..100 {
        v.scroll_wheel(1.0);
    }
    assert!((v.scroll_offset().y - 200.0).abs() < 1e-6);
    assert!(!v.scroll_wheel(1.0), "已经在底了就不该再报变化");
    // 往上滚回到 0。
    for _ in 0..100 {
        v.scroll_wheel(-1.0);
    }
    assert!(v.scroll_offset().y.abs() < 1e-6);
}

#[test]
fn a_disabled_scroll_view_ignores_the_wheel() {
    let mut v = view();
    v.set_disabled(true);
    assert!(!v.scroll_wheel(1.0));
    assert_eq!(v.scroll_offset().y, 0.0);
}

#[test]
fn a_drag_below_the_slop_does_not_move_the_content() {
    let mut v = view();
    v.drag_start(Point::new(50.0, 50.0));
    // 抖动 2 DIP —— 不到 slop。
    assert!(!v.drag_update(Point::new(52.0, 52.0)));
    assert_eq!(v.scroll_offset().x, 0.0);
    assert_eq!(v.scroll_offset().y, 0.0);
}

#[test]
fn a_drag_past_the_slop_moves_the_content_by_the_over_slop_amount() {
    let mut v = view();
    v.drag_start(Point::new(50.0, 50.0));
    // 越过 slop 的**那一帧**只应用超出部分：手指左移 10，净位移 10 - 4 = 6。
    // 手指向左 ⇒ 内容向左 ⇒ 偏移**增大**。
    assert!(v.drag_update(Point::new(40.0, 50.0)));
    assert!(
        (v.scroll_offset().x - 6.0).abs() < 1e-6,
        "应当只应用超出 slop 的部分，实得 {}",
        v.scroll_offset().x
    );
    // 之后按增量累加。
    assert!(v.drag_update(Point::new(35.0, 50.0)));
    assert!(
        (v.scroll_offset().x - 11.0).abs() < 1e-6,
        "越过之后按增量累加，实得 {}",
        v.scroll_offset().x
    );
}

#[test]
fn dragging_down_moves_the_content_up() {
    let mut v = view();
    v.set_scroll_offset(Point::new(0.0, 100.0));
    v.drag_start(Point::new(50.0, 50.0));
    v.drag_update(Point::new(50.0, 50.0 + SCROLL_DRAG_SLOP + 10.0));
    // 手指向下 ⇒ 内容向上 ⇒ 偏移**减小**。
    assert!(
        (v.scroll_offset().y - 90.0).abs() < 1e-6,
        "实得 {}",
        v.scroll_offset().y
    );
}

#[test]
fn a_drag_is_clamped_at_the_ends() {
    let mut v = view();
    v.set_scroll_offset(Point::new(0.0, 100.0));
    v.drag_start(Point::new(50.0, 50.0));
    // 手指一路向上 ⇒ 偏移撞到上限 200。
    v.drag_update(Point::new(50.0, -5000.0));
    assert!((v.scroll_offset().y - 200.0).abs() < 1e-6);
    // 一路向下 ⇒ 撞到 0。
    v.drag_update(Point::new(50.0, 5000.0));
    assert!(v.scroll_offset().y.abs() < 1e-6);
}

#[test]
fn the_drag_axes_are_independent() {
    let mut v = view();
    v.drag_start(Point::new(50.0, 50.0));
    // 只在 X 上动 —— Y 轴还没越过 slop，不该动。
    v.drag_update(Point::new(50.0 - SCROLL_DRAG_SLOP - 20.0, 50.0));
    assert!(v.scroll_offset().x > 0.0);
    assert!(
        v.scroll_offset().y.abs() < 1e-6,
        "Y 轴没越过 slop 就不该动，实得 {}",
        v.scroll_offset().y
    );
}

#[test]
fn ending_the_drag_resets_the_slop_state() {
    let mut v = view();
    v.drag_start(Point::new(50.0, 50.0));
    v.drag_update(Point::new(80.0, 50.0));
    v.drag_end();
    assert!(!v.is_dragging());
    // 再拖一次：小幅移动不该立刻动（slop 重新生效）。
    v.drag_start(Point::new(50.0, 50.0));
    assert!(!v.drag_update(Point::new(52.0, 50.0)));
    assert!(v.scroll_offset().x.abs() < 1e-6);
}

#[test]
fn shrinking_the_content_pulls_the_offset_back_into_range() {
    let mut v = view();
    v.set_scroll_offset(Point::new(0.0, 200.0));
    // 内容变短了 —— 偏移必须跟着退回来，否则内容会停在空白处。
    v.set_content_size(Point::new(100.0, 100.0));
    assert!(v.scroll_offset().y.abs() < 1e-6);
}

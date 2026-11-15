//! `ScrollBar` 的行为断言：拇指几何、拖动换算、翻页。
//!
//! 拇指长度与位置是**唯一**能让用户看出"还有多少内容、滚到哪了"的东西，
//! 所以这两条几何关系必须精确；而拖动换算是滚动条与滚动容器之间的**契约**
//: 错一位就会「拇指跟着手走，内容却不动」。

use super::{ScrollBar, ScrollBarAxis, MIN_THUMB_LENGTH, PAGE_FRACTION};

/// 轨道 200、内容 500、视口 100 的竖直滚动条。
fn bar() -> ScrollBar {
    let mut b = ScrollBar::new("bar");
    b.set_axis(ScrollBarAxis::Vertical);
    b.set_geometry(200.0, 500.0, 100.0);
    b
}

#[test]
fn the_thumb_length_is_proportional_to_the_viewport() {
    let b = bar();
    // 视口占内容的 1/5 ⇒ 拇指占轨道的 1/5 = 40。
    assert!(
        (b.thumb_length() - 40.0).abs() < 1e-4,
        "实得 {}",
        b.thumb_length()
    );
}

#[test]
fn a_tiny_viewport_still_gets_a_grabbable_thumb() {
    let mut b = ScrollBar::new("bar");
    // 内容 10000、视口 10 ⇒ 按比例只有 0.2 DIP，点不中。
    b.set_geometry(200.0, 10000.0, 10.0);
    assert!(
        b.thumb_length() >= MIN_THUMB_LENGTH,
        "拇指不该细到点不中，实得 {}",
        b.thumb_length()
    );
}

#[test]
fn the_thumb_position_tracks_the_offset() {
    let mut b = bar();
    b.set_offset(0.0);
    assert!(b.thumb_offset().abs() < 1e-4);
    // 偏移 200（上限 400）⇒ 拇指走了一半行程（200-40=160）的一半 = 80。
    b.set_offset(200.0);
    assert!(
        (b.thumb_offset() - 80.0).abs() < 1e-3,
        "实得 {}",
        b.thumb_offset()
    );
    // 到底 ⇒ 拇指贴到轨道末端。
    b.set_offset(400.0);
    assert!(
        (b.thumb_end() - 200.0).abs() < 1e-3,
        "到底时拇指应当贴着轨道末端，实得 {}",
        b.thumb_end()
    );
}

#[test]
fn content_that_fits_gives_a_full_length_thumb_and_no_scrolling() {
    let mut b = ScrollBar::new("bar");
    b.set_geometry(200.0, 100.0, 200.0);
    assert!(!b.is_scrollable());
    assert!((b.thumb_length() - 200.0).abs() < 1e-4);
    assert!(b.thumb_offset().abs() < 1e-4);
}

#[test]
fn dragging_the_thumb_scrolls_by_the_ratio_of_the_two_travels() {
    let mut b = bar();
    b.set_offset(0.0);
    b.drag_start(0.0);
    // 拇指的行程 = 200 - 40 = 160；内容的行程 = 500 - 100 = 400。
    // 指针走 40 ⇒ 内容走 40 * 400/160 = 100。
    let off = b.drag_update(40.0);
    assert!(
        (off - 100.0).abs() < 1e-3,
        "实得 {off} —— 拇指与内容的行程比例算错了"
    );
    assert!(b.is_dragging());
}

#[test]
fn a_drag_is_clamped_to_the_content_range() {
    let mut b = bar();
    b.drag_start(0.0);
    let off = b.drag_update(10_000.0);
    assert!((off - 400.0).abs() < 1e-3, "实得 {off}");
    b.drag_update(-10_000.0);
    assert!(b.offset().abs() < 1e-3);
}

#[test]
fn the_thumb_hit_test_matches_its_geometry() {
    let mut b = bar();
    b.set_offset(0.0);
    assert!(b.hits_thumb(10.0));
    assert!(!b.hits_thumb(100.0));
    b.set_offset(400.0);
    // 到底时拇指在 160..200。
    assert!(!b.hits_thumb(10.0));
    assert!(b.hits_thumb(180.0));
}

#[test]
fn paging_moves_by_ninety_percent_of_the_viewport() {
    let mut b = bar();
    b.set_offset(0.0);
    let off = b.page(true);
    assert!((off - 100.0 * PAGE_FRACTION).abs() < 1e-3, "实得 {off}");
    let off = b.page(false);
    assert!(off.abs() < 1e-3, "实得 {off}");
}

#[test]
fn a_vertical_bar_ignores_the_horizontal_wheel_delta() {
    let mut b = bar();
    b.set_offset(0.0);
    // 横向滚轮不该带动竖直滚动条。
    let off = b.wheel(1.0, 0.0);
    assert!(off.abs() < 1e-3, "实得 {off}");
    let off = b.wheel(0.0, 1.0);
    assert!(off > 0.0, "纵向滚轮应当翻页");
}

#[test]
fn a_horizontal_bar_uses_the_horizontal_delta() {
    let mut b = ScrollBar::new("bar");
    b.set_axis(ScrollBarAxis::Horizontal);
    b.set_geometry(200.0, 500.0, 100.0);
    b.set_offset(0.0);
    let off = b.wheel(0.0, 1.0);
    assert!(off.abs() < 1e-3, "实得 {off} —— 横向条不该吃纵向滚轮");
    let off = b.wheel(1.0, 0.0);
    assert!(off > 0.0);
}

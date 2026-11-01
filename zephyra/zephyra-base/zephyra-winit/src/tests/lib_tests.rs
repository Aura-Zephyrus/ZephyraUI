// 本文件由 lib.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（lib.rs 用 #[path] 指过来），所以 `use super::*` 与私有可见性照旧。

use super::*;

/// 透明窗口必须清成 alpha=0，否则「窗口透明」合出来仍是一块实色底。
#[test]
fn transparent_window_clears_with_zero_alpha() {
    let surface = Theme::dark().surface;
    assert!(surface.a > 0.0, "前提：主题底色本身是不透明的");

    let clear = clear_color(true, surface);

    assert_eq!(clear.a, 0.0, "透明窗口的清屏色必须全透明");
    // RGB 允许保留（premultiplied 换算在渲染器里做，alpha=0 时乘出来就是 0）。
    assert_eq!(clear.r, surface.r);
    assert_eq!(clear.g, surface.g);
    assert_eq!(clear.b, surface.b);
}

/// 不透明窗口一个字节都不能变 —— 这条改动不允许顺手改掉既有画面。
#[test]
fn opaque_window_keeps_the_theme_surface_verbatim() {
    let surface = Theme::dark().surface;
    assert_eq!(clear_color(false, surface), surface);
}

/// 已经透明的底色再过一遍不变（幂等）：重画帧会重复走这条路径。
#[test]
fn clear_color_is_idempotent_for_an_already_transparent_surface() {
    let surface = Theme::dark().surface;
    let once = clear_color(true, surface);
    assert_eq!(clear_color(true, once), once);
}

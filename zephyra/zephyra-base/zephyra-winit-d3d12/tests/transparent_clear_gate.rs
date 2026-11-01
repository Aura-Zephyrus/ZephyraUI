//! 窗口透明模式的清屏判据：**透明 ⇒ alpha=0**。
//!
//! # 为什么这个测试一个 GPU API 都不碰
//!
//! 这条判据的失效**只能在真窗口下看见**：不透明的 clear 会让「透明窗口」合出来
//! 仍是一块实色底，而 `--dump-cpu` 不呈现窗口、`RenderReport` 里一切正常 ——
//! 症状像极了「应用根背景写死了」，隔着一层排查。
//!
//! 所以判据抽成 `clear_color(bool, Color) -> Color` 这个纯函数（见 `host.rs`），
//! 在没有设备的环境里断言它；本文件与 `device_lost_gate.rs` 同一手法。
//!
//! D3D11 后端有一份同判据的独立实现与独立门禁
//! （`zephyra-winit/src/tests/lib_tests.rs`）——两个后端互不依赖是有意的，
//! 代价就是这条判据写两遍、必须两边都钉住。

use zephyra_core::core::Color;
use zephyra_winit_d3d12::clear_color;

/// 主题底色：不透明的深灰，接近真实 `Theme::dark().surface` 的形态。
const SURFACE: Color = Color::rgba(0.12, 0.12, 0.13, 1.0);

#[test]
fn transparent_window_clears_with_zero_alpha() {
    let clear = clear_color(true, SURFACE);

    assert_eq!(clear.a, 0.0, "透明窗口的清屏色必须全透明");
    // RGB 保留即可：premultiplied 换算在渲染器里做，alpha=0 乘出来就是 0。
    assert_eq!(
        (clear.r, clear.g, clear.b),
        (SURFACE.r, SURFACE.g, SURFACE.b)
    );
}

#[test]
fn opaque_window_keeps_the_theme_surface_verbatim() {
    // 不透明窗口一个字节都不能变 —— 这条改动不允许顺手改掉既有画面。
    assert_eq!(clear_color(false, SURFACE), SURFACE);
}

#[test]
fn clear_color_is_idempotent_for_an_already_transparent_surface() {
    // 重画帧会重复走这条路径（首帧、resize、设备恢复都各自清一次）。
    let once = clear_color(true, SURFACE);
    assert_eq!(clear_color(true, once), once);
}

/// 已经是半透明（而非全透明）的底色在透明模式下必须被**压到 0**，不是保留。
///
/// 这条钉的是「透明模式不是半透明旋钮」：窗口透明要求的是「没画到的地方全透」，
/// 保留 0.85 之类的 alpha 会让未绘制区域变成一层灰纱 —— 画面上很像「颜色错了」。
#[test]
fn transparent_mode_forces_zero_not_a_kept_partial_alpha() {
    let translucent = Color::rgba(0.2, 0.2, 0.2, 0.85);
    assert_eq!(clear_color(true, translucent).a, 0.0);
}

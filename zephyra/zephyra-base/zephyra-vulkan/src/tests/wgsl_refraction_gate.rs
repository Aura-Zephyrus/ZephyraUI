//! WGSL 折射入口的字符串级门禁。
//!
//! # 为什么需要字符串门禁
//!
//! `fs_refraction` 与 HLSL 的 `PSRefraction` 是**两份手抄**（对应关系写在 WGSL
//! 文件头），像素级对照要真 GPU（WSL2 上的 gallery），不能每次提交都跑。手抄
//! 最危险的不是写错，而是**改着改着退回到旧版**—— 旧版四个坑（宽遮罩、薄棱镜
//! 近似、bezel 按圆角半径夹、漏色散）每一个都「画面合法、不报错」，只有下面
//! 这些断言能在提交前拦住。
//!
//! 每条断言钉一个已实测过的失效模式，出处见 `docs/dev-log/` 的折射三条日志。

/// 与构建期同一份源（都来自 `zephyra_vector::shaders::wgsl_source()`）。
fn wgsl() -> String {
    zephyra_fx::shaders::wgsl_source()
}

/// 入口必须存在 —— 漏了它 `create_entry_pipeline` 要到运行期建管线时才报错，
/// 而那时的症状是「启动即失败」，与「管线建错」无法区分。
#[test]
fn fs_refraction_entry_exists() {
    assert!(
        wgsl().contains("fn fs_refraction(i: VsOut)"),
        "WGSL 里没有 fs_refraction 入口 —— 折射在 Linux 侧会整个消失"
    );
}

/// 遮罩必须是 1px 抗锯齿带。
///
/// 钉的是「宽遮罩把最强折射乘成 0」那个实测差 6 倍的问题（10-22 日志 2.3 节）：
/// `min(half_size) * 0.5` 那一版在边缘正好是 0。
#[test]
fn refraction_mask_is_a_one_pixel_band() {
    assert!(
        wgsl().contains("let edge = 1.0;"),
        "折射遮罩的 edge 不再是 1px —— 宽遮罩会把边缘处最强的折射乘成 0（实测差 6 倍）"
    );
}

/// 倒角带**不按圆角半径夹**。
///
/// 钉的是「倒角参数上 3/4 档变空档」那个问题（10-22 日志 2.4 节）：按
/// `min(bezel, radius)` 夹时，12px 圆角的玻璃块上 26/34/48px 三档全没反应。
/// liquid-glass 网页 demo 的 WebGL 版正是被半径夹的那一版。
#[test]
fn bezel_is_not_clamped_by_the_corner_radius() {
    assert!(
        wgsl().contains("let bezel = min(max(i.color.g, 1.0), min(i.half_size.x, i.half_size.y));"),
        "折射 bezel 的夹法变了 —— 只能按半边长夹，按圆角半径夹会让倒角参数的上 3/4 变空档"
    );
}

/// 位移必须是有界的楔形偏折式。
///
/// 钉的是「薄棱镜近似在大角度下发散」那个问题（10-22 日志 2.2 节）：旧式
/// `h * (tan(Θs) - tan(Θr))` 实测位移峰值 491px，而契约标称值（厚度）是 28px。
#[test]
fn displacement_is_the_bounded_wedge_form() {
    assert!(
        wgsl().contains("sin(slope - theta_r) / max(cos(theta_r), 1e-3)"),
        "位移公式不是有界楔形偏折式 —— 薄棱镜近似会在 t→0（Θs→90°）处发散（实测 491px vs 28px）"
    );
    assert!(
        !wgsl().contains("tan(slope) - tan(theta_r)"),
        "位移公式里出现了薄棱镜近似 —— 它在大角度下发散，实测位移是厚度的 17~27 倍"
    );
}

/// 色散必须把 R 与 B 往**相反**方向拉。
///
/// 钉的是色散生效的判据（10 月折射日志的色散门禁）：只拉一个通道时红蓝交替
/// 背景上与无色散逐像素相同，看着像「色散没接上」。
#[test]
fn dispersion_offsets_red_and_blue_in_opposite_directions() {
    assert!(
        wgsl().contains("off_uv * (1.0 + dispersion)")
            && wgsl().contains("off_uv * (1.0 - dispersion)"),
        "色散的两个方向系数不完整 —— R/B 必须沿法线反向拉开，否则色散永不生效"
    );
}

/// 背景模糊入口必须存在，且采样的是 `g_dst`（背后）而不是 `g_tex`（元素自己）。
///
/// 与 `fs_refraction` 同一条手抄纪律：漏了入口，`fs_backdrop_blur` 在 Linux 侧
/// 整个消失；采样错了纹理，磨砂糊的是元素自己而不是背后，症状是「磨砂像普通模糊」。
#[test]
fn fs_backdrop_blur_exists_and_samples_the_backdrop() {
    assert!(
        wgsl().contains("fn fs_backdrop_blur(i: VsOut)"),
        "WGSL 里没有 fs_backdrop_blur 入口 —— 磨砂在 Linux 侧会整个消失"
    );
    // 在这个入口的源码段里，采样必须是 g_dst（元素背后），不是 g_tex（元素自己）。
    let src = wgsl();
    let start = src
        .find("fn fs_backdrop_blur(i: VsOut)")
        .expect("上面刚断言存在");
    let body = &src[start..];
    let end = body.find("\n}").map(|e| e + 2).unwrap_or(body.len());
    let body = &body[..end];
    assert!(
        body.contains("textureSampleLevel(g_dst"),
        "fs_backdrop_blur 必须采样 g_dst（背后画面），否则磨砂糊的是元素自己"
    );
    assert!(
        body.contains("gauss_weight"),
        "fs_backdrop_blur 必须用高斯权重做卷积"
    );
    assert!(
        body.contains("let mask = saturate(-d / 1.0);"),
        "fs_backdrop_blur 的遮罩必须是 1px 抗锯齿带（与折射同一条纪律）"
    );
}

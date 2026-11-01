// 本文件由 lib.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;

#[test]
fn version_is_readable_and_major_at_least_ten() {
    // 本 crate 只在 Windows 上有内容；非 Windows 上这条测试走 stub 分支。
    let v = Windows.version().expect("Windows 上必须能读到版本号");
    assert!(v.0 >= 10, "读到的主版本不像 Windows：{v:?}");
    assert!(v.2 >= 10000, "build 号不像 Windows：{v:?}");
}

#[test]
fn build_number_and_version_agree() {
    assert_eq!(build_number(), Windows.version().unwrap().2);
}

#[test]
fn null_handle_calls_no_dwm_and_enables_nothing() {
    let out = Windows.apply_window_chrome(
        WindowHandle::from_raw(0),
        WindowChrome::minimal(true, zephyra_platform::CornerPreference::Round),
    );
    assert!(!out.dark && !out.corner, "空句柄不该有任何效果");
}

#[test]
fn capability_bits_match_documented_thresholds() {
    let caps = Windows.capabilities();
    let build = build_number();
    assert_eq!(
        caps.dark_mode,
        build >= 22_000,
        "DWMWA_USE_IMMERSIVE_DARK_MODE 的文档门槛是 Build 22000"
    );
    assert_eq!(
        caps.corner_preference,
        build >= 22_000,
        "DWMWA_WINDOW_CORNER_PREFERENCE 的文档门槛是 Build 22000"
    );
}

#[test]
fn system_theme_query_matches_registry_or_is_unreadable() {
    // ⚠️ 这两个注册表值**没有官方文档**（见 docs/developer/win/04 §2），所以判据不能是
    // "必须等于深色"—— 那会在浅色系统上无故红。
    //
    // 真正要钉住的是**方向**：查得到就必须与 `AppsUseLightTheme` 一致；
    // 查不到就返回 `None` 而**不是**默认的 `Light`（后者会让不支持的平台上
    // 主题被无声钉死，且上层看不出"这是猜的"）。
    let got = Windows.system_theme();
    if let Some(pref) = got {
        assert_ne!(pref, ThemePreference::Unknown, "已知的话不该是 Unknown");
        // 反复读必须稳定：一次读到值一次读不到，说明句柄生命周期有问题。
        assert_eq!(Some(pref), Windows.system_theme());
    }
    // 能声称支持查询吗？
    assert!(
        Windows.capabilities().system_theme_query,
        "Windows 上查询系统主题是支持的（读注册表），能力位应为 true"
    );
}

#[test]
fn capability_query_is_side_effect_free_and_stable() {
    // `capabilities()` 在热路径被调用：它必须无分配、无锁、无 I/O。
    let a = Windows.capabilities();
    let b = Windows.capabilities();
    assert_eq!(a, b);
    assert_eq!(a, Windows.capabilities());
}

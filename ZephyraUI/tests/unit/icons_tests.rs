//! 控件层图标集门禁：播放器图标确实登记在 UI 层，且内置字形没被挤掉。

use super::*;

#[test]
fn ui_layer_icons_are_registered_on_top_of_builtin() {
    let set = icon_set();
    let names = set.names();
    for name in ["play", "pause", "volume-up", "volume-off", "fullscreen"] {
        assert!(names.contains(&name), "UI 层缺少图标 {name} 的登记");
    }
    // 不能为了加自己的图标把框架内置表丢了 —— 既有控件还在用 gear / check。
    assert!(names.contains(&"gear"), "内置图标被 UI 层图标集挤掉了");
    assert!(names.len() > 20, "图标集太小，遍历可能没走完");
}

#[test]
fn player_icons_resolve_to_real_geometry() {
    let set = icon_set();
    // 未登记的名字回落到进程内唯一的 EMPTY：地址比较能判出「真的解析出了几何」。
    let missing = set.get("__definitely_not_a_registered_icon__");
    for name in ["play", "pause", "volume-up", "volume-off", "fullscreen"] {
        let icon = set.get(name);
        assert!(
            !std::ptr::eq(icon, missing),
            "图标 {name} 落到了 EMPTY —— SVG 解析或登记有问题"
        );
    }
}

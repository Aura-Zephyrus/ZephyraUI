//! 本文件由 `ZephyraUI/src/i18n.rs` 的 `#[cfg(test)] mod tests` 抽出 ——
//! 测试代码与生产代码分离。
//!
//! 模块路径不变（生产文件里仍用 `#[path = ...]` 声明），所以 `use super::*`
//! 与对父模块私有项的访问照旧。

use super::*;

#[test]
fn builtin_text_has_all_three_languages() {
    let msgs = builtin_messages();
    for locale in ["en", "zh-Hans", "ja"] {
        assert!(
            msgs.get(locale, CHECKBOX_SELECT_ALL).is_some(),
            "控件层自带的 key 在 {locale} 下缺译文"
        );
    }
}

#[test]
fn builtin_keys_are_namespaced() {
    for key in BUILTIN_KEYS {
        assert!(
            key.starts_with(NAMESPACE),
            "控件层 key 必须带 {NAMESPACE}. 前缀，否则与应用层撞名：{key}"
        );
    }
}

#[test]
fn merge_builtin_never_overwrites_the_app() {
    let mut app = Messages::new();
    app.insert("zh-Hans", CHECKBOX_SELECT_ALL, "全选邮件");
    merge_builtin(&mut app);
    assert_eq!(
        app.get("zh-Hans", CHECKBOX_SELECT_ALL),
        Some("全选邮件"),
        "应用写好的条目被控件层的默认值覆盖了"
    );
    // 另两门仍走控件层默认 —— 不会出现「只有一门语言有译文」。
    assert_eq!(app.get("en", CHECKBOX_SELECT_ALL), Some("Select all"));
}

#[test]
fn merge_builtin_fills_the_missing_locale() {
    let mut app = Messages::new();
    app.insert("zh-Hans", CHECKBOX_SELECT_ALL, "全选邮件");
    merge_builtin(&mut app);
    assert_eq!(app.get("ja", CHECKBOX_SELECT_ALL), Some("すべて選択"));
}

#[test]
fn resolve_falls_back_to_the_key_itself() {
    let msgs = builtin_messages();
    let prefs = Preferences::new();
    assert_eq!(
        resolve(&msgs, &prefs, "en", "zephyra.nope"),
        "zephyra.nope",
        "查不到时必须回落键名本身（可见的降级），不是空串"
    );
}

#[test]
fn key_param_contract_covers_the_button_and_selection_families() {
    for (component, param) in [
        ("Button", "label_key"),
        ("PillButton", "label_key"),
        ("RoundButton", "label_key"),
        ("CheckBox", "label_key"),
        ("CheckBoxGroup", "label_key"),
        ("RadioButton", "label_key"),
        ("ToggleSwitch", "label_key"),
    ] {
        assert!(
            is_key_param(component, param),
            "契约表漏了 {component}.{param}"
        );
    }
    assert!(!is_key_param("Button", "label"));
}

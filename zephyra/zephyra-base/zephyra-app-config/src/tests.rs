use super::*;

fn test_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("zephyra-app-config-{}-{name}", std::process::id()))
}

#[test]
fn reads_frameless_setting_from_project_root() {
    let root = test_root("frameless");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(CONFIG_FILE), r#"{"window":{"frameless":true}}"#).unwrap();

    let config = load_from_project_root(&root).unwrap();

    assert!(config.window.frameless);
    // 未写 transparent 时按 serde default 落为 false —— 主窗口透明与否也必须
    // 有「文件没写就是不透明」的确定性，不能悬在 Option 里。
    assert!(!config.window.transparent);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reads_transparent_setting_from_project_root() {
    let root = test_root("transparent");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(CONFIG_FILE), r#"{"window":{"transparent":true}}"#).unwrap();

    let config = load_from_project_root(&root).unwrap();

    assert!(config.window.transparent);
    std::fs::remove_dir_all(root).unwrap();
}

/// 显式给定 per-window 选项时覆盖主窗口配置 —— 「不同窗口不同定义」的语义门禁。
#[test]
fn window_opts_override_the_main_window_default() {
    let mut config = AppConfig::default();
    config.window.transparent = true;

    let follow = WindowOpts::default();
    assert!(follow.resolve_transparent(&config));

    let explicit = WindowOpts {
        transparent: Some(false),
        ..Default::default()
    };
    assert!(!explicit.resolve_transparent(&config));
}

/// `windows.<name>` 只写它要改的字段，其余继承 `window` 节。
#[test]
fn named_profile_inherits_unspecified_fields() {
    let root = test_root("profile-inherit");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join(CONFIG_FILE),
        r#"{
            "window": { "frameless": true, "transparent": false },
            "windows": { "inspector": { "title": "Inspector", "width": 420, "height": 600 } }
        }"#,
    )
    .unwrap();

    let config = load_from_project_root(&root).unwrap();
    let opts = WindowOpts::named(&config, "inspector");

    assert_eq!(opts.title.as_deref(), Some("Inspector"));
    assert_eq!(opts.size, Some((420, 600)));
    // 没写的字段继承主窗口节。
    assert!(opts.resolve_frameless(&config));
    assert!(!opts.resolve_transparent(&config));
    assert!(
        opts.resolve_resizable(&config),
        "未写 resizable ⇒ 默认可调整"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// 命名定义里显式写的字段优先于 `window` 节。
#[test]
fn named_profile_overrides_main_window_fields() {
    let root = test_root("profile-override");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join(CONFIG_FILE),
        r#"{
            "window": { "frameless": false, "transparent": false },
            "windows": { "hud": { "frameless": true, "transparent": true, "always_on_top": true, "tool_window": true } }
        }"#,
    )
    .unwrap();

    let config = load_from_project_root(&root).unwrap();
    let opts = WindowOpts::named(&config, "hud");

    assert!(opts.resolve_frameless(&config));
    assert!(opts.resolve_transparent(&config));
    assert!(opts.resolve_always_on_top(&config));
    assert!(opts.resolve_tool_window(&config));
    std::fs::remove_dir_all(root).unwrap();
}

/// 未知名字回退到主窗口节，但保留名字（让拼错在日志里看得见）。
#[test]
fn unknown_named_profile_falls_back_to_main_window() {
    let mut config = AppConfig::default();
    config.window.frameless = true;

    let opts = WindowOpts::named(&config, "typo");

    assert_eq!(opts.name.as_deref(), Some("typo"));
    assert!(opts.resolve_frameless(&config));
}

/// `WindowOpts::main` 与 `WindowOpts::default()` 对同一份配置给出同样的结果。
#[test]
fn main_opts_match_default_resolution() {
    let mut config = AppConfig::default();
    config.window.frameless = true;
    config.window.transparent = false;

    let main = WindowOpts::main(&config);
    let default = WindowOpts::default();

    assert_eq!(
        main.resolve_frameless(&config),
        default.resolve_frameless(&config)
    );
    assert_eq!(
        main.resolve_transparent(&config),
        default.resolve_transparent(&config)
    );
}

/// 透明必然无边框：这是三个后端共用的判据，不能只在某一层做。
#[test]
fn transparent_implies_frameless() {
    let config = AppConfig::default();
    let opts = WindowOpts {
        transparent: Some(true),
        frameless: Some(false),
        ..Default::default()
    };

    assert!(opts.resolve_frameless(&config));
}

#[test]
fn missing_file_uses_default_window_settings() {
    let root = test_root("missing");
    std::fs::create_dir_all(&root).unwrap();

    let config = load_from_project_root(&root).unwrap();

    assert!(!config.window.frameless);
    assert!(!config.window.transparent);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_json_is_reported() {
    let root = test_root("invalid");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(CONFIG_FILE), r#"{"window":{"frameless":"yes"}}"#).unwrap();

    let error = load_from_project_root(&root).unwrap_err();

    assert!(error.contains(CONFIG_FILE));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn uses_embedded_config_when_built_with_one() {
    let Some((contents, source)) = EMBEDDED_CONFIG else {
        return;
    };
    let expected = parse_config(contents, source).unwrap();

    assert_eq!(load_from_current_dir(), expected);
}

#[test]
fn frameless_env_override_forces_both_directions_and_ignores_other_values() {
    let mut config = AppConfig::default();
    assert!(!config.window.frameless);

    apply_frameless_override(&mut config, Some("1"));
    assert!(config.window.frameless);

    apply_frameless_override(&mut config, Some("0"));
    assert!(!config.window.frameless);

    // Other values (and absence) leave the setting untouched.
    config.window.frameless = true;
    apply_frameless_override(&mut config, Some("yes"));
    assert!(config.window.frameless);
    apply_frameless_override(&mut config, None);
    assert!(config.window.frameless);
}

#[test]
fn transparent_env_override_forces_both_directions_and_ignores_other_values() {
    let mut config = AppConfig::default();

    apply_transparent_override(&mut config, Some("1"));
    assert!(config.window.transparent);

    apply_transparent_override(&mut config, Some("0"));
    assert!(!config.window.transparent);

    // Other values (and absence) leave the setting untouched.
    config.window.transparent = true;
    apply_transparent_override(&mut config, Some("yes"));
    assert!(config.window.transparent);
    apply_transparent_override(&mut config, None);
    assert!(config.window.transparent);
}

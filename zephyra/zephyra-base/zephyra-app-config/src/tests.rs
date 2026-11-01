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
    assert!(follow.transparent.unwrap_or(config.window.transparent));

    let explicit = WindowOpts {
        transparent: Some(false),
    };
    assert!(!explicit.transparent.unwrap_or(config.window.transparent));
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

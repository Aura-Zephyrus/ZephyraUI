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
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_file_uses_default_window_settings() {
    let root = test_root("missing");
    std::fs::create_dir_all(&root).unwrap();

    let config = load_from_project_root(&root).unwrap();

    assert!(!config.window.frameless);
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

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=ZEPHYRA_APP_CONFIG_PATH");

    let embedded = match env::var_os("ZEPHYRA_APP_CONFIG_PATH") {
        Some(path) => {
            let path = PathBuf::from(path);
            println!("cargo:rerun-if-changed={}", path.display());
            let path = format!("{:?}", path.to_string_lossy().as_ref());
            format!(
                "const EMBEDDED_CONFIG: Option<(&str, &str)> = \
                 Some((include_str!({path}), {path}));\n"
            )
        }
        None => "const EMBEDDED_CONFIG: Option<(&str, &str)> = None;\n".to_string(),
    };

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(out_dir.join("embedded_config.rs"), embedded)
        .expect("write embedded application config declaration");
}

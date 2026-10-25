//! 构建期：把 `ui/controls.zypui` 编译成二进制（OUT_DIR），运行时零解析。
//!
//! 与 zstylec 同一模式：编译器是 build-dependency，源文件改动触发重编
//! （cargo 对 `include_str!` 的依赖追踪走 rerun-if-changed）。

use std::env;
use std::path::PathBuf;

fn main() {
    let src = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("ui/controls.zypui");
    println!("cargo:rerun-if-changed={}", src.display());

    let source = std::fs::read_to_string(&src).expect("读 ui/controls.zypui");
    // 名字泄漏成 'static：zypuic 的诊断/IR 全程 &'static。
    let name: &'static str = Box::leak(src.display().to_string().into_boxed_str());

    let bytes = match zypuic::compile(&[(name, source.as_str())]) {
        Ok(bytes) => bytes,
        Err(diags) => {
            let mut msg = String::from("ui/controls.zypui 编译失败：\n");
            for d in &diags {
                msg.push_str(&d.render());
                msg.push('\n');
            }
            panic!("{msg}");
        }
    };

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("controls.zypui.bin");
    std::fs::write(&out, &bytes).expect("写编译产物");
}

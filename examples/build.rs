//! 构建期：把 `ui/demo.zypui` 编译成一份二进制（OUT_DIR），运行时零解析。
//!
//! demo 的树全是裸元素（container/text/image/control），不引用控件组件库
//! —— 单文件一个工件即可。运行时侧 `ui.rs` 的 `include_bytes!` 与这里的
//! 产物一一对应。

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = manifest.join("ui/demo.zypui");
    println!("cargo:rerun-if-changed={}", src.display());

    let src_str = std::fs::read_to_string(&src).expect("读 ui/demo.zypui");
    // 名字泄漏成 'static：zypuic 的诊断/IR 全程 &'static。
    let name: &'static str = Box::leak(src.display().to_string().into_boxed_str());

    let bytes = match zypuic::compile(&[(name, src_str.as_str())]) {
        Ok(bytes) => bytes,
        Err(diags) => {
            let mut msg = String::from("ui/demo.zypui 编译失败：\n");
            for d in &diags {
                msg.push_str(&d.render());
                msg.push('\n');
            }
            panic!("{msg}");
        }
    };

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("demo.zypui.bin");
    std::fs::write(&out, &bytes).expect("写编译产物");
}

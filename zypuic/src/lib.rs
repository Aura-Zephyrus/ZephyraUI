#![forbid(unsafe_code)]
#![deny(rustdoc::broken_intra_doc_links)]
//! `zypuic` 的库入口：解析、校验、编译 `.zypui` 文本。
//!
//! # 为什么既是 bin 又是 lib
//!
//! bin 是命令行工具（`zypuic check` 进 CI）。lib 给 **build.rs** 用：
//! 应用在构建期调 [`compile`] 把 `.zypui` 编进 `OUT_DIR`，`include_bytes!`
//! 吃进运行时；也给 **dev 模式**用 —— 宿主运行期 [`parse_all`] 文本、热迭代，
//! 与 zstylec 的 dev 模式同一先例。
//!
//! ```text
//! dev:     .zypui 文本 ──parse_all──▶ RawUi ──Ui::new──▶ 每帧 build
//! release: build.rs ──compile──▶ OUT_DIR/*.bin ──include_bytes!──▶ decode ─▶ 同上
//! ```
//!
//! 两条路共享同一份 IR（见 `zypui::ir` 的命门说明）。

mod build;
mod encode;
mod expr_parse;
pub mod xml;

#[cfg(test)]
mod tests;

// 排版门禁：扫仓内全部 `.zypui`（不是内联样例），所以单独一个文件。
#[cfg(test)]
#[path = "tests/style_gate_tests.rs"]
mod style_gate_tests;

pub use xml::{Diag, Level, XEl, XNode};

use zypui::RawUi;

/// 解析 + 构建多份 `.zypui` 文件（组件跨文件共享命名空间）。
///
/// 出错时返回**全部**诊断 —— 一次看到所有错，而不是改一个跑一趟。
pub fn parse_all(files: &[(&'static str, &str)]) -> Result<RawUi, Vec<Diag>> {
    let mut parsed: Vec<(&'static str, Vec<XEl>)> = Vec::with_capacity(files.len());
    for (name, src) in files {
        let (els, diags) = xml::parse_xml(src, name);
        // 非法 XML 直接短路：build 阶段的诊断建立在语法成立之上。
        if has_errors(&diags) {
            return Err(diags);
        }
        parsed.push((name, els));
    }
    build::build(&parsed)
}

/// 单文件版（CLI check 用）。
pub fn parse(file: &'static str, src: &str) -> Result<RawUi, Vec<Diag>> {
    parse_all(&[(file, src)])
}

/// 编译成二进制（build.rs 的入口）。等价 `encode(&parse_all(..)?)`。
pub fn compile(files: &[(&'static str, &str)]) -> Result<Vec<u8>, Vec<Diag>> {
    Ok(encode::encode(&parse_all(files)?))
}

/// 诊断是否包含错误。
pub fn has_errors(diags: &[Diag]) -> bool {
    diags.iter().any(|d| d.level == Level::Error)
}

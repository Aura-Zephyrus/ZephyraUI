#![deny(rustdoc::broken_intra_doc_links)]
//! `zstylec` 的库入口：解析 `.zstyle` 文本。
//!
//! # 为什么既是 bin 又是 lib
//!
//! bin 是命令行工具（构建期把 `.zstyle` 编成二进制）。
//! lib 是给 **dev 模式**用的：宿主要在运行期直接解析文本、换样式表看效果，
//! 不可能每改一次就去敲一次命令行。
//!
//! ```text
//! dev:     .zstyle 文本 ──zstylec::parse()──▶ StyleSheet ──apply──▶ Node
//! release: .zstyle.bin  ──from_bytes()────▶ StyleSheet ──apply──▶ Node
//!                    ^^^^^^^^^^^ 尚未实现（B5）
//! ```
//!
//! ⚠️ 生产二进制链这个 crate 才拿得到 dev 模式。若只想要 release，
//! 只链 `zephyra-style` 即可 —— 它不含 `cssparser`。
//!
//! # `fmt`：只动空白
//!
//! IR 会把值归一化（`10px` → `Float`、`@令牌` → `Resource`），从 IR 回写不可能
//! 还原源码拼写，所以 [`format`] 直接吃原文、只重排空白 —— 见 `fmt` 模块文档。

mod check;
mod errors;
mod fmt;
mod parse;
mod props_map;
mod value;

pub use check::{check, check_file, check_text, has_errors, Diag, FileReport, Level};
// 不叫 `format` —— 那会跟 std 的 `format!` 宏撞名，`cargo doc` 直接
// 「error: `format` is both a function and a macro」整个 crate 都文档不出来。
pub use fmt::{format as format_style, is_formatted};
pub use parse::{parse, ParseResult};

/// 全部受支持的 CSS 属性名（`zstylec props` 的数据源）。
///
/// 为什么暴露成公开 API：CLI 要列它们，而「CLI 与编译器看到的属性表不一致」正是
/// 那种「改了 A 结果 B 不变」���坑 —— 走同一个函数就没有第二条路径。
pub fn supported_props() -> Vec<&'static str> {
    props_map::all_css_names().collect()
}

#[cfg(test)]
mod tests;

// `fmt` 的用例按模块分文件：排版器的承诺（不改语义 / 幂等 / 不认识的输入返回
// None）与解析器的用例混在一个文件里只会互相稀释。
#[cfg(test)]
#[path = "tests/fmt_tests.rs"]
mod fmt_tests;

// 排版门禁：扫仓内全部 `.zstyle`（不是内联样例），所以单独一个文件。
#[cfg(test)]
#[path = "tests/style_gate_tests.rs"]
mod style_gate_tests;

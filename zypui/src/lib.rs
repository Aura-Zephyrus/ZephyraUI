#![forbid(unsafe_code)]
#![deny(rustdoc::broken_intra_doc_links)]
//! `.zypui` 运行时：同一份 IR 的解码、表达式求值与实例化。
//!
//! # 数据流
//!
//! ```text
//! dev:     .zypui 文本 ──zypuic::parse──▶ RawUi ──Ui::new──┐
//! release: .zypui.bin ──zypui::decode─────────────────────┤
//!                                                          ▼
//!                                            ui.build(page, host, interner)
//!                                                          ▼
//!                                                  Vec<Element>
//! ```
//!
//! 两条来源汇进**同一份** [`ir::RawUi`]，之后实例化只有一份实现。
//! 「dev 对、release 错」这类漂移在结构上不可能发生。
//!
//! # 模块
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`val`] | 求值结果的值类型（字符串全是 `&'static`） |
//! | [`expr`] | 表达式 AST（Slint 式：比较/逻辑/拼接/三元/调用/绑定引用） |
//! | [`ir`] | 声明树：元素/条件/迭代/组件引用/槽位 |
//! | [`intern`] | 动态字符串去重（`&'static` 化，同一串只泄一份） |
//! | [`eval`] | 每帧求值：作用域链（组件实参 → for 变量）+ 内置词 + 宿主绑定 |
//! | [`decode`] | 二进制 → [`ir::RawUi`]（零解析，长度前缀，损坏输入返回 Err 不 panic） |
//! | [`instantiate`] | [`Ui`] + [`Host`] → `Element`（每帧一次，含组件展开） |

pub mod decode;
pub mod eval;
pub mod expr;
pub mod instantiate;
pub mod intern;
pub mod ir;
pub mod val;

#[cfg(test)]
mod tests;

pub use expr::{BinOp, Expr};
pub use instantiate::{Host, Ui};
pub use intern::Interner;
pub use ir::{ClassTok, Component, ElementNode, ForNode, IfNode, Node, Page, RawUi, UseNode};
pub use val::Val;

use std::fmt;

/// 解码或实例化失败的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZypuiError {
    pub msg: String,
}

impl fmt::Display for ZypuiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for ZypuiError {}

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T, ZypuiError> {
    Err(ZypuiError { msg: msg.into() })
}

//! 声明式动画的接入层。
//!
//! 引擎（`zephyra-svg-anim`）只产出纯数据；这里把数据接到控件的 paint 属性上。
//! 运行时语法只有 SMIL；CSS `@keyframes` 只作构建期导入（见 docs/developer/syntax.md）。

pub mod driver;

pub use driver::{AnimDriver, PREFADE_SOURCES};

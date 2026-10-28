#![forbid(unsafe_code)]
//! ZephyraUI —— 控件层：样式表、令牌、控件结构声明。
//!
//! # 这个 crate 的边界
//!
//! | 在这里 | 不在这里 |
//! |---|---|
//! | 控件的 `.zstyle` 样式表（[`styles`]） | 应用外壳与页面布局的 `.zstyle` |
//! | `@令牌` 名单镜像（[`tokens`]） | 路由、页面、应用状态 |
//! | 控件结构声明 `ui/controls.zypui` | 任何渲染后端 |
//!
//! # 结构的真相只有一份：`.zypui`
//!
//! 控件结构住在 [`ui/controls.zypui`](ui/controls.zypui)（声明式组件模板），
//! 由**应用**在自己的 `build.rs` 里与页面模板一起编译成二进制 —— 见
//! `gallery/build.rs`：它直接读本目录下的这份文件，所以结构真相全仓只有
//! 一份，gallery 不复制它。
//!
//! ⚠ 本 crate 曾同时导出 59 个同名 Rust 构造器，与 `.zypui` 逐字节 parity。
//! 那一层已经完成使命（证明翻译无误）并删除：全仓除 parity 测试外**无人消费**，
//! 却让每个控件改一次要改两处。
//!
//! # 两条不能碰的规矩
//!
//! 1. **外观不进源码。** 控件只发 `NodeTag` / `ElementKey` / 分类名 / 内容，
//!    颜色与尺寸全在 `.zstyle`。门禁按**整棵树 × 全部页面 × 全部交互态**判
//!    （`gallery/tests/gates.rs`）。
//!
//! 2. **本 crate 不依赖渲染后端。** 控件库的用户不该被迫引入 D3D12。
//!    后端由应用（`gallery/`）用 feature 选。

pub mod styles;
pub mod tokens;
pub mod ui;

pub use styles::{stylesheet, STYLESHEETS};
pub use tokens as token_names;

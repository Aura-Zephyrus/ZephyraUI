//! 页面表 —— 一页一行。
//!
//! # 迁移后这里只剩**元数据**
//!
//! 页面的**结构**全部住在 `ui/shell.zypui`（每页一个组件，`Window` 页按
//! 路由分发）。这里保留 id / 英文标题 / 图标的三元组：导航栏、门禁
//! （`route_table_and_page_table_agree`）与文档仍以它为页面清单的唯一
//! 真相。画面上的标题走 [`crate::app::Gallery::tr`]（本地化），不走这里的
//! 英文原名 —— 两者分开是为了让「路由元数据」不依赖「用户的语言」。
//!
//! # 顺序即导航栏从上到下的顺序；Settings 在最后
//!
//! Settings 不在 `ROUTES` 里（它由 shell 置底），但它仍是一个页面，所以
//! 仍在这里。两份表的分工与门禁见 `mod.rs` 的 `ROUTES` 说明。

/// `(id, 英文标题, 图标名)`。图标名来自 core 的内置图标集 —— 拼错时
/// `ICON_NAME` 解析不到、图标**静默**消失，由门禁
/// `every_icon_name_the_tree_emits_is_registered` 盯住。
pub const PAGES: &[(&str, &str, &str)] = &[
    // ---- 指南层（学习顺序）----
    ("guide", "Quick start", "home"),
    ("state", "State", "check"),
    ("styling", "Styling", "pen"),
    ("layout", "Layout", "grid"),
    ("theme", "Theme & i18n", "settings"),
    ("testing", "Testing", "bell"),
    // ---- 附录：控件参考（一页内 segmented 切九族）----
    ("reference", "Controls", "doc"),
    // 液态玻璃：一等路由（四个 `[−]/[+]` 步进器调折射参数）。
    ("glass", "Liquid Glass", "sliders"),
    // Settings 排在最后，但**不是** `ROUTES` 的一员（它由 shell 置底）。
    ("settings", "Settings", "settings"),
];

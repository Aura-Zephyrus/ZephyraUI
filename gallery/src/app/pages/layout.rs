//! 布局件：Card / Expander / SettingCard / SettingExpander。
//!
//! # 这一页不是"控件展示"而是"容器用法示范"
//!
//! 前几页的控件都是**叶子**（自己就是一个矩形）。这一页的东西全是
//! **容器**——它们自己不定尺寸，靠子项长大。所以要展示的是「怎么摆」，
//! 而不是「它们长什么样」：每张卡里放的内容才是重点。

use zephyra_core::element::Element;

use super::Gallery;
use crate::app::pages;
use zephyra_ui::controls;

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "layout",
        "page-layout",
        vec![
            pages::section(
                "sec-card",
                "Card",
                vec![pages::page_root(
                    "cards",
                    "card-grid",
                    vec![
                        controls::card(
                            "card-a",
                            "card",
                            vec![
                                controls::text_label("ca-t", g.tr("ui.card"), "text-body-strong"),
                                controls::caption("ca-d", g.tr("ui.a-plain-surface")),
                            ],
                        ),
                        controls::card(
                            "card-b",
                            "card",
                            vec![
                                controls::text_label("cb-t", g.tr("ui.card"), "text-body-strong"),
                                controls::caption("cb-d", g.tr("ui.same-class-other-content")),
                            ],
                        ),
                    ],
                )],
            ),
            pages::section(
                "sec-expander",
                "Expander",
                vec![controls::expander(
                    "exp:details",
                    "Advanced",
                    g.expanded,
                    vec![
                        controls::text_label(
                            "exp-l1",
                            g.tr("ui.hidden-until-expanded"),
                            "text-caption",
                        ),
                        controls::text_label("exp-l2", g.tr("ui.second-line"), "text-caption"),
                    ],
                )],
            ),
            pages::section(
                "sec-setting",
                "Setting rows",
                vec![
                    controls::setting_item(
                        "set-item-a",
                        "bell",
                        "Notifications",
                        "A row with an icon and a description.",
                    ),
                    controls::setting_expander(
                        "set-exp",
                        "Advanced",
                        "Collapsible group",
                        // 展开态走独立的 `ref_exp`：`exp:details` 是另一页的
                        // 展开器，共用会出现「在这页收起，那页也跟着收起」
                        // 的幽灵联动——两件事各有一个状态。
                        g.ref_exp,
                        vec![controls::setting_item(
                            "set-item-b",
                            "settings",
                            "Experimental",
                            "Inside the expander.",
                        )],
                    ),
                ],
            ),
        ],
    )
}

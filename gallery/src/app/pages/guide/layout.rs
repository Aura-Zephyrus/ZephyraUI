//! 布局与响应式：flex 的填充与折行，以及仍真实存在的边界。
//!
//! # 这一章教什么
//!
//! 两个 flex 例子（填充、折行）+ 两条仍然真实的边界（无滚动、断点）外加
//! 44 epx 触控目标。先知道「flex 要容器**显式声明**才生效」，才不会把
//! 「写了 flex-grow 却没反应」当成 bug 去查样式表 —— 没声明的容器走 Stack，
//! 子项的 flex-grow 一概不生效（core 的门禁
//! `layout_tests::a_container_without_flex_declarations_ignores_flex_grow` 钉住）。
//!
//! # 无滚动为什么还在
//!
//! core 没有滚动容器：内容超出的唯一表现是下沿被裁掉。grow / 折行只改变
//! 「内容怎么排」，不带来滚动 —— 所以它俩不是这一条的替代品。

use zephyra_core::element::Element;
use zephyra_ui::controls;

use crate::app::pages;
use crate::app::Gallery;

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "layout",
        "page-layout",
        vec![
            controls::text_label(
                "layout-blurb",
                g.tr(
                    "ui.flex-rows-that-grow-and-wrap-and-the-one-boundary-that-is-still-real-no-scroll",
                ),
                "text-body",
            ),
            // ---- flex：填充与折行 ----
            //
            // 两个容器**并排**放在一行里（内容区高 756，分两行会把本页最后一节
            // 的触控按钮顶出客户区 —— 而那正是这一页自己教的「没有滚动，掉出去
            // 就是掉出去」）。两个容器都在样式表里声明了 `flex-direction`
            // （`.fx-row` / `.fx-wrap`），所以它们走分配路径：左容器演示 grow，
            // 右格吃掉余下的全部行宽（420 − 100 − 10 = 310）；右容器演示折行，
            // 340px 装得下三个 100px 格 + 两个 10px 间距，第四个折到第二行。
            pages::section(
                "layout-flex",
                g.tr("ui.flexbox-rows-that-grow-and-wrap"),
                vec![pages::demo_many(
                    g.tr("ui.flex-grow-1-fills-the-row-and-flex-wrap-packs-items-into-lines"),
                    vec![
                        // grow：右格吃掉余下的全部行宽（420 − 100 − 10 = 310）。
                        pages::page_root(
                            "fx-grow-demo",
                            "fx-row",
                            vec![
                                controls::card(
                                    "fx-fixed",
                                    "fx-cell",
                                    vec![controls::text_label(
                                        "fx-fixed-t",
                                        g.tr("ui.fixed"),
                                        "text-caption",
                                    )],
                                ),
                                controls::card(
                                    "fx-grow-cell",
                                    "fx-grow",
                                    vec![controls::text_label(
                                        "fx-grow-t",
                                        g.tr("ui.grows"),
                                        "text-caption",
                                    )],
                                ),
                            ],
                        ),
                        // 折行：340 装得下 100+10+100+10+100 = 330，装不下第四个。
                        pages::page_root(
                            "fx-wrap-demo",
                            "fx-wrap",
                            vec![
                                controls::card("fx-w1", "fx-cell", vec![]),
                                controls::card("fx-w2", "fx-cell", vec![]),
                                controls::card("fx-w3", "fx-cell", vec![]),
                                controls::card("fx-w4", "fx-cell", vec![]),
                                controls::card("fx-w5", "fx-cell", vec![]),
                                controls::card("fx-w6", "fx-cell", vec![]),
                            ],
                        ),
                    ],
                )],
            ),
            // ---- 边界一：无滚动 ----
            //
            // 内容区高 756。这一列卡片的总高故意贴近上限 —— 开发者拖窗口
            // 变小时会看到最后一张卡片的下沿「被切掉」，而不是出现滚动条。
            // 那是 core 的当前边界，不是 bug。
            pages::section(
                "layout-noscroll",
                g.tr("ui.no-scroll-overflow-is-clipped"),
                vec![
                    pages::demo(
                        g.tr("ui.card-1"),
                        controls::card(
                            "lc-1",
                            "card",
                            vec![
                                controls::text_label("lc-1-t", g.tr("ui.first"), "text-body-strong"),
                                controls::text_label(
                                    "lc-1-d",
                                    g.tr("ui.fits-inside-the-card"),
                                    "text-caption",
                                ),
                            ],
                        ),
                    ),
                    pages::demo(
                        g.tr("ui.card-2"),
                        controls::card(
                            "lc-2",
                            "card",
                            vec![
                                controls::text_label("lc-2-t", g.tr("ui.second"), "text-body-strong"),
                                controls::text_label("lc-2-d", g.tr("ui.also-fits"), "text-caption"),
                            ],
                        ),
                    ),
                    pages::demo(
                        g.tr("ui.card-3"),
                        controls::card(
                            "lc-3",
                            "card",
                            vec![
                                controls::text_label("lc-3-t", g.tr("ui.third"), "text-body-strong"),
                                controls::text_label(
                                    "lc-3-d",
                                    g.tr("ui.watch-me-when-the-window-shrinks"),
                                    "text-caption",
                                ),
                            ],
                        ),
                    ),
                ],
            ),
            // ---- 边界二：断点三档 ----
            //
            // 宽度 640 / 1000 / 1400 是官方三档的边界。窄档下导航栏收成图标列
            // （`nav-pane.compact`），内容区拿到全宽。开发者拖窗口宽度就能看到切换。
            pages::section(
                "layout-breakpoints",
                "Breakpoints: small / medium / large",
                vec![pages::demo(
                    g.tr("ui.current-breakpoint"),
                    controls::text_label(
                        "layout-bp",
                        match g.width_class() {
                            zephyra_core::responsive::WidthClass::Small => {
                                g.tr("ui.small-nav-collapsed-to-icons")
                            }
                            zephyra_core::responsive::WidthClass::Medium => {
                                g.tr("ui.medium-nav-shows-labels")
                            }
                            zephyra_core::responsive::WidthClass::Large => {
                                g.tr("ui.large-full-layout")
                            }
                        },
                        "text-body-strong",
                    ),
                )],
            ),
            // ---- 边界三：触控目标 ----
            //
            // 官方触控目标是 44 epx。小于这个尺寸的按钮在触控设备上点不中 ——
            // 而鼠标路径不受影响。`Gallery::touch_target` 给出官方换算公式，
            // 门禁 `touch_target_converts_to_official_ephemeral_px` 钉住它。
            // 两个按钮都发 `act:` key —— 反馈与尺寸演示互不干扰。
            pages::section(
                "layout-touch",
                "Touch targets need 44 epx",
                vec![
                    pages::demo(
                        g.tr("ui.44-epx-correct"),
                        controls::button(
                            "act:touch-ok",
                            "Big enough",
                            None,
                            "standard",
                            g.hovers("act:touch-ok"),
                            g.holds("act:touch-ok"),
                        ),
                    ),
                    pages::demo(
                        g.tr("ui.28-epx-too-small"),
                        controls::round_button(
                            "act:touch-bad",
                            "Tiny",
                            None,
                            false,
                            g.hovers("act:touch-bad"),
                            g.holds("act:touch-bad"),
                        ),
                    ),
                ],
            ),
        ],
    )
}

//! 下拉按钮（同一控件变形为菜单）的「进去 / 返回」回归。
//!
//! 两条路径分开钉：
//!
//! - **选项 → 收起**：Up 的 click 分派里用前缀认 `liquid:dd:*:opt*`；
//! - **点外面 → 收起**：空白处没有可交互目标，router 不产生 Click，只能靠
//!   Down + `hovered` 判归属。只测前者会漏掉「点外面关不掉」。

use zephyra_fx::{InputState, PointerInput, PointerInputPhase, View};

use zephyra_gallery::app::{Gallery, LIQUID_DD_GLASS_KEY, LIQUID_DD_SOLID_KEY};

fn down_with_hover(hovered: Option<&'static str>) -> InputState {
    InputState {
        hovered: hovered.map(Into::into),
        pointer_events: vec![PointerInput {
            x: 900.0,
            y: 700.0,
            phase: PointerInputPhase::Down,
        }],
        ..InputState::default()
    }
}

#[test]
fn choosing_an_option_closes_the_menu_and_records_the_action() {
    let mut g = Gallery::default();
    g.click(LIQUID_DD_SOLID_KEY);
    assert!(g.liquid_dd_solid_open, "点击本体应展开");

    g.click("liquid:dd:solid:opt2");
    assert!(!g.liquid_dd_solid_open, "选中菜单项后必须收起");
    assert_eq!(g.last_action, "liquid-dd-solid");
}

#[test]
fn outside_down_closes_the_menu_without_a_click_target() {
    let mut g = Gallery::default();
    g.click(LIQUID_DD_SOLID_KEY);
    assert!(g.liquid_dd_solid_open);

    // 空白处：hovered = None，因此 router 不会产出 Click。
    g.on_input(&down_with_hover(None));
    assert!(!g.liquid_dd_solid_open, "按下空白处必须收起");
}

#[test]
fn down_on_the_dropdown_itself_keeps_it_open() {
    let mut g = Gallery::default();
    g.click(LIQUID_DD_SOLID_KEY);
    assert!(g.liquid_dd_solid_open);

    g.on_input(&down_with_hover(Some(LIQUID_DD_SOLID_KEY)));
    assert!(g.liquid_dd_solid_open, "按在本体上不该把菜单收掉");
}

#[test]
fn opening_one_dropdown_closes_the_other() {
    let mut g = Gallery::default();
    g.click(LIQUID_DD_SOLID_KEY);
    g.click(LIQUID_DD_GLASS_KEY);
    assert!(!g.liquid_dd_solid_open);
    assert!(g.liquid_dd_glass_open);

    g.click(LIQUID_DD_SOLID_KEY);
    assert!(g.liquid_dd_solid_open);
    assert!(!g.liquid_dd_glass_open);
}

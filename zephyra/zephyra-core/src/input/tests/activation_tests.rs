// 本文件由 activation.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use crate::context::{dark_context, Context};
use crate::element::Element;
use crate::id::{ElementKey, NodeTag};
use crate::input::text::{Key, KeyInput};
use crate::node::Node;
use crate::property::PropertyValue;
use crate::props::{ACTIVATABLE, DISABLED, FOCUSABLE};

use super::ActivationState;

/// 造一棵「焦点 + 可激活」都齐的树：
///
/// ```text
/// root
///   btn  (FOCUSABLE + ACTIVATABLE)      —— 一个正常按钮
///   dead (FOCUSABLE + ACTIVATABLE + DISABLED) —— 禁用按钮
///   box  (FOCUSABLE，不 ACTIVATABLE)     —— 文本框（Space 打空格）
/// ```
fn tree() -> Node {
    let el = Element::new(NodeTag::Container)
        .with_key("root")
        .with_children([
            Element::new(NodeTag::Control)
                .with_key("btn")
                .with(&FOCUSABLE, PropertyValue::bool(true))
                .with(&ACTIVATABLE, PropertyValue::bool(true)),
            Element::new(NodeTag::Control)
                .with_key("dead")
                .with(&FOCUSABLE, PropertyValue::bool(true))
                .with(&ACTIVATABLE, PropertyValue::bool(true))
                .with(&DISABLED, PropertyValue::bool(true)),
            Element::new(NodeTag::Control)
                .with_key("box")
                .with(&FOCUSABLE, PropertyValue::bool(true)),
        ]);
    Node::from_element(&el)
}

fn ctx() -> Context<'static> {
    dark_context()
}

fn key(name: &'static str) -> ElementKey {
    ElementKey::Name(name)
}

// ─────────────────────────── 激活键的识别 ───────────────────────────

#[test]
fn space_and_enter_are_activation_keys_others_are_not() {
    assert!(ActivationState::is_activation_key(Key::Space));
    assert!(ActivationState::is_activation_key(Key::Enter));
    assert!(!ActivationState::is_activation_key(Key::Tab));
    assert!(!ActivationState::is_activation_key(Key::Escape));
    assert!(!ActivationState::is_activation_key(Key::Backspace));
}

// ─────────────────────────── 按下闩 ───────────────────────────

#[test]
fn pressing_space_on_a_focused_activatable_node_latches_held() {
    let root = tree();
    let c = ctx();
    let mut act = ActivationState::new();
    let changed = act.key_down(&root, Some(&key("btn")), c);
    assert!(changed, "首次按下应当进入按下态");
    assert_eq!(act.held(), Some(&key("btn")), "held 应指向焦点按钮");
}

#[test]
fn held_is_none_before_any_key() {
    let act = ActivationState::new();
    assert_eq!(act.held(), None, "初始无键盘按下");
}

#[test]
fn repeated_press_does_not_change_state() {
    let root = tree();
    let c = ctx();
    let mut act = ActivationState::new();
    assert!(act.key_down(&root, Some(&key("btn")), c), "首次按下变 true");
    assert!(
        !act.key_down(&root, Some(&key("btn")), c),
        "自动重复的按下不再报「变化」"
    );
    assert_eq!(act.held(), Some(&key("btn")), "held 不变");
}

// ─────────────────────────── 抬起清闩 ───────────────────────────

#[test]
fn releasing_clears_held_and_reports_change() {
    let root = tree();
    let c = ctx();
    let mut act = ActivationState::new();
    act.key_down(&root, Some(&key("btn")), c);
    assert!(act.key_up(), "抬起应当报「退出按下态」");
    assert_eq!(act.held(), None, "held 归空");
    assert!(!act.key_up(), "再次抬起无变化");
}

// ─────────────────────────── FOCUSABLE ≠ ACTIVATABLE ───────────────────────────

#[test]
fn focusing_a_non_activatable_node_does_not_latch() {
    let root = tree();
    let c = ctx();
    let mut act = ActivationState::new();
    // 焦点在文本框（只 FOCUSABLE，不 ACTIVATABLE）：Space 是打空格，不该触发点击。
    let changed = act.key_down(&root, Some(&key("box")), c);
    assert!(!changed, "不可激活节点不进入按下态");
    assert_eq!(act.held(), None, "held 保持空");
}

#[test]
fn no_focus_does_not_latch() {
    let root = tree();
    let c = ctx();
    let mut act = ActivationState::new();
    assert!(!act.key_down(&root, None, c), "无焦点不进入按下态");
    assert_eq!(act.held(), None);
}

// ─────────────────────────── DISABLED 挡住激活 ───────────────────────────

#[test]
fn disabled_node_cannot_be_activated() {
    let root = tree();
    let c = ctx();
    let mut act = ActivationState::new();
    let changed = act.key_down(&root, Some(&key("dead")), c);
    assert!(!changed, "禁用按钮不响应键盘激活");
    assert_eq!(act.held(), None, "held 保持空");
}

// ─────────────────────────── 失焦清闩 ───────────────────────────

#[test]
fn clear_releases_the_latch() {
    let root = tree();
    let c = ctx();
    let mut act = ActivationState::new();
    act.key_down(&root, Some(&key("btn")), c);
    assert!(act.clear(), "失焦清闩应当报「退出按下态」");
    assert_eq!(act.held(), None, "held 归空");
    assert!(!act.clear(), "再清无变化");
}

// ─────────────────────────── on_key 边沿分派 ───────────────────────────

#[test]
fn on_key_dispatches_press_and_release_edges() {
    let root = tree();
    let c = ctx();
    let mut act = ActivationState::new();
    // Space 按下 → 闩住。
    let r = act.on_key(
        &KeyInput::press(Key::Space),
        Some(&root),
        Some(&key("btn")),
        c,
    );
    assert_eq!(r, Some(true), "激活键按下报 Some(变 true)");
    assert_eq!(act.held(), Some(&key("btn")));
    // Space 抬起 → 清闩。
    let r = act.on_key(
        &KeyInput::release(Key::Space),
        Some(&root),
        Some(&key("btn")),
        c,
    );
    assert_eq!(r, Some(true), "激活键抬起报 Some(变 true)");
    assert_eq!(act.held(), None);
}

#[test]
fn on_key_ignores_non_activation_keys() {
    let root = tree();
    let c = ctx();
    let mut act = ActivationState::new();
    let r = act.on_key(
        &KeyInput::press(Key::Tab),
        Some(&root),
        Some(&key("btn")),
        c,
    );
    assert_eq!(r, None, "非激活键返回 None");
    assert_eq!(act.held(), None, "Tab 不该闩住激活");
}

#[test]
fn enter_activates_like_space() {
    let root = tree();
    let c = ctx();
    let mut act = ActivationState::new();
    let r = act.on_key(
        &KeyInput::press(Key::Enter),
        Some(&root),
        Some(&key("btn")),
        c,
    );
    assert_eq!(r, Some(true), "Enter 与 Space 同构");
    assert_eq!(act.held(), Some(&key("btn")));
}

#[test]
fn on_key_with_no_tree_does_not_latch() {
    // 抬起边沿不需要树；按下边沿在树还没建出来时视作不可激活。
    let c = ctx();
    let mut act = ActivationState::new();
    let r = act.on_key(&KeyInput::press(Key::Space), None, Some(&key("btn")), c);
    assert_eq!(r, Some(false), "无树时不闩，但仍是激活键");
    assert_eq!(act.held(), None);
}

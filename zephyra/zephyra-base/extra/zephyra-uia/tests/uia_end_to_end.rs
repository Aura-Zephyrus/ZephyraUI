//! 端到端：UIA 客户端的一次 Toggle ⇒ 应用收到一次 click。
//!
//! # 这条测试守住什么
//!
//! 整条链有五段，每一段单独都有测试：
//!
//! ```text
//! 保留树 ──rules::build──► 快照 ──provider──► COM 提供者
//!                                              │
//!                          客户端调 Toggle() ──► 动作队列
//!                                              │
//!                          bridge.pump ────────► 聚焦 + 键盘激活
//!                                              │
//!                          应用的「上帧按着 → 这帧松了」──► click
//! ```
//!
//! 但**接缝**才是 bug 的地方 —— 每一段都对、接起来不通，症状是
//! 「屏幕阅读器按了复选框，什么都没发生」，而画面上完全看不出异常。
//!
//! 这条测试把五段接起来跑一遍，走的是**真实的 COM 接口**
//! （`IToggleProvider` —— 原始提供者这一侧的图案接口），不是内部函数。
//!
//! ⚠ 注意别用 `IUIAutomationTogglePattern`：那是**客户端侧**接口（UIA 自己
//! 实现，客户端拿到的是它的代理）；原始提供者要实现的是 `IToggleProvider`。
//! 两者 IID 不同 —— 用错的话，UIA 的服务端 QI 会失败，然后**静默**判定
//! 「这个元素没有 Toggle 图案」。

#![cfg(windows)]

use windows::core::Interface;
use windows::Win32::UI::Accessibility::{IToggleProvider, UIA_TogglePatternId};

use zephyra_core::context::Context;
use zephyra_core::element::Element as Elem;
use zephyra_core::id::{ElementKey, NodeTag};
use zephyra_core::input::{ActivationState, FocusState};
use zephyra_core::node::Node;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_core::theme::Theme;
use zephyra_uia::{Bridge, WindowInfo};

/// 动作队列是**进程级单例** ⇒ 两个测试必须串行。
///
/// 这与被测语义一致：UIA 的动作队列本来就只有一个，「谁排的」不该互相干扰。
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 拿锁（毒了就取内层值，让后面的测试继续跑）。
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// 一棵带一颗复选框的树（形状照 `CheckBox` 模板：control + label 子节点）。
fn tree() -> Node {
    let label = Elem::new(NodeTag::Text)
        .with_key("label")
        .with(&props::TEXT, PropertyValue::StaticStr("Enable feature"));
    let checkbox = Elem::new(NodeTag::Control)
        .style("check-box")
        .style("checked")
        .with_key("check:alpha")
        .with(&props::FOCUSABLE, PropertyValue::Bool(true))
        .with(&props::ACTIVATABLE, PropertyValue::Bool(true))
        .with_children([label]);
    let root = Elem::new(NodeTag::Box).with_children([checkbox]);
    Node::from_element(&root)
}

fn ctx() -> Context<'static> {
    Context::new(Box::leak(Box::new(Theme::dark())))
}

/// 应用侧重建 click 的逻辑 —— 与 `gallery/frontend/app/mod.rs` 同一形状。
///
/// 应用认 click 的判据是「`pressed` 由 `Some` 变成 `None`」。
fn reconstruct_click(
    last_pressed: &Option<ElementKey>,
    pressed: &Option<ElementKey>,
) -> Option<ElementKey> {
    match (last_pressed, pressed) {
        (Some(prev), None) => Some(prev.clone()),
        _ => None,
    }
}

#[test]
fn a_toggle_from_a_uia_client_reaches_the_app_as_a_click() {
    let _serial = serial();
    let _ = zephyra_uia::take_actions();
    let mut tree = tree();

    // 1. 宿主每帧发布快照。
    zephyra_uia::publish(zephyra_uia::rules::build(
        &tree,
        &WindowInfo {
            name: "Test".into(),
            client: zephyra_core::core::Rect::new(0.0, 0.0, 800.0, 600.0),
        },
        ctx(),
    ));

    // 2. UIA 客户端拿到那颗复选框的 Toggle 图案。
    let provider = zephyra_uia::provider::element_provider(Some("check:alpha".into()));
    let unknown = unsafe { provider.GetPatternProvider(UIA_TogglePatternId) }
        .expect("复选框必须有 Toggle 图案");
    let toggle = unknown
        .cast::<IToggleProvider>()
        .expect("QI 到 Toggle 图案接口");

    // 3. 客户端看到的状态：勾上了。
    assert_eq!(
        unsafe { toggle.ToggleState() }.expect("当前状态").0,
        1,
        "勾上的复选框在 UIA 里是 ToggleState=1"
    );

    // 4. 客户端按下去 —— 这一步把动作排进队列。
    unsafe { toggle.Toggle() }.expect("Toggle 应当成功");
    assert_eq!(
        zephyra_uia::pending_action_count(),
        1,
        "Toggle 应当排一个动作"
    );

    // 5. 宿主那一帧的尾巴：施加动作。
    let mut focus = FocusState::new();
    let mut activation = ActivationState::new();
    let mut bridge = Bridge::new();
    let mut last_pressed: Option<ElementKey> = None;

    let pumped = bridge.pump(&mut tree, &mut focus, &mut activation, ctx());
    assert_eq!(pumped.applied.len(), 1, "动作应当被施加");
    assert!(pumped.pressed_changed);

    // 6. 帧 N+1 的 `sync_input`：应用看见「按下」。
    let pressed = activation.held().cloned();
    assert_eq!(
        pressed.as_ref().and_then(|k| k.as_name()),
        Some("check:alpha"),
        "闩住的必须是那颗复选框"
    );
    assert!(
        reconstruct_click(&last_pressed, &pressed).is_none(),
        "「按下」这一拍还不算 click"
    );
    last_pressed = pressed;

    // 7. 帧 N+1 的尾巴：松开。
    let pumped = bridge.pump(&mut tree, &mut focus, &mut activation, ctx());
    assert!(pumped.applied.is_empty(), "这一帧只做释放");
    assert!(pumped.pressed_changed);

    // 8. 帧 N+2 的 `sync_input`：应用看见「松开」⇒ click。
    let pressed = activation.held().cloned();
    let click = reconstruct_click(&last_pressed, &pressed);
    assert_eq!(
        click.as_ref().and_then(|k| k.as_name()),
        Some("check:alpha"),
        "UIA 的一次 Toggle 必须变成应用的一次 click"
    );
}

/// 禁用项：UIA 的 Toggle 必须**到不了**应用。
///
/// 客户端通常会先看 `IsEnabled` 再决定要不要给 Toggle 图案，但**不能只靠
/// 客户端守规矩** —— 自动化脚本什么都会干。
#[test]
fn a_toggle_on_a_disabled_checkbox_never_reaches_the_app() {
    let _serial = serial();
    let _ = zephyra_uia::take_actions();
    let mut tree = tree();
    // 把那颗复选框改成禁用的。
    fn disable(n: &mut Node) {
        if n.key().and_then(|k| k.as_name()) == Some("check:alpha") {
            n.set_prop(&props::DISABLED, PropertyValue::Bool(true));
        }
        for c in n.children_mut() {
            disable(c);
        }
    }
    disable(&mut tree);

    zephyra_uia::publish(zephyra_uia::rules::build(
        &tree,
        &WindowInfo {
            name: "Test".into(),
            client: zephyra_core::core::Rect::new(0.0, 0.0, 800.0, 600.0),
        },
        ctx(),
    ));

    let provider = zephyra_uia::provider::element_provider(Some("check:alpha".into()));
    let unknown = unsafe { provider.GetPatternProvider(UIA_TogglePatternId) }
        .expect("复选框有 Toggle 图案（图案在不在与禁不禁用无关）");
    let toggle = unknown
        .cast::<IToggleProvider>()
        .expect("QI 到 Toggle 图案接口");

    unsafe { toggle.Toggle() }.expect("Toggle 应当成功");

    let mut focus = FocusState::new();
    let mut activation = ActivationState::new();
    let mut bridge = Bridge::new();
    let pumped = bridge.pump(&mut tree, &mut focus, &mut activation, ctx());
    assert!(
        pumped.applied.is_empty(),
        "禁用项不该被激活 —— 应用不该收到 click"
    );
    assert!(activation.held().is_none(), "禁用项不该闩住");
}

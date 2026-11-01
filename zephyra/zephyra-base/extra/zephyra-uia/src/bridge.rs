//! 无障碍动作 → 应用的桥。
//!
//! # 为什么动作要「绕一圈键盘」才能到应用
//!
//! UIA 的 `Toggle` / `Invoke` 说的是「**把这个控件按一下**」，而复选框的状态住在
//! **应用**里（`.zypui` 是无状态模板，每帧重建）。所以动作必须回到应用手里，
//! 而且要**走应用认得的那条路** —— 否则同一次「勾上」在鼠标、键盘、屏读器三条
//! 入口下会走出三种结果。
//!
//! 本仓已经有那条路：**键盘激活**。
//!
//! - `FocusState` 把焦点写到节点上（`FOCUSED` 属性）；
//! - `ActivationState::key_down` 在焦点节点 `ACTIVATABLE` 且非 `DISABLED` 时闩住
//!   `held`；
//! - 宿主把 `held` 并进 `InputState::pressed`，应用按「上帧按着 → 这帧松了 =
//!   点击」重建 click（`gallery/frontend/app/mod.rs`）。
//!
//! 于是无障碍动作就复用它：聚焦 + 闩住 → 下一帧松开 → 应用收到一次**与键盘
//! 完全同构**的 click。这条路径已经有门禁钉死（`ActivationState` 的闩住/不闩住
//! 用例），无障碍不需要另开一条 —— 另开一条就等于有两套「点击」语义。
//!
//! # 为什么要「按一下」而不是「直接改状态」
//!
//! 因为状态住在应用里。本 crate（无障碍层）看不到应用的状态模型，也**不该**看到
//! —— 它只认识「控件的 key」。要改状态就得让应用自己改，而应用改状态只有一个入口：
//! 它的 click 处理函数。
//!
//! # 三帧的节奏
//!
//! 应用把 click 认成「`pressed` 由 `Some` 变成 `None`」，所以一次激活至少要跨
//! 两帧才能被看见：
//!
//! ```text
//! 帧 N    闩住（held = Some(k)）      应用看到 pressed = Some(k)   ← 记下
//! 帧 N+1  松开（held = None）          应用看到 pressed = None      ← 点击！
//! ```
//!
//! 本模块的 [`Bridge::pump`] 每帧调一次，**必须**在宿主的 `sync_input` **之后**
//! 调用（见该方法的文档）。
//!
//! # 排队而不是丢
//!
//! 一个动作被施加后，闩住的那一帧里**不再**施加新动作 —— 否则「松开」与「再按下」
//! 落在同一帧，应用看到的 `pressed` 是 `Some(k) → Some(k2)`，那个「没有 `None` 的
//! 间隔」就被吞掉了：第一次点击丢了。
//!
//! 所以新动作**排队**，等闩住释放后的下一帧再施加。屏读器发动作的节奏是人的节奏
//! （几百毫秒），队列实际上永远只有一项；真排队了也只是慢一点，不会丢。

use std::collections::VecDeque;

use zephyra_core::context::Context;
use zephyra_core::id::ElementKey;
use zephyra_core::input::{ActivationState, FocusState};
use zephyra_core::node::Node;

use crate::Action;

/// [`Bridge::pump`] 的结果。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Pumped {
    /// 本帧**实际施加**的动作。
    ///
    /// 被拒的动作（元素不在树上、禁用、不可激活）不在这里 —— 它们被丢弃了，
    /// 因为重试也不会成功（下一帧的树里它同样不可激活）。
    pub applied: Vec<Action>,
    /// `pressed` 通道有没有变（闩住了一个激活目标，或松开了它）。
    ///
    /// 变了宿主就该把新的 `pressed` 报给应用 —— 应用靠「上帧按着 → 这帧松了」
    /// 重建 click，所以它必须**分别**看见那两拍。
    ///
    /// # 为什么宿主有时要自己补一次 `sync_input`
    ///
    /// D3D11 宿主每帧开头都 `sync_input`，于是这一拍自然被下一帧带过去。
    /// D3D12 宿主是**事件驱动**的（`sync_input` 只在事件回调里调），静止时
    /// 一帧都不跑 —— 那宿主就必须在这里显式补一次，否则应用永远等不到
    /// 「松开」那一拍，点击就丢了。
    pub pressed_changed: bool,
}

/// 无障碍动作桥。
///
/// 跨帧持有：它要记住「上一帧闩住了谁」。
#[derive(Debug, Default)]
pub struct Bridge {
    /// 已经按下、等待下一帧释放的激活目标。
    latched: Option<ElementKey>,
    /// 还没施加的动作。
    ///
    /// 只在「闩住的那一帧」之外才会被填 —— 见模块文档「排队而不是丢」。
    queue: VecDeque<Action>,
}

impl Bridge {
    /// 新建（没有闩住、没有排队）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前是否闩住了某个激活目标。
    pub fn is_latched(&self) -> bool {
        self.latched.is_some()
    }

    /// 排队中的动作数（诊断用）。
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// 立刻释放闩住的目标（不等下一帧）。
    ///
    /// # 给谁用
    ///
    /// **事件驱动**的宿主（`zephyra-winit-d3d12`）：它的 `sync_input` 只在事件
    /// 回调里调，静止时一帧都不跑 —— 于是「松开」那一拍可能永远等不到。
    ///
    /// 那种宿主的做法是：闩住 → 立刻 `sync_input`（应用看见「按下」）→
    /// `release_now` → 再 `sync_input`（应用看见「松开」⇒ click）。
    ///
    /// 帧驱动**的宿主（`zephyra-winit`）不用它** —— 那里每帧开头都
    /// `sync_input`，「按下」自然会被下一帧带过去，「松开」留在下一帧的
    /// `pump` 里做（那里本来就有）。
    ///
    /// 返回是否真的释放了（没闩住任何东西时返回 `false`）。
    pub fn release_now(&mut self, activation: &mut ActivationState) -> bool {
        if self.latched.take().is_some() {
            activation.key_up();
            true
        } else {
            false
        }
    }

    /// 每帧调一次：释放上一帧的闩，并把排队的动作变成一次「键盘激活」。
    ///
    /// # 调用时机
    ///
    /// **必须在宿主的 `sync_input` 之后调用。**
    ///
    /// `sync_input` 把 `ActivationState::held` 报给应用，于是本帧闩住的东西
    /// 要到**下一帧**的 `sync_input` 才被应用看见；而松开要在再下一帧才被看见
    /// —— 那正好凑成应用认 click 的「`Some` → `None`」两拍。
    ///
    /// 早一帧（`sync_input` 之前）会让闩住的那一拍与松开的那一拍落在同一帧，
    /// 应用便看不到中间的 `Some` —— 点击直接消失，且画面上完全看不出异常。
    ///
    /// # 返回值
    ///
    /// 见 [`Pumped`]。
    pub fn pump(
        &mut self,
        root: &mut Node,
        focus: &mut FocusState,
        activation: &mut ActivationState,
        ctx: Context<'_>,
    ) -> Pumped {
        // 1. 释放上一帧的闩。
        //
        // 这一帧**只**做释放：不施加新动作。理由见模块文档「排队而不是丢」。
        if self.latched.take().is_some() {
            activation.key_up();
            return Pumped {
                applied: Vec::new(),
                pressed_changed: true,
            };
        }

        // 2. 取新动作（只有队列空时才去全局队列拉 —— 上一帧没取完的先取完）。
        if self.queue.is_empty() {
            self.queue.extend(crate::take_actions());
        }
        let Some(action) = self.queue.pop_front() else {
            return Pumped::default();
        };

        // 3. 施加。
        let (applied, pressed_changed) = self.apply(&action, root, focus, activation, ctx);
        Pumped {
            applied: if applied { vec![action] } else { Vec::new() },
            pressed_changed,
        }
    }

    /// 施加一个动作。返回 `(是否生效, pressed 通道是否变了)`。
    fn apply(
        &mut self,
        action: &Action,
        root: &mut Node,
        focus: &mut FocusState,
        activation: &mut ActivationState,
        ctx: Context<'_>,
    ) -> (bool, bool) {
        match action {
            Action::Focus(key) => {
                let Some(elem_key) = element_key_of(root, key) else {
                    return (false, false);
                };
                focus.focus(root, Some(&elem_key), ctx);
                // 只移焦点 —— `pressed` 通道没动。
                (true, false)
            }
            Action::Toggle(key) | Action::Invoke(key) => {
                let Some(elem_key) = element_key_of(root, key) else {
                    return (false, false);
                };
                // 先聚焦：屏读器的行为就是「先聚焦到它，再激活它」。
                //
                // 这一步顺带把 `ACTIVATABLE` / `DISABLED` 的判据交给 core ——
                // 禁用的复选框在这里就闩不住，与键盘路径完全一致。
                focus.focus(root, Some(&elem_key), ctx);
                // 再闩住：这就是「按下」。
                if activation.key_down(root, Some(&elem_key), ctx) {
                    self.latched = Some(elem_key);
                    (true, true)
                } else {
                    (false, false)
                }
            }
        }
    }
}

/// 按**名字**找节点的 key。
///
/// # 为什么不能直接用 core 的 `Node::find`
///
/// core 的 `find` 吃 [`ElementKey`]，而 [`ElementKey::Name`] 只收 `&'static str`。
/// 无障碍侧拿到的是**运行期**的 `String`（UIA 的 AutomationId 是字符串），
/// 没法直接构造出一个能与之相等的 `ElementKey`。
///
/// 于是这里在树里按名字找，找到后**取回树里那个 `ElementKey` 本身** ——
/// 它是 intern 过的 `&'static str`（见 `zypui` 的 `Interner`），克隆它不花钱，
/// 而且交给 core 的仍是它原本的身份。
fn element_key_of(root: &Node, name: &str) -> Option<ElementKey> {
    fn visit(node: &Node, name: &str) -> Option<ElementKey> {
        if node.key().and_then(|k| k.as_name()) == Some(name) {
            return node.key().cloned();
        }
        for child in node.children() {
            if let Some(found) = visit(child, name) {
                return Some(found);
            }
        }
        None
    }
    visit(root, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zephyra_core::context::Context;
    use zephyra_core::element::Element as Elem;
    use zephyra_core::id::NodeTag;
    use zephyra_core::props;
    use zephyra_core::theme::Theme;

    fn ctx() -> Context<'static> {
        Context::new(Box::leak(Box::new(Theme::dark())))
    }

    /// 一个可用的复选框。
    fn checkbox(disabled: bool) -> Elem {
        let el = Elem::new(NodeTag::Control)
            .style("check-box")
            .with(
                &props::FOCUSABLE,
                zephyra_core::property::PropertyValue::Bool(true),
            )
            .with(
                &props::ACTIVATABLE,
                zephyra_core::property::PropertyValue::Bool(true),
            );
        if disabled {
            el.with(
                &props::DISABLED,
                zephyra_core::property::PropertyValue::Bool(true),
            )
        } else {
            el
        }
    }

    /// 一棵有两个复选框的树：一个可用、一个禁用。
    ///
    /// 属性是**声明**在元素上的 —— `ACTIVATABLE` / `DISABLED` 与
    /// `FOCUSABLE` 一样由控件模板写，宿主与 core 都只读。
    fn tree() -> Node {
        let root = Elem::new(NodeTag::Container).with_children([
            checkbox(false).with_key("check:alpha"),
            checkbox(true).with_key("check:locked"),
        ]);
        Node::from_element(&root)
    }

    #[test]
    fn toggle_latches_then_releases() {
        let _g = crate::test_lock::guard();
        let mut root = tree();
        let mut focus = FocusState::new();
        let mut activation = ActivationState::new();
        let mut bridge = Bridge::new();

        crate::push_action(Action::Toggle("check:alpha".into()));
        let pumped = bridge.pump(&mut root, &mut focus, &mut activation, ctx());
        assert_eq!(pumped.applied.len(), 1, "Toggle 应当被施加");
        assert!(pumped.pressed_changed, "闩住时 pressed 通道变了");
        assert_eq!(
            activation.held().and_then(|k| k.as_name()),
            Some("check:alpha"),
            "应当闩住那个复选框"
        );

        // 下一帧：只释放。
        let pumped = bridge.pump(&mut root, &mut focus, &mut activation, ctx());
        assert!(pumped.applied.is_empty(), "释放那一帧不施加新动作");
        assert!(pumped.pressed_changed, "松开时 pressed 通道也变了");
        assert!(activation.held().is_none(), "应当松开");
    }

    #[test]
    fn disabled_elements_are_not_activated() {
        let _g = crate::test_lock::guard();
        let mut root = tree();
        let mut focus = FocusState::new();
        let mut activation = ActivationState::new();
        let mut bridge = Bridge::new();

        crate::push_action(Action::Toggle("check:locked".into()));
        let pumped = bridge.pump(&mut root, &mut focus, &mut activation, ctx());
        assert!(pumped.applied.is_empty(), "禁用项不该被激活");
        assert!(activation.held().is_none());
    }

    #[test]
    fn a_missing_element_is_dropped() {
        let _g = crate::test_lock::guard();
        let mut root = tree();
        let mut focus = FocusState::new();
        let mut activation = ActivationState::new();
        let mut bridge = Bridge::new();

        crate::push_action(Action::Toggle("check:gone".into()));
        let pumped = bridge.pump(&mut root, &mut focus, &mut activation, ctx());
        assert!(pumped.applied.is_empty());
        assert!(!pumped.pressed_changed);
        assert!(activation.held().is_none());
    }

    #[test]
    fn focus_only_moves_focus() {
        let _g = crate::test_lock::guard();
        let mut root = tree();
        let mut focus = FocusState::new();
        let mut activation = ActivationState::new();
        let mut bridge = Bridge::new();

        crate::push_action(Action::Focus("check:alpha".into()));
        let pumped = bridge.pump(&mut root, &mut focus, &mut activation, ctx());
        assert_eq!(pumped.applied.len(), 1);
        assert!(!pumped.pressed_changed, "只移焦点不该动 pressed 通道");
        assert_eq!(
            focus.focused().and_then(|k| k.as_name()),
            Some("check:alpha")
        );
        assert!(activation.held().is_none(), "只移焦点不该顺便激活");
    }

    /// 第二个动作必须**排队**，不能把第一次的「松手」吃掉。
    #[test]
    fn a_second_action_waits_for_the_release() {
        let _g = crate::test_lock::guard();
        let mut root = tree();
        let mut focus = FocusState::new();
        let mut activation = ActivationState::new();
        let mut bridge = Bridge::new();

        crate::push_action(Action::Toggle("check:alpha".into()));
        crate::push_action(Action::Toggle("check:alpha".into()));

        assert_eq!(
            bridge
                .pump(&mut root, &mut focus, &mut activation, ctx())
                .applied
                .len(),
            1
        );
        // 闩住的那一帧：只释放，不施加第二个。
        assert!(bridge
            .pump(&mut root, &mut focus, &mut activation, ctx())
            .applied
            .is_empty());
        assert!(activation.held().is_none());
        // 再下一帧：第二个才上。
        assert_eq!(
            bridge
                .pump(&mut root, &mut focus, &mut activation, ctx())
                .applied
                .len(),
            1
        );
        assert!(activation.held().is_some());
    }
}

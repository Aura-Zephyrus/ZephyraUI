//! 键盘激活状态机：焦点落在可激活控件上时，Space / Enter 触发点击。
//!
//! # 为什么这一层在核心里
//!
//! 与 [`super::focus`] 同一个理由。WinUI 把「Space/Enter 意味着点击」这条语义放在
//! 控件自身（`ButtonBase`，见 `ButtonBaseKeyProcess.h` 的
//! `KeyPress::ButtonBase::IsPress`）——因为它的控件是**有状态的 C++ 对象**，每个
//! 按钮能自己持一个 `m_bIsSpaceOrEnterKeyDown` 闩。我们的控件是**每帧重造的无状态
//! `.zypui` 模板**，没有一个 Button 实例能持有这个闩。所以闩必须放进一个跨帧的核心
//! 状态机，与 [`super::focus`]、[`super::router`] 同级。
//!
//! 若把激活逻辑丢给每个应用自己写，行为必然不一致——有的应用 Space 触发、有的 Enter
//! 才触发、有的失焦后闩没清导致「回来一按空格就误点」。而激活只依赖两样东西——**树**
//! （`ACTIVATABLE` / `DISABLED` 属性）与**键盘事件**（[`super::text::KeyInput`]）——
//! 两者都在 core 里，没有任何理由把它放别处。
//!
//! # 谁持有激活状态
//!
//! **宿主**（`HostWindow`），与 [`super::focus`] 的 `FocusState` 同址。像 hover/press
//! 一样，激活闩是跨帧的全局事实。宿主在收到激活键边沿时**当帧**调
//! [`ActivationState::key_down`] / [`ActivationState::key_up`]（与 Tab / 指针同理：
//! 用户按空格期望立刻看到按下态，攒到下一帧 `sync_input` 才生效会慢一拍）。
//!
//! # 激活结果怎么送到应用：复用 pressed 通道
//!
//! 键盘激活**不**新开一条事件流，而是写进与指针按下**同一条** `pressed` 通道——
//! 宿主在 `sync_input` 里填 `pressed: router.pressed().or(activation.held())`。于是：
//!
//! - Space 按住期间模板出**按下态**（`press` 内置词为真）——与指针按下完全同构，
//!   正是 WinUI「两路同构」的落地。
//! - 抬起后 `pressed` 归空，应用既有的「上一帧按着、这一帧松了 = 点击」重建逻辑
//!   **自动**为键盘触发——应用零改动。
//!
//! 指针与键盘同时按住时 `.or()` 让**指针优先**；任一路抬起都不会误发点击（另一路
//! 仍在时 `pressed` 不归空）——这正是 WinUI 那个闩
//! `if (!m_bIsPointerLeftButtonDown)` 要防的误触发。
//!
//! # 与 FOCUSABLE 的关系
//!
//! [`crate::props::ACTIVATABLE`] 与 [`crate::props::FOCUSABLE`] 刻意分开：文本框同样
//! 可聚焦，可它的 Space 是**插入空格**而不是触发点击。聚焦回答「Tab 该不该停在这」，
//! 激活回答「停在这时空格/回车意味着什么」。焦点系统（[`super::focus`]）只把焦点
//! 移到 `FOCUSABLE` 节点上；本状态机只在焦点节点同时是 `ACTIVATABLE` 时才闩住。
//!
//! # DISABLED 挡住激活
//!
//! 与 WinUI `IsEnabled` 挡住每一个处理器一致：[`ActivationState::key_down`] 在节点
//! `DISABLED` 时**不**进入按下态。

use crate::context::Context;
use crate::id::ElementKey;
use crate::input::text::{Key, KeyInput};
use crate::node::Node;

/// 键盘激活状态机。
///
/// 跨帧持有，宿主每帧在键盘边沿上调用。内部只记一件事：**当前是否有一个节点正被
/// 键盘「按住」**（等价一次指针按下）。这个 `held` 就是宿主并进 `pressed` 通道的那一路。
#[derive(Clone, Default)]
pub struct ActivationState {
    /// 当前被键盘按住而处于按下态的节点。
    held: Option<ElementKey>,
}

impl ActivationState {
    /// 新建（无键盘按下）。
    pub fn new() -> Self {
        Self { held: None }
    }

    /// 当前被键盘按住而「按下」的节点 —— 宿主把它并进 `pressed` 通道。
    pub fn held(&self) -> Option<&ElementKey> {
        self.held.as_ref()
    }

    /// `key` 是不是**激活键**（Space / Enter）。
    ///
    /// WinUI 的 `KeyPress::ButtonBase::IsPress` 认 Space、Enter、GamepadA 三个；本仓
    /// 先做键盘的两个，手柄留到有输入映射时再补。
    pub fn is_activation_key(key: Key) -> bool {
        matches!(key, Key::Space | Key::Enter)
    }

    /// 处理一次激活键的**按下**边沿。
    ///
    /// 仅当 `focused` 指向树里一个 `ACTIVATABLE` 且**非** `DISABLED` 的节点时进入
    /// 按下态（闩住 `held`）；否则什么都不做、返回 `false`。
    ///
    /// 返回是否进入了按下态（宿主据此 `dirty = true` 触发重绘）。
    pub fn key_down(
        &mut self,
        root: &Node,
        focused: Option<&ElementKey>,
        ctx: Context<'_>,
    ) -> bool {
        let target = match focused {
            Some(t) if is_activatable(root, t, ctx) => t.clone(),
            // 没有焦点，或焦点节点不可激活（文本框打空格）——不闩。
            _ => return false,
        };
        if self.held.as_ref() == Some(&target) {
            return false; // 自动重复的按下不改变状态。
        }
        self.held = Some(target);
        true
    }

    /// 处理一次激活键的**抬起**边沿。
    ///
    /// 无论 `held` 是否指向该键都清空——抬起激活键就该退出键盘按下态。返回是否
    /// 退出了按下态（宿主据此决定要不要重绘，以及应用下帧看到 `pressed` 归空）。
    pub fn key_up(&mut self) -> bool {
        self.held.take().is_some()
    }

    /// 清空闩（窗口失焦 / 焦点被清时用）。
    ///
    /// 与 WinUI `ButtonBase::OnLostFocus` 清 `m_bIsSpaceOrEnterKeyDown` 一致：失焦时
    /// 若不清闩，回来后 `held` 还指着旧节点，一按空格就误触发一次「还按着」的假态。
    pub fn clear(&mut self) -> bool {
        self.key_up()
    }
}

/// 树里 key 等于 `target` 的节点是否 `ACTIVATABLE` 且非 `DISABLED`。
fn is_activatable(root: &Node, target: &ElementKey, ctx: Context<'_>) -> bool {
    match find_node(root, target) {
        Some(n) => n.is_activatable(ctx) && !n.is_disabled(ctx),
        None => false,
    }
}

/// 按 key 找节点（只读）。
fn find_node<'a>(node: &'a Node, key: &ElementKey) -> Option<&'a Node> {
    if node.key() == Some(key) {
        return Some(node);
    }
    for child in node.children() {
        if let Some(found) = find_node(child, key) {
            return Some(found);
        }
    }
    None
}

/// [`KeyInput`] → 是否为激活键按下/抬起边沿的辅助（宿主不必逐个 match）。
impl ActivationState {
    /// 处理一个 [`KeyInput`] 边沿：激活键按下 → [`Self::key_down`]，抬起 → [`Self::key_up`]。
    ///
    /// 非激活键返回 `None`（宿主不做任何事）。返回 `Some(changed)` 表示这是激活键、
    /// `changed` 是状态有没有变。这样宿主的键盘分支只剩一行。
    ///
    /// `root` 是 `Option`：抬起边沿不需要树，而按下边沿在 `None`（这帧还没树）时
    /// 视作不可激活、返回 `Some(false)`——宿主可以直接把 `cache.node()` 递进来。
    pub fn on_key(
        &mut self,
        input: &KeyInput,
        root: Option<&Node>,
        focused: Option<&ElementKey>,
        ctx: Context<'_>,
    ) -> Option<bool> {
        if !Self::is_activation_key(input.key) {
            return None;
        }
        Some(if input.pressed {
            match (root, focused) {
                (Some(root), Some(fk)) => self.key_down(root, Some(fk), ctx),
                _ => false,
            }
        } else {
            self.key_up()
        })
    }
}

#[cfg(test)]
#[path = "tests/activation_tests.rs"]
mod tests;

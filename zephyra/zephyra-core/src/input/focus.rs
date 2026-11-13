//! 焦点状态机：键盘焦点在树上的分配、移动与圈闭。
//!
//! # 为什么这一层在核心里
//!
//! 与 [`super::router`] 同一个理由：「这个键该给谁」是**框架**职责而非应用职责。
//! 每个应用各写一遍焦点管理，行为必然不一致——有的 Tab 停在按钮上、有的不圈闭、
//! 有的失焦不恢复。而焦点只依赖两样东西——**树**（`FOCUSABLE` 属性）与**键盘事件**
//! （[`super::text::TextEvent`]）——两者都在 core 里，没有任何理由把状态机放别处。
//!
//! # 谁持有焦点状态
//!
//! **宿主**（`HostWindow`）。像 [`super::router`] 的 hover/press 一样，焦点是跨帧
//! 的全局事实，且必须在 `view()`（应用重造声明树）**之后**、diff（`apply`）**之前**
//! 写进保留树——否则这一帧的样式系统读不到焦点变化。
//!
//! 宿主每帧在 `sync_input` 里调 [`FocusState::focus`]（写 FOCUSED 属性到当前/上一个
//! 节点），并在收到 Tab 事件时调 [`FocusState::tab`]（移动焦点）。
//!
//! # 焦点环为什么默认不显示（`focus-visible`）
//!
//! 焦点是**键盘**概念：鼠标点一下开关不该多出一圈蓝边（桌面应用的通行做法，
//! 也是 `focus_ring` 令牌文档里「键盘用户看不到焦点位置」的本意）。于是状态机
//! 区分「谁有焦点」（[`FocusState::focused`]，鼠标/键盘都设）与「焦点环可见」
//! （[`FocusState::is_visible`]，只有 Tab 才置真）。宿主把后者连同焦点一起报告
//! 给应用，应用的 `focused` 分类名因此只对键盘来源生效；Space / Enter 与
//! 无障碍读的仍是前者，点击后照样能键盘激活。
//!
//! # Tab 顺序怎么决定
//!
//! **树深度优先遍历**（从左到右），收集 `FOCUSABLE=true` 的节点，顺序即文档序——
//! 与 DOM 的 `tabindex=0` 行为一致。控件的视觉顺序就是键盘顺序，不需要在 `.zypui`
//! 里另加 `tabIndex` 属性；想要不同顺序时改控件结构即可。
//!
//! # FOCUSED 怎么写进树
//!
//! 通过 [`crate::props::FOCUSED`] 属性，走 [`crate::node::Node::set_prop`]
//! （**Local 层**）。为什么不是 `set_styled_prop`：`apply` 每帧会把样式表规则重写
//! 一遍并 `retain_styled_props` 拆掉「本轮没人声明的样式值」——焦点是状态机写的、
//! 不是样式表声明的，写进 Styled 层会被当垃圾拆掉。Local 层 `apply` 不动。
//!
//! 代价：local prop 写进去不会自动清，所以状态机必须**显式**把 `false` 写回
//! 上一个节点（`apply` 的职责）。

use crate::context::Context;
use crate::id::ElementKey;
use crate::node::Node;
use crate::property::PropertyValue;
use crate::props::FOCUSED;

/// 焦点状态机。
///
/// 跨帧持有，宿主每帧调用。内部记着「当前焦点节点」与「上一次写过的节点」——
/// 后者用来在焦点移动时把 `false` 写回旧节点（local prop 不会自动清）。
#[derive(Clone)]
pub struct FocusState {
    /// 当前拥有焦点的节点。
    focused: Option<ElementKey>,
    /// 上一次调用 `apply` 时写过 FOCUSED=true 的节点。
    ///
    /// 与 `focused` 的差别：`focused` 是**目标**（状态机认为谁该有焦点），
    /// 这个字段是**已经写进树**的（这帧的实际状态）。两者可能因为「目标节点
    /// 没找到」而短暂不一致。
    written: Option<ElementKey>,
    /// 焦点环是否可见（`focus-visible` 语义）。
    ///
    /// 鼠标点击设焦点时是 `false`（键盘焦点仍成立，只是**不该画环**——点了
    /// 一下开关就多出一圈蓝边，那不是用户要的反馈）；Tab 移动焦点时是 `true`
    /// （键盘用户看不到焦点在哪，必须画环）。两个后端消费它的方式不同：
    /// 激活 / 无障碍读 [`Self::focused`]（永远真实），视觉读 [`Self::is_visible`]。
    visible: bool,
    /// 焦点变化版本号。宿主比较它决定要不要重绘。
    revision: u64,
}

impl Default for FocusState {
    fn default() -> Self {
        Self::new()
    }
}

impl FocusState {
    /// 新建（无焦点）。
    pub fn new() -> Self {
        Self {
            focused: None,
            written: None,
            visible: false,
            revision: 0,
        }
    }

    /// 当前拥有焦点的节点（目标，不保证已写进树）。
    pub fn focused(&self) -> Option<&ElementKey> {
        self.focused.as_ref()
    }

    /// 焦点环是否该画（`focus-visible`）。
    ///
    /// 只在焦点由**键盘**引入（Tab）时为真；鼠标点击设焦点时为假 —— 焦点
    /// 本身成立（Space / Enter / 无障碍照常工作），但视觉上不画焦点环。
    /// 宿主把 [`Self::focused`] 与它一起报告给应用，应用据此决定要不要发
    /// `focused` 分类名。
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// 焦点状态的版本号。
    ///
    /// 任何一次「谁有焦点」的变化都会让它递增。宿主在每帧末尾比较新旧值
    /// 即可决定是否需要重绘——不必自己追踪状态。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 把焦点直接设到某个节点（鼠标点击时用）。
    ///
    /// `target` 可以是任意 key；若树里没有这个可聚焦节点，**焦点清空**而不是
    /// 保持原状——用户点了一个不可聚焦的地方，焦点就应该让给那个点（通常随后
    /// 由 Tab 重新进入）。这符合「点击空白处取消焦点」的常见交互。
    ///
    /// 鼠标设焦点**不显示焦点环**（`focus-visible` 语义，见 [`Self::is_visible`]）。
    ///
    /// 返回是否发生了变化。
    pub fn focus(
        &mut self,
        root: &mut Node,
        target: Option<&ElementKey>,
        ctx: Context<'_>,
    ) -> bool {
        // `target` 是借用；要存进 `self.focused`（拥有所有权的 ElementKey）必须 cloned。
        // 同时要判断树里有没有这个可聚焦节点 —— 没有就是「点了空白处」，清空焦点。
        let focused = match target {
            Some(t) if find_focusable(root, t, ctx) => Some(t.clone()),
            Some(_) => None, // 点了不可聚焦的地方 = 取消焦点
            None => None,
        };
        let mut changed = false;
        if focused != self.focused {
            self.focused = focused;
            changed = true;
        }
        // 鼠标点击即便落在**同一个**已聚焦节点上，也要把环收掉（用户此刻在
        // 用鼠标，键盘环不该出来）。可见性变化同样算一次状态变化。
        if self.visible {
            self.visible = false;
            changed = true;
        }
        if changed {
            self.revision += 1;
        } else if self.written == self.focused {
            // 目标与可见性都没变，且上次已写进树：无事可做。
            return false;
        }
        self.write_state(root);
        true
    }

    /// 按 Tab 移动焦点。
    ///
    /// `shift=true` 反向。没有焦点时 Tab 聚焦第一个、Shift+Tab 聚焦最后一个。
    /// 到达末端后**圈闭**到另一端（循环）。没有可聚焦节点时保持原状。
    ///
    /// 返回是否发生了变化（焦点移动了）。
    pub fn tab(&mut self, root: &mut Node, shift: bool, ctx: Context<'_>) -> bool {
        let order = collect_focusable(root, ctx);
        if order.is_empty() {
            return false;
        }
        let next = match &self.focused {
            Some(cur) => {
                let pos = order.iter().position(|k| k == cur);
                match pos {
                    Some(i) if !shift => order[(i + 1) % order.len()].clone(),
                    Some(i) if shift => order[(i + order.len() - 1) % order.len()].clone(),
                    // 焦点节点不在本棵树的可聚焦列表里（树结构变了）：
                    // 从首/尾重新进。
                    Some(_) if !shift => order[0].clone(),
                    Some(_) => order[order.len() - 1].clone(),
                    None => order[0].clone(),
                }
            }
            None if shift => order[order.len() - 1].clone(),
            None => order[0].clone(),
        };
        self.focused = Some(next);
        // 键盘引入的焦点 → 显示焦点环。
        self.visible = true;
        self.revision += 1;
        self.write_state(root);
        true
    }

    /// 清空焦点（窗口失焦时用）。
    ///
    /// 把 `false` 写回当前节点，并清空内部状态——否则失焦后再点回来，
    /// 旧焦点会一直写着 true。
    pub fn clear(&mut self, root: Option<&mut Node>) -> bool {
        if self.focused.is_none() && self.written.is_none() && !self.visible {
            return false;
        }
        self.focused = None;
        self.visible = false;
        self.revision += 1;
        if let Some(root) = root {
            self.write_state(root);
        } else {
            self.written = None;
        }
        true
    }

    /// 把 FOCUSED=true 写到 `focused`、false 写到上一个 `written` 节点。
    ///
    /// # 为什么要「先清旧、再设新」
    ///
    /// local prop 不会自动清（见模块文档）。若只设新焦点、不清旧节点，
    /// 焦点移动后旧节点会一直显示焦点环——那是「焦点移走了还在高亮」的经典 bug。
    ///
    /// 不需要 `ctx`：`set_prop` 是写 Local 层属性，不解析资源键、不查样式。
    fn write_state(&mut self, root: &mut Node) {
        // 先清掉上一次写的节点（若它还在这棵树里且不是新焦点）。
        if let Some(old) = self.written.take() {
            if self.focused.as_ref() != Some(&old) {
                if let Some(n) = find_node_mut(root, &old) {
                    n.set_prop(&FOCUSED, PropertyValue::bool(false));
                }
            }
        }
        // 再设新焦点。
        if let Some(new) = &self.focused {
            if let Some(n) = find_node_mut(root, new) {
                n.set_prop(&FOCUSED, PropertyValue::bool(true));
                self.written = Some(new.clone());
            }
            // 目标节点不在树里：不设 written，下次 apply 时旧 written 已被清、
            // 新 written 为空——状态自洽。
        }
    }
}

/// 收集树里所有可聚焦节点的 key（深度优先，文档序）。
///
/// # 为什么是深拷贝的 `Vec<ElementKey>`
///
/// Tab 键是**用户节奏**（每秒个位数），不在每帧热路径上；为它分配一个 Vec
/// 完全可接受。若将来有几十个焦点节点的窗口想要零分配，可以改成递归传
/// `&mut Vec`——但那要以「不可变借用树 + 可变借用 vec」为代价，现在的简单版
/// 更清楚。
pub fn collect_focusable(node: &Node, ctx: Context<'_>) -> Vec<ElementKey> {
    let mut out = Vec::new();
    fn visit(n: &Node, ctx: Context<'_>, out: &mut Vec<ElementKey>) {
        // 折叠节点不参与（与 hit_test 同一判据：折叠既占位也不绘制）。
        if n.is_collapsed(ctx) {
            return;
        }
        if n.is_focusable(ctx) {
            if let Some(key) = n.key() {
                out.push(key.clone());
            }
        }
        for child in n.children() {
            visit(child, ctx, out);
        }
    }
    visit(node, ctx, &mut out);
    out
}

/// 树里是否有 key 等于 `target` 的可聚焦节点。
fn find_focusable(node: &Node, target: &ElementKey, ctx: Context<'_>) -> bool {
    collect_focusable(node, ctx).iter().any(|k| k == target)
}

/// 按 key 找节点（可变）。
fn find_node_mut<'a>(node: &'a mut Node, key: &ElementKey) -> Option<&'a mut Node> {
    if node.key() == Some(key) {
        return Some(node);
    }
    for child in node.children_mut() {
        if let Some(found) = find_node_mut(child, key) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
#[path = "../../tests/unit/focus.rs"]
mod tests;

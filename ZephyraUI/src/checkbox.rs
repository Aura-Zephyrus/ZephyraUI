//! 复选框的状态模型 —— 控件层里**唯一**知道「复选框状态怎么变」的地方。
//!
//! # 为什么状态住在 Rust 侧而不是 `.zypui`
//!
//! `.zypui` 是**每帧重建**的无状态模板：它没有「实例」，也就没有地方存
//! 「这个框现在是选中还是半选」。状态只能是宿主的数据，而「状态怎么变」
//! （点一下从哪到哪、组的全选/半选怎么算）是**规则**，不是数据 ——
//! 规则必须有一份权威实现，否则每个 app 各写一遍、各错一遍。
//!
//! 所以分工是：
//!
//! | 关注 | 住在哪 |
//! |---|---|
//! | 状态**值**与流转**规则** | 本模块（Rust） |
//! | 状态**长什么样**（节点树、分类名） | `ui/controls.zypui` |
//! | 状态的**颜色与尺寸** | `controls-checkbox.zstyle` |
//! | 状态**存在哪**（跨帧留存） | 宿主应用 |
//!
//! # 三态与两态
//!
//! [`CheckState`] 有三个值，但**两态复选框只会用到其中两个** ——
//! `Unchecked` 与 `Checked`。`Indeterminate` 是**派生态**：它不由用户直接
//! 摆出来，而是「组里选了一部分」这件事的表示。
//!
//! 这也是为什么「三态」在结构上是个**开关**（`.zypui` 里的 `tristate`
//! 参数）而不是状态值的一半：两态框压根不该出现横杠节点。
//!
//! # 流转规则
//!
//! ```text
//! Unchecked ──点──▶ Checked ──点──▶ Unchecked
//!      ▲                                 │
//!      └─────────────────────────────────┘
//!
//! Indeterminate ──点──▶ Checked
//! ```
//!
//! 半选点一下是**全选**（不是清空）—— 与 WinUI / HTML / macOS 一致。
//! 理由：半选是「有选择但不全」的**派生态**，用户点它时表达的是「我要全要」；
//! 若点它变成全不选，用户就得点两次才能全选，代价与意图相反。

/// 复选框的状态分类词 —— **选中**。
///
/// 与 `controls-checkbox.zstyle` 里的 `.checked` 规则同名。
/// 两处必须同改（样式表拼错分类名是**静默失效**：画面上只是没那个态）。
pub const CLASS_CHECKED: &str = "checked";

/// 复选框的状态分类词 —— **半选**。
///
/// 与 `controls-checkbox.zstyle` 里的 `.indeterminate` 规则同名。
pub const CLASS_INDETERMINATE: &str = "indeterminate";

/// 复选框的三态。
///
/// # 为什么是三态而不是「bool + 半个 bool」
///
/// 半选不是一个能用 bool 表达的状态（它是第三值），也不是「未选」的同义词。
/// 用两个 bool 拼出来的话，「两者同时为真」就是一个**能表达但不该存在**的
/// 状态 —— 而它一旦存在，画面上就会出现「既勾又横」的框，且没有任何报错。
/// 枚举把这个非法状态从类型上删掉。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum CheckState {
    /// 未选中。
    #[default]
    Unchecked,
    /// 已选中。
    Checked,
    /// 半选（部分选中）。
    Indeterminate,
}

impl CheckState {
    /// 这个状态对应的**分类词** —— 直接发给 `.zypui` 的 `state` 实参。
    ///
    /// 空串 = 不发分类名（`.zypui` 里空串的动态分类词不发，见
    /// `zypui::instantiate` 的 `ClassTok::Dyn` 分支）。所以未选中就是
    /// 「不带任何状态类」，样式表管它叫基础态。
    ///
    /// # 为什么由 Rust 产出这个词
    ///
    /// 分类名是**样式表的词汇**，而样式表与状态模型同属控件层。让模型
    /// 产出这个词，模板就退化成一个不动的管道 —— 模板不需要知道有哪些状态，
    /// 也就不可能与样式表对不上。
    pub const fn class_word(self) -> &'static str {
        match self {
            CheckState::Unchecked => "",
            CheckState::Checked => CLASS_CHECKED,
            CheckState::Indeterminate => CLASS_INDETERMINATE,
        }
    }

    /// 点一下之后的状态。
    ///
    /// - `Unchecked` → `Checked`
    /// - `Checked` → `Unchecked`
    /// - `Indeterminate` → `Checked`（半选点一下 = 全选，理由见模块文档）
    pub const fn toggled(self) -> CheckState {
        match self {
            CheckState::Unchecked => CheckState::Checked,
            CheckState::Checked => CheckState::Unchecked,
            CheckState::Indeterminate => CheckState::Checked,
        }
    }

    /// 是否已选中。
    pub const fn is_checked(self) -> bool {
        matches!(self, CheckState::Checked)
    }

    /// 是否半选。
    pub const fn is_indeterminate(self) -> bool {
        matches!(self, CheckState::Indeterminate)
    }
}

/// 把一组子项状态**聚合成**一个组状态。
///
/// 规则（与组头的三态一一对应）：
///
/// - 一个子项都没有 → [`CheckState::Unchecked`]（空组没有「全选」可言）；
/// - 有子项处于半选 → [`CheckState::Indeterminate`]（半选向上冒泡）；
/// - 全部选中 → [`CheckState::Checked`]；
/// - 其余（选了部分）→ [`CheckState::Indeterminate`]。
///
/// # 为什么空组是「未选中」而不是「已选中」
///
/// 「空集是全选的子集」在数学上成立，但在**界面**上会骗人：空组没有子项，
/// 点组头也没有任何东西可全选。若组头显示成已选中，用户会以为「都选了」，
/// 而实际上一个都没有。空组显示成未选中，点它也**无副作用**。
pub fn aggregate(states: &[CheckState]) -> CheckState {
    if states.is_empty() {
        return CheckState::Unchecked;
    }
    let mut checked = 0usize;
    let mut has_indeterminate = false;
    for s in states {
        match s {
            CheckState::Checked => checked += 1,
            CheckState::Indeterminate => has_indeterminate = true,
            CheckState::Unchecked => {}
        }
    }
    if has_indeterminate {
        return CheckState::Indeterminate;
    }
    if checked == states.len() {
        CheckState::Checked
    } else if checked == 0 {
        // 一个都没选 = 全不选，**不是**半选。半选的含义是「选了一部分」，
        // 「一部分」不能是空集。
        CheckState::Unchecked
    } else {
        CheckState::Indeterminate
    }
}

/// 一个复选框。
///
/// 存的是**状态**，不是外观 —— 外观在 `.zstyle`，结构在 `.zypui`。
/// 它存在的意义是让宿主把「这个框的 key、标签、状态」放在一起，
/// 于是点击分派只需按 key 找到它，不必在宿主里维护平行的 `Vec<bool>`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkbox {
    key: &'static str,
    label: &'static str,
    state: CheckState,
    disabled: bool,
}

impl Checkbox {
    /// 新建一个未选中的复选框。
    ///
    /// `key` 是**事件路由身份**（宿主按它把点击分派回来），`label` 是显示
    /// 文本（应用自己的文案表给，控件不翻译）。
    pub fn new(key: &'static str, label: &'static str) -> Self {
        Self {
            key,
            label,
            state: CheckState::Unchecked,
            disabled: false,
        }
    }

    /// 指定初始状态。
    pub fn with_state(mut self, state: CheckState) -> Self {
        self.state = state;
        self
    }

    /// 设为禁用。
    ///
    /// 禁用的框不响应点击，也不参与键盘激活 —— core 的命中测试与
    /// `ActivationState` 都认 `props::DISABLED`，样式表则把它画成灰的。
    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 直接改禁用态（`push` 之后链式设置用不了 `with_*` 时用它）。
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
    }

    /// 事件路由身份。
    pub fn key(&self) -> &'static str {
        self.key
    }

    /// 显示文本。
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// 当前状态。
    pub fn state(&self) -> CheckState {
        self.state
    }

    /// 直接改状态（不做流转）。
    pub fn set_state(&mut self, state: CheckState) {
        self.state = state;
    }

    /// 按 [`CheckState::toggled`] 流转一次。
    ///
    /// 禁用的框**不流转** —— 禁用态下界面不该有任何反应。
    pub fn toggle(&mut self) {
        if self.disabled {
            return;
        }
        self.state = self.state.toggled();
    }

    /// 是否禁用。
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// 当前状态的分类词（给 `.zypui` 的 `state` 实参）。
    pub fn class_word(&self) -> &'static str {
        self.state.class_word()
    }
}

/// 一个复选框组：**一个三态的组头 + 一层平铺的两态子项**。
///
/// # 规则
///
/// - **组头**的状态**永远派生**于子项，不单独存 —— 存了就会出现两份真相
///   （子项变了、组头忘了改），而那种不一致在画面上完全看不出来；
/// - 点子项**只改自己**，组头随之重算；
/// - 点组头 = **全选 / 全部不选**：当前全选 → 全部不选，其余（全不选或半选）
///   → 全选。半选时点组头是「全选」，与 [`CheckState::toggled`] 的半选规则一致；
/// - 子项**只有两态**。半选是组头的性质，不是子项的性质 —— 所以组是
///   **平铺一层**的，不存在「组里再套组」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckboxGroup {
    header: Checkbox,
    items: Vec<Checkbox>,
}

impl CheckboxGroup {
    /// 新建一个组。组头初始未选中（组头状态永远派生，不单独设）。
    pub fn new(header_key: &'static str, header_label: &'static str) -> Self {
        Self {
            header: Checkbox::new(header_key, header_label),
            items: Vec::new(),
        }
    }

    /// 追加一个子项，返回它以便继续设置（例如初始状态、禁用）。
    pub fn push(&mut self, key: &'static str, label: &'static str) -> &mut Checkbox {
        self.items.push(Checkbox::new(key, label));
        self.items.last_mut().expect("刚 push 进去的子项必然存在")
    }

    /// 追加一个子项并返回整个组（链式用法）。
    pub fn with_item(mut self, key: &'static str, label: &'static str) -> Self {
        self.push(key, label);
        self
    }

    /// 按 key 找子项。
    pub fn item(&self, key: &str) -> Option<&Checkbox> {
        self.items.iter().find(|i| i.key() == key)
    }

    /// 按 key 找子项（可变）。
    pub fn item_mut(&mut self, key: &str) -> Option<&mut Checkbox> {
        self.items.iter_mut().find(|i| i.key() == key)
    }

    /// 切换一个子项 —— **只改它自己**。
    ///
    /// 返回是否真的切了：key 不存在或子项被禁用时返回 `false`。
    pub fn toggle_item(&mut self, key: &str) -> bool {
        match self.item_mut(key) {
            Some(item) if !item.is_disabled() => {
                item.toggle();
                true
            }
            _ => false,
        }
    }

    /// 直接设置一个子项的状态。
    ///
    /// 返回是否真的改了：key 不存在或子项被禁用时返回 `false`。
    pub fn set_item_state(&mut self, key: &str, state: CheckState) -> bool {
        match self.item_mut(key) {
            Some(item) if !item.is_disabled() => {
                item.set_state(state);
                true
            }
            _ => false,
        }
    }

    /// 全选 / 全部不选。
    ///
    /// 当前组头是「全选」→ 全部不选；否则（全不选或半选）→ 全选。
    ///
    /// **被禁用的子项不受影响** —— 禁用的框不该被「全选」捎带改掉。
    /// 空的组是**无副作用**的（没有子项可改，组头也因此永远是未选中）。
    pub fn toggle_all(&mut self) {
        let target = if self.header_state().is_checked() {
            CheckState::Unchecked
        } else {
            CheckState::Checked
        };
        for item in &mut self.items {
            if !item.is_disabled() {
                item.set_state(target);
            }
        }
    }

    /// 组头（未选中 / 选中 / 半选）—— **派生值**，不存。
    ///
    /// # 为什么禁用子项**不参与**聚合
    ///
    /// 组头是「全选 / 全部不选」那个控件，它管的是**可切换**的那批子项。
    /// 若把禁用的子项也算进聚合，就会出现一个**卡死**的组头：
    ///
    /// ```text
    /// 3 个子项，其中 1 个禁用且未选
    ///   → 聚合：2 选 1（半选）
    ///   → 点组头：全选（跳过禁用项）→ 那 2 个本来就选了，**什么都没变**
    ///   → 组头永远是半选，点它永远没反应
    /// ```
    ///
    /// 那个「点了没反应」在画面上完全看不出来 —— 画面只是一颗不变的头。
    /// 把禁用项排除在聚合之外，组头就只反映可切换的那批，「全选」也就
    /// 一定能走到「全选」态。
    ///
    /// 代价是「组头全选」时那一行禁用项仍是未选 —— 但它是**禁用**的
    /// （画成灰的、点不动），用户看得出它不在这一批里。
    pub fn header_state(&self) -> CheckState {
        let states: Vec<CheckState> = self
            .items
            .iter()
            .filter(|i| !i.is_disabled())
            .map(|i| i.state())
            .collect();
        aggregate(&states)
    }

    /// 组头控件本体（要拿它的 key / label 时用）。
    pub fn header(&self) -> &Checkbox {
        &self.header
    }

    /// 组头控件本体（可变）。改它的 `state` **不会**影响派生出来的组状态 ——
    /// 组状态永远从子项算；要改组状态请改子项或调 [`Self::toggle_all`]。
    pub fn header_mut(&mut self) -> &mut Checkbox {
        &mut self.header
    }

    /// 全部子项。
    pub fn items(&self) -> &[Checkbox] {
        &self.items
    }

    /// 子项个数。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 组里一个子项都没有吗。
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
#[path = "tests/checkbox_tests.rs"]
mod tests;

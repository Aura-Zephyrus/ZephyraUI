//! 无障碍树的**数据模型** —— 平台中立（COM 那一半在 `provider.rs`）。
//!
//! # 为什么模型要独立于 COM
//!
//! UIA 的接口全是 COM：`VARIANT`、`SAFEARRAY`、`HRESULT`。若直接拿 COM 类型
//! 当模型，于是「这棵树长什么样」这件事**只能在 Windows 上验证** —— 而无障碍树
//! 恰恰是最该能在任何机器上断言的东西（Linux CI 上跑不了 Windows 屏读器，
//! 于是「角色映射对不对」「名字取对没有」就只能靠人眼看）。
//!
//! 所以这里只放**纯数据**：角色、开关状态、名字、矩形。COM 那一层只是把它
//! 翻译成 UIA 认识的形状。
//!
//! # 一份快照而不是一个活的树
//!
//! UIA 的客户端会在**任意时刻**问「这个元素叫什么」「它现在勾上了吗」，而
//! 那些时刻与渲染帧无关（屏读器在后台轮询、自动化脚本在事件回调里查）。
//! 让 COM 对象回头去问渲染线程会把两个世界搅在一起 —— 渲染线程正在写树，
//! 问的人正在读，锁与生命周期都会变成 bug 温床。
//!
//! 所以：宿主每帧把无障碍树**拍成一份不可变快照**（`Snapshot`），COM 侧只读
//! 快照。读侧不需要锁（`Arc` 里换指针），写侧只有一处。

use zephyra_core::core::Rect;

/// 元素在无障碍树里的**角色**。
///
/// 角色对应 UIA 的 ControlType（`UIA_*ControlTypeId`）。屏读器念的是
/// 「复选框，已勾选」还是「单选钮，未选中」—— 全靠这个值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// 复选框（三态与两态都是它）。
    ///
    /// UIA 没有独立的「三态复选框」类型：三态复选框就是 CheckBox，只是
    /// ToggleState 多一个 `Indeterminate`。这与 WinUI / HTML 一致。
    CheckBox,
    /// 按钮。
    Button,
    /// 开关（两态：开 / 关）。
    ///
    /// # 为什么 UIA 里没有「Switch」
    ///
    /// UIA 的 ControlType 表里**没有** Switch —— 开关只能报成
    /// `UIA_CheckBoxControlTypeId` + TogglePattern。屏读器于是念「复选框，
    /// 已选定」而不是「开关，已打开」。这是平台的既有限制，不是本仓的选择；
    /// `class_name` 仍报 `Switch`，让 Inspect 里能区分两者。
    Switch,
    /// 单选钮。
    ///
    /// 状态不是 TogglePattern 而是 **SelectionItem**：单选钮的「选中」是
    /// 「在一组里挑了一个」，不是「这个开关被打开了」。两套图案在 UIA 里
    /// 的语义不同 —— 报错了屏读器会念「复选框」而不是「单选钮」。
    RadioButton,
    /// 分组容器（复选框组这类）。
    Group,
    /// 窗口根。
    Window,
}

/// 开关（Toggle）图案的状态。
///
/// 与 `zephyra_ui::CheckState` 一一对应，但**刻意不共用同一个类型**：
/// 控件层不该知道 Windows 的存在，而这里也不该知道「三态流转规则」那套
/// 业务语义 —— 它只知道「现在是关 / 开 / 不确定」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToggleState {
    /// 未选中。
    Off,
    /// 已选中。
    On,
    /// 半选（部分选中）。
    Indeterminate,
}

impl ToggleState {
    /// 这个状态在 UIA 里的数值（`UIA_ToggleState*`）。
    ///
    /// 平台中立地写死数值，是为了让本模块在 Linux 上也能断言映射关系 ——
    /// 否则「映射对不对」这件事又要等 Windows 才能验。
    pub const fn uia_value(self) -> i32 {
        match self {
            ToggleState::Off => 0,
            ToggleState::On => 1,
            ToggleState::Indeterminate => 2,
        }
    }
}

/// 无障碍树里的一个元素。
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// 元素 id（同一份快照内唯一）。
    ///
    /// **id 即下标**（见 [`Snapshot::elements`]）—— UIA 的 runtime id 与
    /// `get_ChildElementIds` 用的都是整数，稳定且可比较。
    pub id: usize,
    /// 父元素 id（`None` = 根元素）。
    pub parent: Option<usize>,
    /// 事件路由身份 —— UIA 的 Toggle / Invoke 动作最终按它回到应用。
    ///
    /// 它就是控件模板里的 `key`，与 `Gallery::click` 分派用的是**同一个**串。
    pub key: String,
    /// 可访问名 —— 屏读器念出来的那个。
    pub name: String,
    /// 角色。
    pub role: Role,
    /// 开关状态（`None` = 这个元素没有 Toggle 图案）。
    ///
    /// 单选钮也用它表示「选中了吗」（`On` = 选中），但那**不**意味着它
    /// 暴露 TogglePattern —— 单选钮暴露的是 SelectionItem，两者按 `role`
    /// 分派，见 `provider.rs` 的 `GetPatternProvider`。
    pub toggle: Option<ToggleState>,
    /// 是否可用（禁用态为 `false`）。
    pub enabled: bool,
    /// 是否有键盘焦点。
    pub focused: bool,
    /// 是否在屏幕外（滚出视口 / 被裁掉）。
    pub offscreen: bool,
    /// **屏幕坐标**下的矩形。
    ///
    /// UIA 的 `BoundingRectangle` 是屏幕坐标，而布局出来的矩形是客户区坐标 ——
    /// 两者差一个窗口偏移。宿主在发布快照前把偏移加好。
    pub rect: Rect,
    /// 子元素 id（文档序）。
    pub children: Vec<usize>,
}

/// 一份无障碍树快照。
///
/// 不可变：宿主每帧造一份新的换掉旧的，COM 侧只读。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    /// 根元素 id（恒为 `0`）。
    pub root: usize,
    /// 全部元素，**按 id 排列且 id 即下标**。
    pub elements: Vec<Element>,
}

impl Snapshot {
    /// 按 id 取元素。
    pub fn get(&self, id: usize) -> Option<&Element> {
        self.elements.get(id)
    }

    /// 按事件路由身份（控件的 `key`）取元素。
    pub fn find_by_key(&self, key: &str) -> Option<&Element> {
        self.elements.iter().find(|e| e.key == key)
    }

    /// 根元素。
    pub fn root_element(&self) -> Option<&Element> {
        self.get(self.root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_state_values_match_uia() {
        // UIA_ToggleStateOff=0 / On=1 / Indeterminate=2。
        assert_eq!(ToggleState::Off.uia_value(), 0);
        assert_eq!(ToggleState::On.uia_value(), 1);
        assert_eq!(ToggleState::Indeterminate.uia_value(), 2);
    }

    #[test]
    fn snapshot_finds_by_key() {
        let mut snap = Snapshot::default();
        snap.elements.push(Element {
            id: 0,
            parent: None,
            key: "root".into(),
            name: "window".into(),
            role: Role::Window,
            toggle: None,
            enabled: true,
            focused: false,
            offscreen: false,
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            children: vec![1],
        });
        snap.elements.push(Element {
            id: 1,
            parent: Some(0),
            key: "check:alpha".into(),
            name: "Enable feature".into(),
            role: Role::CheckBox,
            toggle: Some(ToggleState::On),
            enabled: true,
            focused: false,
            offscreen: false,
            rect: Rect::new(10.0, 20.0, 22.0, 22.0),
            children: vec![],
        });
        assert_eq!(snap.find_by_key("check:alpha").unwrap().id, 1);
        assert!(snap.find_by_key("nope").is_none());
        assert_eq!(snap.root_element().unwrap().role, Role::Window);
    }
}

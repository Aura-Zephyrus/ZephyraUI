//! 从**保留树**派生无障碍树。
//!
//! # 为什么从树派生，而不是让应用另外报一份
//!
//! 另一条路是「应用自己列一份无障碍元素清单」。它的问题是**两份真相**：应用
//! 报的那份与画出来的那份会漂移 —— 某个控件被条件渲染掉了，清单里还留着；
//! 某个控件挪了位置，矩形还是老的。漂移在画面上完全看不出来（无障碍树不是
//! 画出来的东西），只有屏幕阅读器念出一个「存在但点不到」的控件时才暴露。
//!
//! 从保留树派生就没有这个问题：**画出来的就是暴露的**。
//!
//! # 语义从哪来：分类名
//!
//! 树里没有「角色」这个属性 —— core 不该为无障碍开一个洞（那会让每个控件
//! 都得往节点上挂一个 Windows 概念）。所以角色由**分类名**推：
//!
//! | 分类名 | 角色 |
//! |---|---|
//! | `check-box` | 复选框 |
//! | `button` | 按钮 |
//! | `check-group` / `radio-group` | 分组 |
//! | `switch` | 开关 |
//! | `radio-box` | 单选钮 |
//!
//! 分类名是本仓控件的**结构标记**（`check-box` 就是复选框那个控件），不是
//! 纯装饰 —— 它决定外观，而外观与语义本来就是一回事（「长得像复选框的东西」
//! 对屏读器就该是复选框）。这张映射表住在**无障碍这一层**而不是控件层：
//! 控件层不该知道 Windows 的存在，而整仓只有这一处需要「分类名 → UIA 角色」
//! 这层翻译。
//!
//! # 可访问名从哪来：`key="label"` 的那个子节点
//!
//! 控件模板里每个控件的文本子节点都叫 `label`（`<text key="label" …>`）——
//! 这是本仓的一条既有约定。于是「这个控件叫什么」= 「它那个叫 label 的子节点
//! 的文本」，不用每个控件各报一次名字。

use zephyra_core::core::Rect;
use zephyra_core::node::Node;
use zephyra_core::props;

use crate::model::{Element, Role, Snapshot, ToggleState};

/// 分类名 → 角色。
///
/// 顺序无关（按名查表），但**第一条命中即用**。
const ROLE_BY_CLASS: &[(&str, Role)] = &[
    ("check-box", Role::CheckBox),
    ("button", Role::Button),
    ("check-group", Role::Group),
    ("radio-group", Role::Group),
    ("switch", Role::Switch),
    ("radio-box", Role::RadioButton),
];

/// 开关状态的分类名 → 状态。
///
/// 只认控件**自己**身上的分类名：`check-box` 的 `state` 实参直接落在控件的
/// class 列表里（`class="check-box {state} …"`），开关的 `on` 同样在控件自己
/// 身上（`class="switch {checked ? 'on'} …"`），所以「开关了吗」不需要下钻到
/// 子节点 —— 下钻的子节点在无障碍树里**不存在**（装饰层不留痕）。
///
/// `on` 与 `checked` 都是「已打开 / 已选中」，只是两族的词汇不同：
/// 复选框与单选钮用 `checked`，开关用 `on`。
const TOGGLE_BY_CLASS: &[(&str, ToggleState)] = &[
    ("checked", ToggleState::On),
    ("indeterminate", ToggleState::Indeterminate),
    ("on", ToggleState::On),
];

/// 窗口层面的信息 —— 宿主知道，规则不知道。
#[derive(Debug, Clone, Default)]
pub struct WindowInfo {
    /// 窗口标题（可访问名）。
    pub name: String,
    /// 客户区在**屏幕坐标**下的矩形。
    ///
    /// 布局出来的矩形是客户区坐标（原点 = 客户区左上角），UIA 要的是屏幕
    /// 坐标 —— 两者差一个 `client.x / client.y`。宿主把这偏移在这里算掉，
    /// 于是规则只管加法。
    pub client: Rect,
}

/// 从保留树派生一份无障碍树快照。
///
/// `root` 是保留树的根（对应窗口客户区），`win` 是窗口层面的信息。
pub fn build(root: &Node, win: &WindowInfo) -> Snapshot {
    let mut snap = Snapshot::default();
    // 根元素 = 窗口本身（id 0）。
    snap.elements.push(Element {
        id: 0,
        parent: None,
        key: String::new(),
        name: win.name.clone(),
        role: Role::Window,
        toggle: None,
        enabled: true,
        focused: false,
        offscreen: false,
        rect: win.client,
        children: Vec::new(),
    });
    walk(root, 0, &mut snap, win);
    snap
}

/// 深度优先走一遍树，把暴露的元素塞进快照。
fn walk(node: &Node, parent: usize, snap: &mut Snapshot, win: &WindowInfo) {
    let role = role_of(node);
    // 暴露的元素 id = 它在快照里的下标。
    let self_id = match role {
        Some(role) => {
            let id = snap.elements.len();
            snap.elements.push(element_of(node, id, parent, role, win));
            // 挂到**父**的子列表里 —— 不是挂到自己身上。
            //
            // ⚠ 这一行曾经写错过（挂到了 `child_parent`，那正是它自己）——
            // 于是每个暴露元素都成了自己的子节点，而真正的父子关系全丢了。
            // 症状极隐蔽：属性、名字、状态全都对（那些是按元素查的），
            // 只有**树形**错了 —— 屏幕阅读器念不出「复选框在一个组里」，
            // 而 Inspect 里看到的是一棵自指的怪树。
            snap.elements[parent].children.push(id);
            id
        }
        None => usize::MAX,
    };
    // 子元素的父：暴露的元素挂到**最近的暴露祖先**上。
    //
    // 「最近」而不是「直接父」—— 控件模板里到处是装饰性的包裹层（指示器、
    // 标签行、行容器），它们不该在无障碍树里留一层，但它们的子元素要挂到
    // 真正暴露的那一层去。
    let child_parent = if self_id == usize::MAX {
        parent
    } else {
        self_id
    };
    for c in node.children() {
        walk(c, child_parent, snap, win);
    }
}

/// 这个节点暴露成什么角色（`None` = 不暴露）。
fn role_of(node: &Node) -> Option<Role> {
    let names = node.style_names();
    ROLE_BY_CLASS
        .iter()
        .find(|(class, _)| names.iter().any(|n| n == class))
        .map(|(_, role)| *role)
}

/// 造一个元素。
fn element_of(node: &Node, id: usize, parent: usize, role: Role, win: &WindowInfo) -> Element {
    let toggle = match role {
        // 复选框与开关都走 TogglePattern；单选钮的「选中」在 UIA 里是
        // SelectionItem 的 IsSelected —— 同一个 `toggle` 字段装两种语义，
        // 图案按 `role` 分派（见 `provider.rs` 的 `GetPatternProvider`）。
        Role::CheckBox | Role::Switch | Role::RadioButton => Some(toggle_of(node)),
        _ => None,
    };
    Element {
        id,
        parent: Some(parent),
        key: node
            .key()
            .and_then(|k| k.as_name())
            .unwrap_or_default()
            .to_string(),
        name: accessible_name(node),
        role,
        toggle,
        enabled: !bool_prop(node, &props::DISABLED),
        focused: bool_prop(node, &props::FOCUSED),
        offscreen: false,
        rect: screen_rect(node, win),
        children: Vec::new(),
    }
}

/// 控件的开关状态（只看它自己身上的分类名）。
fn toggle_of(node: &Node) -> ToggleState {
    let names = node.style_names();
    for (class, state) in TOGGLE_BY_CLASS {
        if names.iter().any(|n| n == class) {
            return *state;
        }
    }
    ToggleState::Off
}

/// 可访问名 —— `key="label"` 的那个子节点的文本。
fn accessible_name(node: &Node) -> String {
    fn find(n: &Node) -> Option<String> {
        if n.key().and_then(|k| k.as_name()) == Some("label") {
            return text_of(n);
        }
        for c in n.children() {
            if let Some(t) = find(c) {
                return Some(t);
            }
        }
        None
    }
    find(node).unwrap_or_default()
}

/// 文本节点的文字。
fn text_of(node: &Node) -> Option<String> {
    let (value, _) = node.props().lookup(&props::TEXT)?;
    if let Some(s) = value.get::<&'static str>() {
        return Some(s.to_string());
    }
    if let Some(s) = value.get::<std::sync::Arc<str>>() {
        return Some(s.to_string());
    }
    None
}

/// 布尔属性。
fn bool_prop(node: &Node, key: &zephyra_core::property::PropertyKey) -> bool {
    node.props()
        .lookup(key)
        .and_then(|(v, _)| v.get::<bool>())
        .unwrap_or(false)
}

/// 布局矩形 → 屏幕坐标。
fn screen_rect(node: &Node, win: &WindowInfo) -> Rect {
    let r = node.arranged_rect();
    Rect::new(r.x + win.client.x, r.y + win.client.y, r.width, r.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zephyra_core::element::Element as Elem;
    use zephyra_core::id::{ElementKey, NodeTag};

    fn text_node(key: &'static str, text: &'static str) -> Elem {
        Elem::new(NodeTag::Text)
            .with_key(ElementKey::Name(key))
            .with(
                &props::TEXT,
                zephyra_core::property::PropertyValue::StaticStr(text),
            )
    }

    #[test]
    fn role_comes_from_class_names() {
        let el = Elem::new(NodeTag::Control)
            .style("check-box")
            .style("checked");
        let node = Node::from_element(&el);
        assert_eq!(role_of(&node), Some(Role::CheckBox));
        assert_eq!(toggle_of(&node), ToggleState::On);
    }

    #[test]
    fn toggle_reads_the_elements_own_classes() {
        let el = Elem::new(NodeTag::Control)
            .style("check-box")
            .style("indeterminate");
        let node = Node::from_element(&el);
        assert_eq!(toggle_of(&node), ToggleState::Indeterminate);

        let plain = Elem::new(NodeTag::Control).style("check-box");
        let node = Node::from_element(&plain);
        assert_eq!(toggle_of(&node), ToggleState::Off);
    }

    #[test]
    fn accessible_name_comes_from_the_label_child() {
        let el = Elem::new(NodeTag::Control)
            .style("check-box")
            .with_children([text_node("label", "Enable feature")]);
        let node = Node::from_element(&el);
        assert_eq!(accessible_name(&node), "Enable feature");
    }

    #[test]
    fn the_switch_and_radio_families_get_their_roles() {
        let sw = Elem::new(NodeTag::Control).style("switch").style("on");
        let node = Node::from_element(&sw);
        assert_eq!(role_of(&node), Some(Role::Switch));
        assert_eq!(toggle_of(&node), ToggleState::On);

        let radio = Elem::new(NodeTag::Control)
            .style("radio-box")
            .style("checked");
        let node = Node::from_element(&radio);
        assert_eq!(role_of(&node), Some(Role::RadioButton));
        assert_eq!(toggle_of(&node), ToggleState::On);

        // 装饰子节点不许抢角色：`switch-track` 与 `switch` 是两个分类名。
        let track = Elem::new(NodeTag::Container).style("switch-track");
        let node = Node::from_element(&track);
        assert_eq!(role_of(&node), None, "switch-track 是装饰层，不是开关本体");
    }

    #[test]
    fn a_plain_node_is_not_exposed() {
        let el = Elem::new(NodeTag::Container).style("check-group-items");
        let node = Node::from_element(&el);
        assert_eq!(role_of(&node), None);
    }

    /// 树**形**必须对：每个暴露元素挂在**父**上，装饰层不留痕。
    ///
    /// # 为什么这条要单独钉
    ///
    /// 属性、名字、状态都是「按元素查」的 —— 它们对不对，与树形对不对无关。
    /// 而树形曾经错过一次（每个暴露元素挂到了**自己**身上）：属性全对、
    /// 名字全对、状态全对，只有父子关系全丢 —— 屏幕阅读器念得出「复选框」，
    /// 却念不出「它在某个组里」。
    ///
    /// 这类错在画面上完全看不出来，只有真拿 Inspect 看一眼才发现。
    #[test]
    fn exposed_elements_hang_off_their_parent() {
        // 窗口根 → 分组 → （装饰层 → 复选框）+ 复选框
        let checkbox = Elem::new(NodeTag::Control)
            .style("check-box")
            .style("checked")
            .with_key("check:alpha")
            .with_children([text_node("label", "Enable feature")]);
        let group = Elem::new(NodeTag::Container)
            .style("check-group")
            .with_key("grp")
            .with_children([Elem::new(NodeTag::Container)
                .style("check-group-row")
                .with_children([checkbox])]);
        let root = Elem::new(NodeTag::Container).with_children([group]);
        let tree = Node::from_element(&root);

        let snap = build(&tree, &WindowInfo::default());

        // 窗口根 + 分组 + 复选框 = 3 个暴露元素。
        assert_eq!(snap.elements.len(), 3, "暴露的元素数：{:#?}", snap.elements);

        let root_el = snap.root_element().expect("窗口根");
        assert_eq!(root_el.role, Role::Window);
        assert_eq!(root_el.children.len(), 1, "根下面挂着分组");
        assert_eq!(root_el.children[0], 1);

        let group_el = snap.get(1).expect("分组");
        assert_eq!(group_el.role, Role::Group);
        assert_eq!(
            group_el.children,
            vec![2],
            "分组下面只挂那个复选框（装饰层不留痕）"
        );
        assert_eq!(group_el.children[0], 2);

        let cb = snap.get(2).expect("复选框");
        assert_eq!(cb.role, Role::CheckBox);
        assert_eq!(cb.parent, Some(1), "复选框的父是**分组**，不是它自己");
        assert!(!cb.children.contains(&2), "复选框不能把自己当孩子");
    }
}

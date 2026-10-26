//! 声明树 IR —— 编译期与运行时共用的**唯一**表示。
//!
//! # 为什么一份 IR 是命门
//!
//! dev 模式由 `zypuic::parse` 从文本构造这份 IR，release 模式由
//! [`crate::decode`] 从二进制反序列化出同一份。之后组件展开、表达式求值、
//! 实例化都只有一份实现。一旦出现「二进制里装另一种更省的表示」，
//! 两条路径就会漂移 —— 症状「dev 里对、release 里错」，极难定位。
//! （与 `zephyra-style` IR 文档同一结论，同一条命门。）
//!
//! # 层次
//!
//! ```text
//! RawUi
//!  ├─ components: [Component]      模板库（controls.zypui 等）
//!  └─ pages:     [Page]            每页一棵声明树（可含 Use 引用）
//!
//! Node = El(元素) | If(条件) | For(迭代) | Use(组件引用)
//! ```
//!
//! [`crate::instantiate`] 在每帧实例化时展开 `Use`，并求值模板中的表达式。
//!
//! # 字符串与身份
//!
//! 所有 `&'static str` 由 parse / decode 在**加载期泄漏一次**产生
//! （元素树每帧重建，`zephyra_core::Element` 只收 `&'static`）。
//! IR 树本身由应用持有，不参与每帧重建。

use crate::expr::Expr;
use zephyra_core::id::NodeTag;

/// 一份 `.zypui` 工件：组件模板库 + 全部页面。
#[derive(Debug, Clone)]
pub struct RawUi {
    /// 组件模板（跨文件共享一个命名空间，重名由 zypuic 报错）。
    pub components: Vec<Component>,
    /// 页面（顺序即文件内声明顺序；按 [`Page::name`] 取用）。
    pub pages: Vec<Page>,
}

/// 一个组件模板。
#[derive(Debug, Clone)]
pub struct Component {
    /// 组件名（Use 处引用的名字，PascalCase 约定）。
    pub name: &'static str,
    /// 形参名列表（Use 处传的实参按名对位；模板内标识符沿它解析）。
    pub params: Vec<&'static str>,
    /// 模板体。可含 `<children/>` 槽位（恰好一个，多于此数报错）。
    pub body: Vec<Node>,
}

/// 一个页面：命名的一组根节点。
#[derive(Debug, Clone)]
pub struct Page {
    /// 页名（`ui.instantiate("basics")` 的键）。
    pub name: &'static str,
    /// 页根（通常一个元素；多个根按序并列也是合法的）。
    pub root: Vec<Node>,
}

/// 声明树节点。
#[derive(Debug, Clone)]
pub enum Node {
    /// 裸元素（容器/文本/图像/控件）。
    El(ElementNode),
    /// `if cond : then <else>`。cond 是运行期表达式（含组件实参引用）。
    If(IfNode),
    /// `for var[, index] in list`。list 求值成 [`crate::Val::L`]。
    For(ForNode),
    /// 组件引用 —— 只存在于加载期；expand 之后树里不再有 Use。
    Use(UseNode),
    /// `<children/>` 槽位 —— 组件模板里回填调用方子树的位置。
    /// 只能出现在模板内（expand 时被 Use 的 children 替换；页根处出现即错）。
    Slot,
}

/// 裸元素。
#[derive(Debug, Clone)]
pub struct ElementNode {
    /// 元素类型标签。
    pub tag: NodeTag,
    /// 元素 key（事件路由身份）。缺省 = 无 key。
    pub key: Option<Expr>,
    /// 分类名序列（外观在 `.zstyle`，这里只有**名字**）。
    pub classes: Vec<ClassTok>,
    /// 内容类属性（白名单见 [`BARE_ATTRS`]；名字已在 zypuic 校验）。
    pub attrs: Vec<(&'static str, Expr)>,
    /// 子节点。
    pub children: Vec<Node>,
}

/// 分类名序列里的一项。
#[derive(Debug, Clone)]
pub enum ClassTok {
    /// 静态词。
    Lit(&'static str),
    /// 条件词：cond 为真时追加 `tok`（对应 Rust 侧 `if hovered { push("hovered") }`）。
    If { cond: Expr, tok: &'static str },
    /// 动态词：表达式的**求值结果整体是一个词**（结果含空格是 .zypui 源的错，
    /// 编译期查不到、运行期按单词收下 —— 门禁该盯的那类缝）。
    Dyn(Expr),
}

/// `if` 节点。
#[derive(Debug, Clone)]
pub struct IfNode {
    /// 条件（每帧求值）。
    pub cond: Expr,
    /// 条件为真时的分支。
    pub then: Vec<Node>,
    /// 条件为假时的分支（`<else>`；可为空）。
    pub els: Vec<Node>,
}

/// `for` 节点。
#[derive(Debug, Clone)]
pub struct ForNode {
    /// 迭代变量名（体内 `Ref(var)` 取当前项）。
    pub var: &'static str,
    /// 可选下标变量名（`for="name, i in @icons"`）。
    pub index: Option<&'static str>,
    /// 列表表达式（`@icons` 之类）。
    pub list: Expr,
    /// 循环体（每项实例化一份）。
    pub body: Vec<Node>,
}

/// 组件引用（Use）。
#[derive(Debug, Clone)]
pub struct UseNode {
    /// 组件名。
    pub comp: &'static str,
    /// 实例 key —— 展开时写到根元素的 key 上（并作为该实例 `hover`/`press` 的
    /// 当前 key）。与模板参数同名的 `key` 参数因此**不成立**：key 不是参数，
    /// 是实例身份，专属通道。
    pub key: Option<Expr>,
    /// 追加到根元素分类名尾部的词（实例级修饰，如 `Class="danger"`）。
    pub class_extra: Vec<ClassTok>,
    /// 按名传的实参（名字 ⊆ [`Component::params`]，zypuic 校验）。
    pub args: Vec<(&'static str, Expr)>,
    /// 调用方给的子树，回填进模板的 `<children/>` 槽位。
    pub children: Vec<Node>,
}

/// 裸元素的属性白名单。
///
/// **不在表内的名字 zypuic 直接报错** —— 静默忽略的属性比没有更糟
/// （和 zstylec 的 props_map 同一条规矩）。外观值（颜色/尺寸/间距）刻意
/// 不在表内：外观全在 `.zstyle`，`.zypui` 类 HTML 只管结构 —— 这是
/// 2026-10-25 与用户定稿的分工。
///
/// | 名字 | 去向 | 值 |
/// |---|---|---|
/// | `text` | [`zephyra_core::props::TEXT`] | 字符串 |
/// | `icon` | [`zephyra_core::props::ICON_NAME`] | 字符串（内置图标名） |
/// | `collapsed` | [`zephyra_core::props::COLLAPSED`] | 布尔 |
/// | `interactive` | [`zephyra_core::props::INTERACTIVE`] | 布尔 |
/// | `texture` | [`zephyra_core::props::IMAGE_TEXTURE`] | 字符串（纹理句柄） |
/// | `uv` | [`zephyra_core::props::IMAGE_SRC`] | `x,y,w,h` 四个数 |
/// | `width` / `height` | `WIDTH` / `HEIGHT` | 数值 —— **仅限 `fill:` 键**
///   （滑条/进度条/滚动条的填充宽，内容几何例外；zypuic 编译期强制）。
/// | `filter` | [`zephyra_core::props::FILTER`] | 滤镜链（结构值，唯
///   [`crate::Val::Raw`] 一路 —— 表达式语言没有它的字面量） |
/// | `blend` | [`zephyra_core::props::BLEND_MODE`] | 混合模式（同上，结构值） |
/// | `render-source` | [`zephyra_core::props::RENDER_SOURCE`] | 宿主注册的只读 `kind` 名称 |
/// | `url` | `Host::prop_key` 解析的键 | 字符串（hyperlink 的去向：数据）。
pub const BARE_ATTRS: &[&str] = &[
    "text",
    "icon",
    "collapsed",
    "interactive",
    "texture",
    "uv",
    "width",
    "height",
    "url",
    "filter",
    "blend",
    "render-source",
];

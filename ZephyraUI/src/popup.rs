//! 弹层（Popup）控件的状态模型。
//!
//! # 蓝本（WinUI `Flyout` / Android `PopupWindow`）
//!
//! 弹层是「浮在正常布局流之上的层」：它不占父容器的位置，不参与
//! 兄弟节点的排列，而是按自身尺寸与锚点定位，盖住下层内容。
//!
//! # 分工
//!
//! | 关注 | 住在哪 |
//! |---|---|
//! | open/closed 状态、点击外部关闭 | 本模块 |
//! | 结构（节点树、分类名、`layer: popup`） | `ui/controls.zypui` |
//! | 颜色与尺寸（阴影、圆角、背景、偏移） | `controls-*.zstyle` |
//! | 弹层何时弹出/关闭、点哪里触发、锚定到谁 | 宿主应用 |
//!
//! # 与 `Layer::Popup` 的关系
//!
//! 弹层节点自带 `layer: popup`（见 [`zephyra_core::props::LAYER`]），这样
//! 绘制趟会把它降到 `Layer::Popup`（独立离屏纹理），盖住下层所有内容。
//! 命中测试按 `(layer, z, order)` 排序，Popup 层（3）永远在 Content 层（1）
//! 之上，点中弹层时不会落到下层控件。
//!
//! # 与 Overlay 的区别
//!
//! `Overlay`（layer=2）是「装饰层」（边框、分隔线、焦点环），和内容
//! 在同一目标里画；`Popup`（layer=3）是「浮层层」，独立离屏后合成上去，
//! 可以带阴影、模糊、遮罩，且点击穿透规则不同（只向最上层分发）。

use zephyra_core::element::Element;
use zephyra_core::paint::Layer;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

/// 弹层：浮在正常布局流之上的层。
#[derive(Debug, Clone, PartialEq)]
pub struct Popup {
    /// 弹层的稳定 key（事件路由身份）。
    key: &'static str,
    /// 是否当前打开。
    open: bool,
    /// 点击弹层外是否自动关闭（默认 true）。
    dismiss_on_outside_click: bool,
}

impl Popup {
    /// 建一个默认关闭的弹层。
    pub const fn new(key: &'static str) -> Self {
        Self {
            key,
            open: false,
            dismiss_on_outside_click: true,
        }
    }

    /// 弹层的稳定 key。
    pub const fn key(&self) -> &'static str {
        self.key
    }

    /// 是否当前打开。
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// 打开。
    pub fn open(&mut self) {
        self.open = true;
    }

    /// 关闭。
    pub fn close(&mut self) {
        self.open = false;
    }

    /// 切换。
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// 点击弹层外是否自动关闭。
    pub const fn dismiss_on_outside_click(&self) -> bool {
        self.dismiss_on_outside_click
    }

    /// 设置点击弹层外是否自动关闭。
    pub fn set_dismiss_on_outside_click(&mut self, v: bool) {
        self.dismiss_on_outside_click = v;
    }

    /// 把当前状态写入元素（每帧由宿主的 `view()` 调用）。
    ///
    /// `open` 控制 [`zephyra_core::props::COLLAPSED`]（关闭时不占位、不绘制），
    /// `layer` 固定为 [`Layer::Popup`]。
    pub fn apply_to_element(&self, element: Element) -> Element {
        element
            .with(&props::COLLAPSED, PropertyValue::bool(!self.open))
            .with(&props::LAYER, PropertyValue::layer(Layer::Popup))
    }
}

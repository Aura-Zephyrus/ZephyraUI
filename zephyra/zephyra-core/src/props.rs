//! 标准属性键。
//!
//! 单独成模块而不是散落在 `tree` / `paint` 里，因为**三方都要用**：
//! 布局读尺寸与间距、绘制读外观、控件层写默认值。集中在这里意味着改一个键的
//! 语义只需要看一个文件，而"这个键到底影响重排还是只影响重绘"（见
//! [`crate::property::PropertyChange::affects_layout`]）也能一并对照。
//!
//! 全部是 `const`：属性定义是编译期常量，全树共享同一份身份，diff 靠指针相等配对。

use crate::core::{Color, CornerRadius, CornerShape, UvRect};
use crate::layout::{
    AlignContent, AlignItems, AlignSelf, Direction, FlexDirection, FlexWrap, MainAxisAlign,
};
use crate::property::{PropertyKey, PropertyValue};

// ============================== 布局 ==============================

/// 内边距（四边统一值）。
///
/// 四边统一是刻意的简化：UI 里绝大多数控件的 padding 是对称的。真需要非对称时
/// 把它扩成 `Thickness` 变体，而不是现在就为它把键拆成四个。
pub static PADDING: PropertyKey =
    PropertyKey::inheritable_layout("Padding", PropertyValue::float(0.0));

/// 显式宽度（DIP）。`None` 表示由布局决定。
pub static WIDTH: PropertyKey = PropertyKey::layout("Width", PropertyValue::float(f32::NAN));

/// 显式高度（DIP）。
pub static HEIGHT: PropertyKey = PropertyKey::layout("Height", PropertyValue::float(f32::NAN));

/// 堆叠方向（仅对容器生效）。
pub static DIRECTION: PropertyKey =
    PropertyKey::layout("Direction", PropertyValue::direction(Direction::Horizontal));

/// 主轴上相邻子项的间距。
pub static SPACING: PropertyKey = PropertyKey::layout("Spacing", PropertyValue::float(0.0));

/// 交叉轴对齐（由容器统一施加给子项）。
pub static CROSS_ALIGN: PropertyKey =
    PropertyKey::layout("CrossAlign", PropertyValue::cross_align(AlignItems::Start));

/// 主轴对齐（由容器施加给整组子项）。
pub static MAIN_ALIGN: PropertyKey =
    PropertyKey::layout("MainAlign", PropertyValue::main_align(MainAxisAlign::Start));

/// 子节点参与布局（`false` 时不占位、不绘制）。
///
/// 对应 WinUI 的 `Visibility=Collapsed`。与"宽度为 0"不同：零宽仍占一个间距位，
/// 而折叠完全不占——工具栏在图片模式下少一个按钮时，间距必须跟着消失。
pub static COLLAPSED: PropertyKey = PropertyKey::layout("Collapsed", PropertyValue::bool(false));

/// 溢出处理方式（`visible` / `hidden` / `scroll` / `auto`）。
///
/// **影响布局**：滚动容器（`scroll` / `auto`）的子内容按**无界约束**测量，
/// 于是内容可以比视口大；`visible` / `hidden` 则按父给的约束测量。
/// 改这一档会改变内容尺寸，所以必须重排。
///
/// 默认 `visible`：内容溢出时照常画到容器外（本仓的历史行为）。
pub static OVERFLOW: PropertyKey = PropertyKey::layout(
    "Overflow",
    PropertyValue::overflow(crate::layout::Overflow::Visible),
);

// ============================== 定位 ==============================

/// 定位方式（D1）：`flow` / `parent` / `root`，默认 `flow`。
pub static POSITION: PropertyKey = PropertyKey::layout(
    "Position",
    PropertyValue::position(crate::layout::Position::Flow),
);

/// 手动定位的自锚点（border box 9 点），默认 `top-left`。
pub static ANCHOR: PropertyKey = PropertyKey::layout(
    "Anchor",
    PropertyValue::anchor(crate::layout::Anchor::TopLeft),
);

/// 手动定位在参考系内的锚点 x 坐标（DIP）。`NaN` 表示该轴未显式设置。
pub static LEFT: PropertyKey = PropertyKey::layout("Left", PropertyValue::float(f32::NAN));

/// 手动定位在参考系内的锚点 y 坐标（DIP）。`NaN` 表示该轴未显式设置。
pub static TOP: PropertyKey = PropertyKey::layout("Top", PropertyValue::float(f32::NAN));

// ============================== Flexbox ==============================
//
// 三个 CSS 名刻意映射到**既有键**而不是另开新键：
// `align-items` → `CROSS_ALIGN`、`justify-content` → `MAIN_ALIGN`、`gap` → `SPACING`
// （映射在 `zstylec::props_map`）。两份键的症状是「Stack 路径读得到、Flex 路径
// 读不到」这类半生效状态 —— 同一个语义永远只有一处读法。

/// Flex 主轴方向（row / row-reverse / column / column-reverse）。
///
/// **声明它 = 本容器是 flex 容器**（见 `style::Node::flex`：只有 `FlexDirection`
/// 或 `FlexWrap` 之一被写上才走 flex 分配路径，其余全部走 `Stack` 快路径）。
///
/// 未声明时主轴回落到 [`Direction`]：flex 容器上写 `direction: vertical`
/// 等价于 `flex-direction: column`。
pub static FLEX_DIRECTION: PropertyKey = PropertyKey::layout(
    "FlexDirection",
    PropertyValue::flex_direction(FlexDirection::Row),
);

/// Flex 换行策略（nowrap / wrap / wrap-reverse）。
///
/// 声明它同样是**进入 flex 的开关** —— 「只想要换行」的容器不必同时写
/// `flex-direction`。
pub static FLEX_WRAP: PropertyKey =
    PropertyKey::layout("FlexWrap", PropertyValue::flex_wrap(FlexWrap::NoWrap));

/// 多行时行组在交叉轴上的分布（CSS 的 `align-content`；单行时不生效）。
pub static ALIGN_CONTENT: PropertyKey = PropertyKey::layout(
    "AlignContent",
    PropertyValue::align_content(AlignContent::Stretch),
);

// ============================== Flexbox 子项 ==============================

/// 单项交叉轴对齐（覆盖容器的 align-items，对应 align-self）。
pub static ALIGN_SELF: PropertyKey =
    PropertyKey::layout("AlignSelf", PropertyValue::align_self(AlignSelf::Auto));

/// Flex 扩展因子（剩余空间按比例分配，对应 flex-grow）。
pub static FLEX_GROW: PropertyKey = PropertyKey::layout("FlexGrow", PropertyValue::float(0.0));

/// Flex 收缩因子（溢出时按比例收缩，对应 flex-shrink）。
pub static FLEX_SHRINK: PropertyKey = PropertyKey::layout("FlexShrink", PropertyValue::float(1.0));

/// Flex 基准尺寸（auto 或显式尺寸，对应 flex-basis）。
///
/// `f32::NAN` 表示 `auto`（由内容决定）。
pub static FLEX_BASIS: PropertyKey =
    PropertyKey::layout("FlexBasis", PropertyValue::float(f32::NAN));

/// 显式排序（越小越靠前，对应 order）。
pub static ORDER: PropertyKey = PropertyKey::layout("Order", PropertyValue::int(0));

// ============================== 外观 ==============================

/// 背景填充色。`Color(0,0,0,0)` 表示不画背景。
pub static BACKGROUND: PropertyKey = PropertyKey::new(
    "Background",
    PropertyValue::color(Color::rgba(0.0, 0.0, 0.0, 0.0)),
);

/// 圆角半径（DIP）。
pub static CORNER_RADIUS: PropertyKey = PropertyKey::new("CornerRadius", PropertyValue::float(0.0));

/// 角形状（CSS `corner-shape`）。默认 `round`（`superellipse(1)`，圆弧）；
/// `squircle` 为 `superellipse(2)`（连续曲率观感）。纯视觉，不影响布局。
pub static CORNER_SHAPE: PropertyKey = PropertyKey::new(
    "CornerShape",
    PropertyValue::corner_shape(CornerShape::ROUND),
);

/// 描边颜色。alpha 为 0 表示不描边。
pub static BORDER_COLOR: PropertyKey = PropertyKey::new(
    "BorderColor",
    PropertyValue::color(Color::rgba(0.0, 0.0, 0.0, 0.0)),
);

/// 描边宽度（DIP）。
pub static BORDER_WIDTH: PropertyKey = PropertyKey::new("BorderWidth", PropertyValue::float(0.0));

/// 整体不透明度。
pub static OPACITY: PropertyKey = PropertyKey::inheritable("Opacity", PropertyValue::float(1.0));

/// 绘制时 X 偏移（DIP）。
///
/// # 为什么**不是**布局属性
///
/// `affects_layout() == false` 是这个属性的**全部意义**：位移只改画面，
/// 不改布局盒、不改命中测试。声明成布局属性会让入场动画每帧触发一次全树重排
/// （实测 demo 45 节点 ≈ 一趟 StackPanel 遍历），而动画要的恰恰是"不重排"。
///
/// 于是动画期间控件仍然可点、命中测试仍然按动画前的位置走 —— 这与浏览器上
/// `transform` 的行为一致（`transform` 不参与布局，`will-change` 存在的理由正是
/// 避免每帧重排）。
pub static TRANSLATE_X: PropertyKey = PropertyKey::new("TranslateX", PropertyValue::float(0.0));

/// 绘制时 Y 偏移（DIP）。见 [`TRANSLATE_X`]。
pub static TRANSLATE_Y: PropertyKey = PropertyKey::new("TranslateY", PropertyValue::float(0.0));

/// 绘制时 X 缩放（以元素矩形中心为原点）。
///
/// # 语义（与 [`TRANSLATE_X`] 的差别）
///
/// 与偏移一样是**纯绘制**属性（`affects_layout() == false`），不改布局盒、
/// 不改命中测试。但作用面不同：
///
/// - 偏移**沿父链累加**，带着整棵子树一起动；
/// - 缩放**只作用于本节点自己的外观** —— 背景 / 描边 / 图像 / 图标 /
///   玻璃装饰，想在子节点上缩放就自己再写一遍 `scale-x` / `scale-y`。
///
/// # 为什么不做成"带子树的变换"
///
/// 文字的字形四边形按元素原点缓存（见 [`crate::paint::Command::Text`]），
/// 若缩放要连文字一起放大，就得给那条命令再加一个缩放字段并改三后端的
/// 顶点展开 —— 那是独立一轮。当前用它的场景（开关把手按压时拉伸）是叶子
/// 节点，没有子树也没有文字，所以先把语义钉死在"本节点外观"上，并在这条
/// 注释里写清楚，而不是让调用方以为它能当 CSS `transform` 用。
///
/// # 非法值
///
/// `<= 0` 与非有限值一律按 `1.0`（不缩放）处理：负值意味着镜像，而四类
/// 图元的矩形 / 圆角 / 描边都没有镜像语义（圆角会成为反向外框）；非有限值
/// 会让顶点变 NaN，表现为"整块不画"。
pub static SCALE_X: PropertyKey = PropertyKey::new("ScaleX", PropertyValue::float(1.0));

/// 绘制时 Y 缩放。见 [`SCALE_X`]。
pub static SCALE_Y: PropertyKey = PropertyKey::new("ScaleY", PropertyValue::float(1.0));

/// 这四个键是**绘制变换**：改了它们，节点就会「移走」——旧位置的像素得有人清，
/// 而脏区算的是**新**位置。
///
/// 判据用**地址**而不是名字（与本仓其余处一致）：`PropertyKey` 没有
/// `PartialEq`，名字只是调试输出用的。
pub fn is_transform_key(key: &PropertyKey) -> bool {
    std::ptr::eq(key, &TRANSLATE_X)
        || std::ptr::eq(key, &TRANSLATE_Y)
        || std::ptr::eq(key, &SCALE_X)
        || std::ptr::eq(key, &SCALE_Y)
}

/// 合成层（决定后端如何切 render pass）。
pub static LAYER: PropertyKey =
    PropertyKey::new("Layer", PropertyValue::layer(crate::paint::Layer::Content));

/// 裁剪矩形（相对节点自身，DIP）。`Some` 时绘制趟在该节点的命令前后发出
/// `PushClip`/`PopClip`。`affects_layout = false` —— 只影响绘制，不触发重排。
///
/// 默认 `Rect::new(0, 0, 0, 0)`（全零，等效于不裁剪）；样式表写 `clip-rect: x y w h`
/// 即可生效。
pub static CLIP_RECT: PropertyKey = PropertyKey::new(
    "ClipRect",
    PropertyValue::rect(crate::core::Rect::new(0.0, 0.0, 0.0, 0.0)),
);

/// 滚动偏移 X（DIP）。由 `TextArea` 等滚动控件写入，供 `.zypui` 模板
/// 的 `translate-x` 绑定读取，实现内容滚动。
pub static SCROLL_OFFSET_X: PropertyKey =
    PropertyKey::new("Scroll.OffsetX", PropertyValue::float(0.0));

/// 滚动偏移 Y（DIP）。
pub static SCROLL_OFFSET_Y: PropertyKey =
    PropertyKey::new("Scroll.OffsetY", PropertyValue::float(0.0));

/// 滚动内容宽度（DIP）。由滚动容器的子内容按实际尺寸设置，用于计算
/// 滚动范围的上限。
pub static SCROLL_CONTENT_WIDTH: PropertyKey =
    PropertyKey::new("Scroll.ContentWidth", PropertyValue::float(0.0));

/// 滚动内容高度（DIP）。
pub static SCROLL_CONTENT_HEIGHT: PropertyKey =
    PropertyKey::new("Scroll.ContentHeight", PropertyValue::float(0.0));

/// 选区起始字节下标（DIP）。与 [`CURSOR`] 配合，用于渲染选区高亮块的宽度与位置。
pub static SELECTION_BEGIN: PropertyKey =
    PropertyKey::new("Text.SelectionBegin", PropertyValue::float(0.0));

/// 选区结束字节下标（DIP）。
pub static SELECTION_END: PropertyKey =
    PropertyKey::new("Text.SelectionEnd", PropertyValue::float(0.0));

// ============================== 效果 ==============================

/// 滤镜链。
///
/// 不可继承：滤镜是**节点自身的**外观属性，父节点设了模糊不该让整棵子树跟着
/// 糊掉（那正是"可继承"会带来灾难的地方 —— `Opacity` 可继承是因为半透明叠加
/// 天然正确，模糊叠加则完全不成立）。
pub static FILTER: PropertyKey = PropertyKey::new(
    "Filter",
    PropertyValue::filter(crate::filter::FilterChain::none()),
);

/// 材质：一种**高层**表面语义（纯色 / 磨砂玻璃 / 液态玻璃），与 [`FILTER`] 正交。
///
/// # 为什么独立于 [`FILTER`]
///
/// `filter:` 是底层效果链（`blur` / `drop-shadow` / `refraction` …），把效果
/// 一个接一个排起来；`material:` 是高层的**材质选型**（`solid` / `frosted` /
/// `glass`），一个词对应一整块表面处理。两者**可共存且互不覆盖**：一个节点可以
/// `filter: drop-shadow(...)`（阴影）+ `material: glass`（玻璃底）—— 展开时
/// 材质变一条滤镜、与 `filter:` 的链合并。
///
/// # 为什么参数不进样式表
///
/// 材质是**纯视觉**的选型，`.zstyle` 只写 `material: frosted` 一个词，不用记
/// 厚度/σ/折射率。参数藏进 [`crate::filter::Material`] 的默认值，浓淡强弱是
/// 后续要加档位时的事。
///
/// 不可继承：与 [`FILTER`] 同理 —— "这块表面怎么处理"是节点自己的事。
pub static MATERIAL: PropertyKey = PropertyKey::new(
    "Material",
    PropertyValue::material(crate::filter::Material::solid()),
);

/// 材质参数的**独立覆盖**（`material-params`），与 [`MATERIAL`] 分开。
///
/// `material:` 只选**种类**（`solid` / `frosted` / `glass`），本属性调它的**参数**
/// （σ / 厚度 / 倒角 / 色散 / 折射率）。两者互相独立：同一个 `material: glass`
/// 可以在不同页面用不同的 `material-params`；不写本属性时用种类自带的默认值。
///
/// 每个字段是 `Option`——「没写」与「写了但只调其中几项」都合法，未写的项落到
/// [`crate::filter::Material`] 的默认常量。`material-params` 的书写顺序是
/// `<amount> <bezel> <dispersion> <ior>`（见 [`crate::filter::MaterialParams`]）。
///
/// 不可继承：与 [`FILTER`] 同理。
pub static MATERIAL_PARAMS: PropertyKey = PropertyKey::new(
    "MaterialParams",
    PropertyValue::material_params(crate::filter::MaterialParams::none()),
);

/// 玻璃装饰（`decoration`）：镜面高光 / 外投影 / 内阴影。
///
/// # 为什么它与 [`FILTER`] / [`MATERIAL`] 是第三件事
///
/// | 属性 | 描述什么 |
/// |---|---|
/// | [`FILTER`] | 这条命令**自己**要多经过什么处理（模糊 / 投影 / 折射） |
/// | [`MATERIAL`] | 高层的表面选型（纯色 / 磨砂 / 液态玻璃） |
/// | 本属性 | 一块玻璃的**装饰四件套**：各画在内容的哪一侧 |
///
/// 层序（外投影在内容下、高光与内阴影在内容上）是**契约**，所以它不能塞进
/// 滤镜链 —— 链里没有"这一层在内容之上"的表达。见 [`crate::decoration`]。
///
/// 不可继承：与 [`FILTER`] 同理。
pub static DECORATIONS: PropertyKey = PropertyKey::new(
    "Decorations",
    PropertyValue::decorations(crate::decoration::Decorations::none()),
);

/// 混合模式（默认 [`crate::filter::BlendMode::SrcOver`]，即普通 alpha 合成）。
///
/// 同样不可继承：混合模式描述的是"这一项与它**下面**那层的合成方式"，而"下面"
/// 随父链变化，继承下来语义就乱了。
pub static BLEND_MODE: PropertyKey = PropertyKey::new(
    "BlendMode",
    PropertyValue::blend_mode(crate::filter::BlendMode::SrcOver),
);

/// 外部渲染源（视频帧 / GPU 图表 / 宿主任意自绘代码的接入点）。
///
/// 真值走 [`PropertyValue::Custom`]——属性系统对它是**不透明**的：样式表写不了
/// 它、主题解析跳过它，diff 用指针相等判「没变」（载荷跨帧共享同一 `Arc` 时
/// 稳态零重建）。后端按 [`crate::paint::RenderSource`] 的 `kind` 查钩子注册表。
///
/// 不可继承：与 [`FILTER`] 同理——"这块屏幕归谁画"是节点自己的事。
///
/// 默认值是空串占位而非 [`PropertyValue::Custom`]：属性键的默认值必须能在
/// const 里构造，而 `Custom` 要分配。读侧（`Node::render_source`）把非
/// `Custom` 的值一律读成「无」，所以占位没有语义。
pub static RENDER_SOURCE: PropertyKey =
    PropertyKey::new("RenderSource", PropertyValue::StaticStr(""));

/// 宿主任意 2D 路径几何（Canvas 式动画的接入点）。
///
/// 真值走 [`PropertyValue::Custom`]，载荷是 [`crate::paint::DynamicPath`] —— 已三角化的
/// 顶点（宿主每帧按参数生成）。核心不解释坐标含义，只把它变成 `Command::Path`；
/// 样式表写不了它、主题解析跳过它。默认值是空串占位（`const` 里造不了 `Custom`），
/// 读侧（`Node::path_data`）把非 `Custom` 读成「无」。
///
/// 不可继承：路径是节点自己的绘制内容。
pub static PATH_DATA: PropertyKey = PropertyKey::new("Path.Data", PropertyValue::StaticStr(""));

// ============================== 内容 ==============================

/// 图像纹理句柄（宿主定义）。
pub static IMAGE_TEXTURE: PropertyKey =
    PropertyKey::new("Image.Texture", PropertyValue::static_str(""));

/// 图像绘制矩形（DIP 边长，正方形）。`-1` 表示"撑满节点矩形"。
pub static IMAGE_SIZE: PropertyKey = PropertyKey::new("Image.Size", PropertyValue::float(-1.0));

/// 图像着色（乘算）。
pub static IMAGE_TINT: PropertyKey = PropertyKey::new(
    "Image.Tint",
    PropertyValue::color(Color::rgb(1.0, 1.0, 1.0)),
);

/// 图像源裁剪（归一化 uv，`0,0,1,1` = 整张）。
///
/// 九宫格（`Slice`）、雪碧图图集、SVG viewBox 都靠它取一个子区域。归一化而非
/// 像素，因为宿主上传的纹理尺寸不由核心决定 —— 核心若存像素值，同一张图换个
/// 分辨率就要重新设一遍。
pub static IMAGE_SRC: PropertyKey =
    PropertyKey::new("Image.Src", PropertyValue::uv_rect(UvRect::FULL));

// ============================== 矢量图标 ==============================

/// 内置图标名（[`crate::icon::assets::REGISTRY`] 里的键）。
///
/// # 为什么图标名是**资源键**而不是内联路径数据
///
/// 两条理由：
///
/// 1. **契约层不认识 SVG。** 核心的 `icon` 模块只**解析** path data（`parse_path_data`），
///    而"怎么拿到那段数据"是宿主的事 —— 就像纹理句柄是宿主定义的一样。名字让核心
///    只持一把 `&'static str`，宿主与内置资源表都能满足。
/// 2. **同一个图标可以有多份几何。** 内置表里 `camera` 是一条 path data，但应用
///    自己的图标库也是「名字 → 数据」。用名字作键，两边共用同一条属性。
///
/// # 与 [`IMAGE_TEXTURE`] 的关系
///
/// 两者是**互斥**的图元选择，不是叠加：
///
/// - 图标名 → 走 [`crate::paint::Command::Path`]（CPU 三角化 + MSAA 边缘）；
/// - 纹理句柄 → 走 [`crate::paint::Command::Image`]（采样，位图自带 AA）。
///
/// 同时设两者时**图像胜出**（它不需要三角化，且已有位图缓存）。理由见
/// `crate::paint::render_rec` 里那段的说明 —— 那是一条真会被写出来的歧义，
/// 所以判据写在这里而不留在实现里。
pub static ICON_NAME: PropertyKey = PropertyKey::new("Icon.Name", PropertyValue::static_str(""));

/// 图标填充色。
///
/// 不继承：图标的颜色由**该图标所在的控件**决定（按钮前景色），而继承链上的
/// 文本色未必是同一个语义（同一行里按钮与标签常常不同色）。
pub static ICON_COLOR: PropertyKey = PropertyKey::new(
    "Icon.Color",
    PropertyValue::color(Color::rgb(1.0, 1.0, 1.0)),
);

// ============================== 文本 ==============================

/// 文本内容。
///
/// 值是字符串本身（`StaticStr` 或 `Str`），**不是**资源键 —— 文本不是设计令牌，
/// 不该进主题。颜色与字号才是。
pub static TEXT: PropertyKey = PropertyKey::new("Text", PropertyValue::static_str(""));

/// 字号（DIP）。`NaN` 表示"用宿主默认"。
pub static TEXT_SIZE: PropertyKey =
    PropertyKey::layout("Text.Size", PropertyValue::float(f32::NAN));

/// 文本颜色（继承）。
pub static TEXT_COLOR: PropertyKey = PropertyKey::inheritable(
    "Text.Color",
    PropertyValue::color(Color::rgb(1.0, 1.0, 1.0)),
);

/// 文本最大宽度（DIP），用于断行。`NaN` 表示不限宽（不换行）。
pub static TEXT_MAX_WIDTH: PropertyKey =
    PropertyKey::layout("Text.MaxWidth", PropertyValue::float(f32::NAN));

/// 文本行高倍数（继承）。`NaN` 表示用字体自带值。
pub static TEXT_LINE_HEIGHT: PropertyKey =
    PropertyKey::inheritable_layout("Text.LineHeight", PropertyValue::float(f32::NAN));

/// 字体族名（继承）。空串表示交给字体回退链决定。
pub static FONT_FAMILY: PropertyKey =
    PropertyKey::inheritable("Font.Family", PropertyValue::static_str(""));

/// 字重（继承，400 = Regular）。`NaN` 表示 400。
pub static FONT_WEIGHT: PropertyKey =
    PropertyKey::inheritable("Font.Weight", PropertyValue::float(f32::NAN));

// ---- 高级文字属性（值与 zephyra-vector 的 TextStyle 逐字段对应）----
//
// 约定：数值键用 `NaN` 作「未设置」哨兵（负值合法，不能用 `finite()` 过滤）；
// 枚举 / 结构化值用 `PropertyValue::custom` 承载，默认给 `Float(NaN)`，
// 读取端 `get::<T>()` 失败即视为未设置。

/// 字间距（继承，DIP，可为负）。
pub static TEXT_LETTER_SPACING: PropertyKey =
    PropertyKey::inheritable_layout("Text.LetterSpacing", PropertyValue::float(f32::NAN));

/// 词间距（继承，DIP，可为负）。
pub static TEXT_WORD_SPACING: PropertyKey =
    PropertyKey::inheritable_layout("Text.WordSpacing", PropertyValue::float(f32::NAN));

/// 段间距（继承，DIP，可为负）。
pub static TEXT_PARAGRAPH_SPACING: PropertyKey =
    PropertyKey::inheritable_layout("Text.ParagraphSpacing", PropertyValue::float(f32::NAN));

/// 水平对齐。值为 `TextAlign`。
pub static TEXT_ALIGN: PropertyKey =
    PropertyKey::inheritable("Text.Align", PropertyValue::float(f32::NAN));

/// 段落基准方向。值为 `TextDirection`。
pub static TEXT_DIRECTION: PropertyKey =
    PropertyKey::inheritable_layout("Text.Direction", PropertyValue::float(f32::NAN));

/// 字号缩放系数。`NaN` 表示 1.0。
pub static TEXT_SCALE: PropertyKey =
    PropertyKey::inheritable_layout("Text.Scale", PropertyValue::float(f32::NAN));

/// 最大行数。`NaN` 表示不限。
pub static TEXT_MAX_LINES: PropertyKey =
    PropertyKey::inheritable_layout("Text.MaxLines", PropertyValue::float(f32::NAN));

/// 截断省略号。空串表示 "…"。
pub static TEXT_ELLIPSIS: PropertyKey =
    PropertyKey::inheritable_layout("Text.Ellipsis", PropertyValue::static_str(""));

/// 是否允许连字符断词。
pub static TEXT_HYPHENS: PropertyKey =
    PropertyKey::inheritable_layout("Text.Hyphens", PropertyValue::bool(false));

/// CPAL 调色板索引（COLR v1 彩色字形）。
pub static TEXT_PALETTE: PropertyKey =
    PropertyKey::inheritable("Text.Palette", PropertyValue::int(0));

/// 强制行框。值为 `StrutStyle`。
pub static TEXT_STRUT: PropertyKey =
    PropertyKey::inheritable_layout("Text.Strut", PropertyValue::float(f32::NAN));

/// 可变字体轴。值为 `Vec<([u8; 4], f32)>`。
pub static TEXT_VARIATIONS: PropertyKey =
    PropertyKey::inheritable_layout("Text.Variations", PropertyValue::float(f32::NAN));

/// OpenType 特性开关。值为 `Vec<(u32, u32)>`。
pub static FONT_FEATURES: PropertyKey =
    PropertyKey::inheritable_layout("Font.Features", PropertyValue::float(f32::NAN));

/// 装饰线位标志。值为 `TextDecoration`。
pub static TEXT_DECORATION: PropertyKey =
    PropertyKey::inheritable("Text.Decoration", PropertyValue::float(f32::NAN));

/// 装饰线样式。值为 `TextDecorationStyle`。
pub static TEXT_DECORATION_STYLE: PropertyKey =
    PropertyKey::inheritable("Text.DecorationStyle", PropertyValue::float(f32::NAN));

/// 装饰线与墨迹的相交方式。值为 `TextDecorationMode`。
pub static TEXT_DECORATION_MODE: PropertyKey =
    PropertyKey::inheritable("Text.DecorationMode", PropertyValue::float(f32::NAN));

/// 装饰线粗细（DIP，`NaN` 用字体建议值）。
pub static TEXT_DECORATION_THICKNESS: PropertyKey =
    PropertyKey::inheritable("Text.DecorationThickness", PropertyValue::float(f32::NAN));

/// 下划线相对基线的偏移（DIP，`NaN` 用字体建议值，可为负）。
pub static TEXT_DECORATION_OFFSET: PropertyKey =
    PropertyKey::inheritable("Text.DecorationOffset", PropertyValue::float(f32::NAN));

/// 装饰线粗细倍率。`NaN` 表示 1.0。
pub static TEXT_DECORATION_THICKNESS_MULTIPLIER: PropertyKey = PropertyKey::inheritable(
    "Text.DecorationThicknessMultiplier",
    PropertyValue::float(f32::NAN),
);

/// 伪粗体阈值覆盖（SDF 值域）。
pub static TEXT_EMBOLDEN: PropertyKey =
    PropertyKey::inheritable("Text.Embolden", PropertyValue::float(f32::NAN));

/// 文字阴影。值为 `TextShadow`。
pub static TEXT_SHADOW: PropertyKey =
    PropertyKey::inheritable("Text.Shadow", PropertyValue::float(f32::NAN));

/// LCD 子像素排列。值为 `LcdOrder`。
pub static TEXT_LCD: PropertyKey =
    PropertyKey::inheritable("Text.Lcd", PropertyValue::float(f32::NAN));

/// 字形绕自身中心旋转（弧度）。
pub static TEXT_ROTATION: PropertyKey =
    PropertyKey::inheritable("Text.Rotation", PropertyValue::float(f32::NAN));

/// 文字路径。值为 `TextPath`。
pub static TEXT_PATH: PropertyKey =
    PropertyKey::inheritable_layout("Text.Path", PropertyValue::float(f32::NAN));

/// 光标位置（字节下标，DIP）。由 `TextField` / `TextArea` 控件写入，供 `.zypui`
/// 模板的 `translate-x` 绑定读取，把光标定位到对应字符位置。
pub static CURSOR: PropertyKey = PropertyKey::new("Text.Cursor", PropertyValue::float(0.0));

// ============================== 交互 ==============================

/// 是否可交互。`false` 的节点只作为事件路径的一部分返回，不接收指针。
pub static INTERACTIVE: PropertyKey = PropertyKey::new("Interactive", PropertyValue::bool(false));

/// 是否为窗口拖动区（frameless 应用自绘标题栏用）。
///
/// 宿主在指针按下时命中测试：若命中节点此属性为真，调用窗口系统的
/// `drag_window()` 让系统接管拖动 —— 与 Tauri 的 `data-tauri-drag-region`
/// 同一语义。只标在标题栏容器上；其内的按钮等交互子节点**不**标，于是
/// 子元素豁免自然成立（命中的是按钮而非标题栏）。
pub static WINDOW_DRAG: PropertyKey = PropertyKey::new("Window.Drag", PropertyValue::bool(false));

/// 动画资源 id 或路径（`.zypui` 的 `animation` 属性写入）。
///
/// 只存引用，不存语法串：运行时动画语法只有 SMIL（见 docs/developer/syntax.md）。
pub static ANIMATION: PropertyKey = PropertyKey::new("Animation", PropertyValue::StaticStr(""));

/// 动画进度 0..1（`animation-progress`）。由应用绑定驱动，时钟归应用。
pub static ANIMATION_PROGRESS: PropertyKey =
    PropertyKey::new("Animation.Progress", PropertyValue::float(0.0));

/// 动画播放状态（`animation-play-state`）：`true` = running。
pub static ANIMATION_PLAY_STATE: PropertyKey =
    PropertyKey::new("Animation.PlayState", PropertyValue::bool(true));

/// 是否处于选中态（控件层据此换用 `control_fill_pressed` 之类的资源）。
///
/// 之所以是**属性**而不是控件内部字段：选中态要能被样式与资源解析影响，而
/// `ThemeResource` 只能作用于属性。
pub static SELECTED: PropertyKey = PropertyKey::new("Selected", PropertyValue::bool(false));

/// 是否禁用（禁用态的文字与填充取 `content_disabled` / 更低的填充层）。
pub static DISABLED: PropertyKey = PropertyKey::new("Disabled", PropertyValue::bool(false));

/// 是否可聚焦（进入 Tab 顺序，能获得键盘焦点）。
///
/// 与 [`INTERACTIVE`] **刻意分开**：按钮可交互但**不可聚焦**（Tab 不该停在
/// 按钮上），文本框可聚焦但命中测试靠它自己的 `INTERACTIVE` 子节点。两个属性
/// 回答两个问题——「能被鼠标点到吗」与「该进键盘顺序吗」。
pub static FOCUSABLE: PropertyKey = PropertyKey::new("Focusable", PropertyValue::bool(false));

/// 当前是否拥有键盘焦点。
///
/// 每棵树**至多一个**节点为真，由 `zephyra_core::input::focus::FocusState` 维护
/// 并写进这里。之所以是**属性**而不是节点内部字段：焦点是**跨节点**的全局事实，
/// 只有写成属性，`.zstyle` 才能按它给控件换焦点边框（`focus_ring` /
/// `focus_inner` 令牌），否则焦点样式就得退回源码里写死颜色。
///
/// 控件层读它决定"要不要显示光标 / 焦点环"，`view()` 时查自己的 key 即可。
pub static FOCUSED: PropertyKey = PropertyKey::new("Focused", PropertyValue::bool(false));

/// 是否**可激活**：拥有键盘焦点时，Space / Enter 触发点击（等价一次指针点击）。
///
/// # 为什么与 [`FOCUSABLE`] 分开
///
/// WinUI 的 `KeyPress::ButtonBase::IsPress` 把 Space / Enter 当指针按下，但这条语义
/// 只属于**按钮**一类。文本框同样可聚焦（[`FOCUSABLE`]），可它的 Space 是**插入一个
/// 空格**而不是触发提交。若把「可激活」并进「可聚焦」，焦点一旦落到输入框，打空格
/// 就会误触发——一个焦点系统建好之后**最先**会踩的坑。
///
/// 所以聚焦回答「Tab 该不该停在这」，激活回答「停在这时空格/回车意味着什么」，
/// 两个问题两个属性。控件层同时声明两者即成一个键盘可达的按钮。
pub static ACTIVATABLE: PropertyKey = PropertyKey::new("Activatable", PropertyValue::bool(false));

/// 未使用的占位，避免 `CornerRadius` 导入被判定为 dead code。
///
/// `CornerRadius` 目前只作为文档语义出现（绘制命令用的是裸 `f32` 半径），
/// 但它属于本模块的公开词汇表，保留导入以便后续 `CORNER_RADIUS` 改用该类型。
#[allow(dead_code)]
type RadiusDoc = CornerRadius;

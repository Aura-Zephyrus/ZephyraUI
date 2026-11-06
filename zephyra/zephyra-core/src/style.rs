//! 样式：把 [`Node`] 的属性表 + [`Context`] 翻译成具体值。
//!
//! # 三层读取顺序
//!
//! 每个 getter 走同一条链（见 [`Node::value`]）：本地 → 样式 → 继承 → 默认，
//! 然后按当前主题解析其中的资源键。区别只在**缺省值**与**是否该解析**。
//!
//! # 为什么大部分 getter 是 `pub` 而不是 `pub(crate)`
//!
//! 它们是控件层的公共词汇表 —— `Button` 想知道 hover 背景色，宿主想读 `opacity`
//! 算合成，都直接调即可。把它们藏起来只会逼出"复制一份读取逻辑"的代码，那份副本
//! 会在资源键解析上与本模块产生细微不一致。

use crate::context::Context;
use crate::node::Node;
use crate::property::PropertyValue;

impl Node {
    /// 是否被折叠（不占位、不绘制）。
    pub fn is_collapsed(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::COLLAPSED, ctx)
    }

    /// 定位方式。
    pub fn position(&self, ctx: Context<'_>) -> crate::layout::Position {
        match self.value(&crate::props::POSITION, ctx).as_deref() {
            Some(PropertyValue::Position(p)) => *p,
            _ => crate::layout::Position::Static,
        }
    }

    /// 是否绝对定位（脱离正常流）。
    pub fn is_absolute(&self, ctx: Context<'_>) -> bool {
        self.position(ctx) == crate::layout::Position::Absolute
    }

    /// 内边距（四边统一值）。
    pub fn padding(&self, ctx: Context<'_>) -> crate::core::Thickness {
        crate::core::Thickness::uniform(self.number(&crate::props::PADDING, ctx))
    }

    /// 文本内容（`None` = 无文本）。
    ///
    /// 接受 `StaticStr`（编译期常量）与 `Str`（运行期字符串）两种来源 —— 属性
    /// 默认值只能是前者，但真实文本几乎总是运行期构造的（来自应用状态）。
    ///
    /// # 刻意不走 `value()`
    ///
    /// 其它属性读法都经 [`Node::value`]，因为它们可能是**资源键**，需要在读取那一刻
    /// 按当前 `&Theme` 解析。文本不是设计令牌、永远不会解析成资源，所以直接用
    /// [`crate::property::resolve`] 拿属性表里的 `&PropertyValue`。
    ///
    /// 这么做同时解决了借用问题：`value()` 返回 `Cow<'_, PropertyValue>`，它内部
    /// 的 `Deref` 只能借到局部绑定，于是从里面取出的 `&str` 反而活得比属性表短。
    /// 直接拿 `&PropertyValue` 就得到指向 `Arc<str>` 的引用，**零拷贝**。
    pub fn text(&self) -> Option<&str> {
        let (raw, _source) = crate::property::resolve(self.props(), &crate::props::TEXT, None);
        let s: &str = match raw {
            PropertyValue::StaticStr(s) => s,
            PropertyValue::Str(s) => s.as_ref(),
            _ => return None,
        };
        (!s.is_empty()).then_some(s)
    }

    /// 文本样式（由本节点的文本属性合成，不含字体 —— 字体是宿主的事）。
    pub fn text_style(&self, ctx: Context<'_>) -> crate::text::TextStyle {
        crate::text::TextStyle {
            // 字号缺省不写死：0.0 会被宿主当成"非法"，由宿主回落到自己的默认字号。
            size: self.number_or(&crate::props::TEXT_SIZE, 14.0, ctx).max(1.0),
            weight: 400,
            family: None,
            line_height: {
                let h = self.number_or(&crate::props::TEXT_LINE_HEIGHT, 1.2, ctx);
                (h.is_finite() && h > 0.0).then_some(h)
            },
            scale: 1.0,
        }
    }

    /// 文本的最大宽度（`None` = 不限宽，不换行）。
    pub(crate) fn text_max_width(&self, ctx: Context<'_>) -> Option<f32> {
        finite(self.number_or(&crate::props::TEXT_MAX_WIDTH, f32::NAN, ctx))
    }

    /// 文本块的尺寸（`None` = 本节点无文本，或本进程无文本引擎）。
    ///
    /// 测量走 [`crate::text::TextEngine`] —— 核心不认识字体，所以这一步必然要
    /// 问宿主。没有引擎时返回 `None`，于是文本节点按零尺寸参与布局。
    pub fn text_size(&self, ctx: Context<'_>) -> Option<crate::core::Size> {
        self.shape(ctx).map(|s| s.size)
    }

    /// 排版一次并返回**完整**结果（尺寸 + 字形位置）。
    ///
    /// # 为什么多一个入口而不是让 `text_size` 返回整个 `ShapedText`
    ///
    /// `text_size` 出现在**很多**地方，而其中绝大多数只关心 `size`。让它返回
    /// 整个 `ShapedText` 会逼每个调用方写 `.map(|s| s.size)`，那是噪音。
    /// 而绘制趟要的是完整的那个，于是单独给一个名字。
    ///
    /// 排版结果由**绘制趟**在 `glyph_quads` 未命中时按当前文本算一次（见
    /// [`crate::paint`] 的 `text_command`）。这里只给需要完整排版的调用方一个
    /// 入口；`text_size` 只取 `.size`。
    pub fn shape(&self, ctx: Context<'_>) -> Option<crate::text::ShapedText> {
        let text = self.text()?;
        let engine = ctx.engine?;
        let style = self.text_style(ctx);
        let max_width = self.text_max_width(ctx);
        Some(engine.borrow_mut().shape(text, &style, max_width))
    }

    /// 图像纹理（`None` = 无图像内容）。
    ///
    /// 接受 `StaticStr`（编译期常量）与 `Str`（运行期字符串）两种来源——属性定义
    /// 的默认值只能是前者，但真实纹理由宿主在运行时填入。
    pub fn image_texture(&self, ctx: Context<'_>) -> Option<crate::paint::TextureRef> {
        match self.value(&crate::props::IMAGE_TEXTURE, ctx).as_deref()? {
            PropertyValue::StaticStr(s) if !s.is_empty() => {
                Some(crate::paint::TextureRef(std::borrow::Cow::Borrowed(s)))
            }
            PropertyValue::Str(s) if !s.is_empty() => Some(crate::paint::TextureRef(
                std::borrow::Cow::Owned(s.as_ref().to_owned()),
            )),
            _ => None,
        }
    }

    /// 图标名（`None` = 无矢量图标内容）。
    ///
    /// 与 [`Self::image_texture`] 同一套取值方式：`StaticStr` 与 `Str` 都接受，
    /// 空串视为"没有"。
    ///
    /// ⚠ 返回的字符串是**借来的**，而 [`crate::paint::Command::Path`] 的顶点里
    /// 不含它 —— 名字只在本次降级里用来查图标集，之后就被丢掉。所以借用是安全的。
    pub fn icon_name(&self, ctx: Context<'_>) -> Option<std::borrow::Cow<'_, str>> {
        match self.value(&crate::props::ICON_NAME, ctx).as_deref()? {
            PropertyValue::StaticStr(s) if !s.is_empty() => Some(std::borrow::Cow::Borrowed(*s)),
            PropertyValue::Str(s) if !s.is_empty() => {
                Some(std::borrow::Cow::Owned(s.as_ref().to_owned()))
            }
            _ => None,
        }
    }

    /// 图标填充色。
    pub fn icon_color(&self, ctx: Context<'_>) -> crate::core::Color {
        self.color(&crate::props::ICON_COLOR, ctx)
            .unwrap_or(crate::core::Color::rgb(1.0, 1.0, 1.0))
    }

    /// 图像源裁剪（`None` = 整张纹理）。
    ///
    /// 返回 `None` 而不是 `FULL`，是为了让 `Command::Image.src` 保持 `None` 的
    /// 语义："没裁剪"。后端据此可以跳过 uv 计算，也便于断言"这条命令没裁剪"。
    ///
    /// 越界或零尺寸的 uv 一律退化成 `None`（即整张），而不是原样转发或钳正。
    /// 转发会让采样器取到相邻图块的像素，钳正会让"我本来就想取这一块"的意图与
    /// 实际取到的区域不一致 —— 而这两种错误的症状都出现在采样之后，无从定位。
    /// 退化成整张至少让画面保持正确，让 bug 表现为"裁剪没生效"而非"花屏"。
    pub fn image_src(&self, ctx: Context<'_>) -> Option<crate::core::UvRect> {
        let uv = self
            .value(&crate::props::IMAGE_SRC, ctx)
            .and_then(|v| v.get::<crate::core::UvRect>())?;
        (uv.is_in_range() && !uv.is_full()).then_some(uv)
    }

    /// 堆叠描述。
    pub fn stack(&self, ctx: Context<'_>) -> crate::layout::Stack {
        crate::layout::Stack {
            direction: match self.value(&crate::props::DIRECTION, ctx).as_deref() {
                Some(PropertyValue::Direction(d)) => *d,
                _ => crate::layout::Direction::Horizontal,
            },
            spacing: self.number(&crate::props::SPACING, ctx),
            main_align: match self.value(&crate::props::MAIN_ALIGN, ctx).as_deref() {
                Some(PropertyValue::MainAlign(a)) => *a,
                _ => crate::layout::MainAxisAlign::Start,
            },
        }
    }

    /// 交叉轴对齐。
    pub fn cross_align(&self, ctx: Context<'_>) -> crate::layout::AlignItems {
        match self.value(&crate::props::CROSS_ALIGN, ctx).as_deref() {
            Some(PropertyValue::CrossAlign(a)) => *a,
            _ => crate::layout::AlignItems::Start,
        }
    }

    /// Flex 容器描述；`None` = 本节点**没有声明** flex，布局走 [`crate::layout::Stack`]。
    ///
    /// # 为什么查属性**表**而不是查读到的值
    ///
    /// [`Self::value`] 永远回落到键的默认值，于是「没写」与「写了默认值」拿到
    /// 同一个结果 —— 分不开。而 flex 必须是显式声明才激活的：`lookup` 一次
    /// 线性扫几个 `usize`（见 `PropertyStore` 的实现说明），对**不写 flex 的
    /// 全部既有布局**来说这就是「两次比较后原路返回」，换它们各付一次
    /// 项表扫描是不划算的。
    ///
    /// 声明方式 = 二选一（本地 `.with(...)` 或样式表都算）：
    ///
    /// - `flex-direction: …` —— 同时声明「我是 flex」与「主轴是哪根」；
    /// - `flex-wrap: …` —— 「只想要换行」的容器不必再写方向。
    ///
    /// # 方向的回落顺序
    ///
    /// 写了 `flex-direction` 就以它为准（含反向）；只写了 `flex-wrap` 时主轴
    /// 沿用 Stack 的 `direction` —— 于是 `direction: vertical; flex-wrap: wrap;`
    /// 是一列换行，与 CSS 里 `flex-direction: column; flex-wrap: wrap;` 同义。
    pub fn flex(&self, ctx: Context<'_>) -> Option<crate::layout::Flex> {
        use crate::layout::{AlignContent, Direction, FlexWrap};

        let wrap_declared = self.props().lookup(&crate::props::FLEX_WRAP).is_some();
        let direction_declared = self.props().lookup(&crate::props::FLEX_DIRECTION).is_some();
        if !wrap_declared && !direction_declared {
            return None;
        }
        let (direction, reverse) = if direction_declared {
            match self.value(&crate::props::FLEX_DIRECTION, ctx).as_deref() {
                Some(PropertyValue::FlexDirection(d)) => (d.axis(), d.is_reverse()),
                _ => (Direction::Horizontal, false),
            }
        } else {
            match self.value(&crate::props::DIRECTION, ctx).as_deref() {
                Some(PropertyValue::Direction(d)) => (*d, false),
                _ => (Direction::Horizontal, false),
            }
        };
        Some(crate::layout::Flex {
            direction,
            reverse,
            wrap: match self.value(&crate::props::FLEX_WRAP, ctx).as_deref() {
                Some(PropertyValue::FlexWrap(w)) => *w,
                _ => FlexWrap::NoWrap,
            },
            // `gap` 的 CSS 名映射到同一个键（见 zstylec::props_map），这里读 `Spacing`。
            spacing: self.number(&crate::props::SPACING, ctx),
            justify: match self.value(&crate::props::MAIN_ALIGN, ctx).as_deref() {
                Some(PropertyValue::MainAlign(a)) => *a,
                _ => crate::layout::MainAxisAlign::Start,
            },
            align_items: match self.value(&crate::props::CROSS_ALIGN, ctx).as_deref() {
                Some(PropertyValue::CrossAlign(a)) => *a,
                _ => crate::layout::AlignItems::Start,
            },
            align_content: match self.value(&crate::props::ALIGN_CONTENT, ctx).as_deref() {
                Some(PropertyValue::AlignContent(a)) => *a,
                _ => AlignContent::Stretch,
            },
        })
    }

    /// 背景色。
    pub fn background(&self, ctx: Context<'_>) -> crate::core::Color {
        self.color(&crate::props::BACKGROUND, ctx)
            .unwrap_or(crate::core::Color::rgba(0.0, 0.0, 0.0, 0.0))
    }

    /// 圆角半径。
    pub fn corner_radius(&self, ctx: Context<'_>) -> f32 {
        self.number(&crate::props::CORNER_RADIUS, ctx)
    }

    /// 描边色。
    pub fn border_color(&self, ctx: Context<'_>) -> crate::core::Color {
        self.color(&crate::props::BORDER_COLOR, ctx)
            .unwrap_or(crate::core::Color::rgba(0.0, 0.0, 0.0, 0.0))
    }

    /// 描边宽度。
    pub fn border_width(&self, ctx: Context<'_>) -> f32 {
        self.number(&crate::props::BORDER_WIDTH, ctx)
    }

    /// 整体不透明度（声明为继承）。
    ///
    /// ⚠ 名字里的"继承"指的是 [`crate::props::OPACITY`] 声明成 `inheritable`，
    /// 而**属性系统尚未接线**：`property::resolve` 的 `inherited` 参数在生产代码里
    /// 一律收到 `None`（见本文件 `value()`），所以这里取到的是**本节点**的值。
    /// 绘制趟自己沿树累乘（见 `paint.rs::PaintState::descend`），
    /// 于是**画面上的行为**是对的；不对的只有这个 getter 的名字。
    /// 记账见 `docs/dev-log/task-anim-wire.md` §0。
    pub fn opacity(&self, ctx: Context<'_>) -> f32 {
        self.number_or(&crate::props::OPACITY, 1.0, ctx)
    }

    /// 绘制时 X 偏移（DIP）。
    ///
    /// 取本节点的值（不沿父链相加）—— 累加由绘制趟负责，理由同 [`Self::opacity`]。
    pub fn translate_x(&self, ctx: Context<'_>) -> f32 {
        self.number_or(&crate::props::TRANSLATE_X, 0.0, ctx)
    }

    /// 绘制时 Y 偏移（DIP）。见 [`Self::translate_x`]。
    pub fn translate_y(&self, ctx: Context<'_>) -> f32 {
        self.number_or(&crate::props::TRANSLATE_Y, 0.0, ctx)
    }

    /// 绘制时 X 缩放（以元素中心为原点）。见 [`crate::props::SCALE_X`]。
    pub fn scale_x(&self, ctx: Context<'_>) -> f32 {
        sanitized_scale(self.number_or(&crate::props::SCALE_X, 1.0, ctx))
    }

    /// 绘制时 Y 缩放。见 [`Self::scale_x`]。
    pub fn scale_y(&self, ctx: Context<'_>) -> f32 {
        sanitized_scale(self.number_or(&crate::props::SCALE_Y, 1.0, ctx))
    }

    /// 合成层。
    pub fn layer(&self, ctx: Context<'_>) -> crate::paint::Layer {
        match self.value(&crate::props::LAYER, ctx).as_deref() {
            Some(PropertyValue::Layer(l)) => *l,
            _ => crate::paint::Layer::Content,
        }
    }

    /// 滤镜链（默认空链 = 恒等）。
    ///
    /// 走 `Clone` 而非引用：属性表里存的是 `PropertyValue`，而
    /// `FilterChain` 内部是 `Vec`，借不出`&[Filter]`——`value()` 返回的
    /// `Option<&PropertyValue>` 借到的是外层枚举。复制一条空链不分配，只有
    /// 真的有滤镜时才分配一次。
    ///
    /// # 与 `MATERIAL` 的合并
    ///
    /// `material:` 是高层表面选型，展开成一条滤镜链后**追加**到 `filter:` 的链
    /// 之后——两者读不同纹理（`filter` 的 `blur` 读元素自身、`material` 的
    /// `BackdropBlur`/`Refraction` 读背景 `g_dst`），顺序不影响结果。两者正交：
    /// 一个节点可以 `filter: drop-shadow(...)` + `material: glass`。
    ///
    /// [`crate::filter::Material::Solid`] 展开成**空链**，所以「只写了 filter」
    /// 与「没写 material」的节点，返回的链逐字节不变。
    pub fn filter(&self, ctx: Context<'_>) -> crate::filter::FilterChain {
        let mut chain = match self.value(&crate::props::FILTER, ctx).as_deref() {
            Some(PropertyValue::Filter(f)) => f.clone(),
            _ => crate::filter::FilterChain::none(),
        };
        // 材质展开成滤镜链，并进 filter 之后。Solid ⇒ 空，不进任何一项。
        if let Some(PropertyValue::Material(m)) =
            self.value(&crate::props::MATERIAL, ctx).as_deref()
        {
            // 参数是**独立**的属性（`material-params`）；没写时用种类默认值。
            let params = match self.value(&crate::props::MATERIAL_PARAMS, ctx).as_deref() {
                Some(PropertyValue::MaterialParams(p)) => *p,
                _ => crate::filter::MaterialParams::default(),
            };
            for f in m.filter_with(&params).filters() {
                chain = chain.then(*f);
            }
        }
        chain
    }

    /// 玻璃装饰（默认 [`crate::decoration::Decorations::none`]）。
    ///
    /// 与 [`Self::filter`] 同纪律：不继承、按值返回（三件全是 `Copy`）。
    /// `.zstyle` 写 `decoration: glass` / `control` / `none` 三档关键字。
    pub fn decorations(&self, ctx: Context<'_>) -> crate::decoration::Decorations {
        match self.value(&crate::props::DECORATIONS, ctx).as_deref() {
            Some(PropertyValue::Decorations(d)) => *d,
            _ => crate::decoration::Decorations::none(),
        }
    }

    /// 混合模式（默认 [`crate::filter::BlendMode::SrcOver`]）。
    pub fn blend_mode(&self, ctx: Context<'_>) -> crate::filter::BlendMode {
        match self.value(&crate::props::BLEND_MODE, ctx).as_deref() {
            Some(PropertyValue::BlendMode(b)) => *b,
            _ => crate::filter::BlendMode::default(),
        }
    }

    /// 外部渲染源（没挂时返回 `kind` 为空的「无」）。
    ///
    /// 载荷对样式系统不透明（[`PropertyValue::Custom`]）：主题解析跳过它，
    /// 这里只做本地→样式→默认的常规查找后整体取回。
    pub fn render_source(&self, ctx: Context<'_>) -> crate::paint::RenderSource {
        self.value(&crate::props::RENDER_SOURCE, ctx)
            .and_then(|v| v.get::<crate::paint::RenderSource>())
            .unwrap_or_else(crate::paint::RenderSource::none)
    }

    /// 图像边长（`-1` = 撑满节点矩形）。
    pub fn image_size(&self, ctx: Context<'_>) -> f32 {
        self.number(&crate::props::IMAGE_SIZE, ctx)
    }

    /// 图像着色。
    pub fn image_tint(&self, ctx: Context<'_>) -> crate::core::Color {
        self.color(&crate::props::IMAGE_TINT, ctx)
            .unwrap_or(crate::core::Color::rgb(1.0, 1.0, 1.0))
    }

    /// 是否可交互。
    pub fn is_interactive(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::INTERACTIVE, ctx)
    }

    /// 是否可聚焦（进入 Tab 顺序，能获得键盘焦点）。
    ///
    /// 与 [`Self::is_interactive`] 刻意分开：按钮可交互但不可聚焦（Tab 不该停在
    /// 按钮上），文本框可聚焦但命中测试靠它自己的 `INTERACTIVE` 子节点。
    pub fn is_focusable(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::FOCUSABLE, ctx)
    }

    /// 当前是否拥有键盘焦点（由 `zephyra_core::input::focus::FocusState` 写入）。
    pub fn is_focused(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::FOCUSED, ctx)
    }

    /// 是否**可激活**：拥有焦点时 Space / Enter 触发点击（见 [`crate::props::ACTIVATABLE`]）。
    pub fn is_activatable(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::ACTIVATABLE, ctx)
    }

    /// 是否禁用。禁用节点不响应指针点击，也不参与键盘激活（见 [`crate::props::DISABLED`]）。
    pub fn is_disabled(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::DISABLED, ctx)
    }

    /// 读一个颜色属性，解析其中的资源键。
    pub(crate) fn color(
        &self,
        key: &crate::property::PropertyKey,
        ctx: Context<'_>,
    ) -> Option<crate::core::Color> {
        self.value(key, ctx)
            .and_then(|v| v.get::<crate::core::Color>())
    }

    /// 读一个属性值（本地→样式→继承→默认），并按当前主题解析其中的资源键。
    ///
    /// 返回 `Cow`：非资源值直接借用（零分配），资源键才现场解析。资源键解析失败
    /// （角色名拼错）时返回 `None` ——调用方据此决定降级，而不是拿到一个黑色。
    pub fn value<'a>(
        &'a self,
        key: &'a crate::property::PropertyKey,
        ctx: Context<'_>,
    ) -> Option<std::borrow::Cow<'a, crate::property::PropertyValue>> {
        let (raw, _source) = crate::property::resolve(self.props(), key, None);
        match crate::theme::resolve_resource(ctx.theme, raw) {
            Some(resolved) => Some(resolved),
            None if raw.is_resource() => None,
            None => Some(std::borrow::Cow::Borrowed(raw)),
        }
    }

    /// 读一个数值属性，缺省 0。
    pub(crate) fn number(&self, key: &crate::property::PropertyKey, ctx: Context<'_>) -> f32 {
        self.number_or(key, 0.0, ctx)
    }

    /// 读一个数值属性，带缺省。
    ///
    /// 过滤非有限值：属性默认值用 `NaN` 表示"未设置"，若直接透传会让
    /// `NaN` 混进几何计算并污染整条布局链（下游全变 `NaN`，且不会报错）。
    pub(crate) fn number_or(
        &self,
        key: &crate::property::PropertyKey,
        fallback: f32,
        ctx: Context<'_>,
    ) -> f32 {
        self.value(key, ctx)
            .and_then(|v| v.get::<f32>())
            .filter(|v| v.is_finite())
            .unwrap_or(fallback)
    }

    /// 读一个布尔属性。
    pub(crate) fn flag(&self, key: &crate::property::PropertyKey, ctx: Context<'_>) -> bool {
        self.value(key, ctx)
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    }
}

/// `NaN` 表示"未设置"，转成 `None`。
fn finite(v: f32) -> Option<f32> {
    (v.is_finite() && v >= 0.0).then_some(v)
}

/// 绘制缩放的合法化：非有限值与 `<= 0` 一律按"不缩放"处理。
///
/// 理由见 [`crate::props::SCALE_X`]：负值是镜像，而四类图元都没有镜像语义；
/// 非有限值会让顶点变 NaN（"整块不画"，症状离病因很远）。
fn sanitized_scale(v: f32) -> f32 {
    if v.is_finite() && v > 0.0 {
        v
    } else {
        1.0
    }
}

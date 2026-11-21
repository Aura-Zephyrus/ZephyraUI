//! CSS 属性名 → `&'static PropertyKey` 的映射表。
//!
//! # 为什么 CSS 名字和 Rust 里的不一样
//!
//! CSS 惯例是 kebab-case（`corner-radius`），而 `zephyra_core::props` 用的是
//! PascalCase + 点（`CornerRadius`、`Text.Size`）。
//!
//! **用 kebab-case**，不是把 Rust 名直接搬过来。理由：「CSS 的写法 + 我们的语义」
//! 这个边界值这个成本——前端经验直接能用，而映射表本身是集中管理，
//! 加新属性只加一行。
//!
//! # 拼错怎么办
//!
//! 一律**报错**，并给出最接近的几个（见 [`super::errors`]）。绝不静默忽略——
//! 静默忽略的样式表比没有样式表更糟：设计者以为生效了。

use zephyra_core::property::PropertyKey;
use zephyra_core::props;

/// 一条映射：CSS 名 + 对应的属性键 + 「改它要不要重排」的中文说明。
///
/// # 为什么要带说明
///
/// 因为**编译期就能算出**这条声明会不会触发重排，而这对写样式表的人是真信息：
/// 写了 `text-size` 意味着改它要重排，比 `background` 贵。编译时给出来，
/// 写的时候就知道。
pub(crate) struct Prop {
    /// CSS 里的写法（kebab-case）。
    pub css: &'static str,
    pub key: &'static PropertyKey,
}

/// 全部受支持的 CSS 属性名。
///
/// ⚠️ 这张表是**唯一**的入口。加新属性 = 这里加一行，别处不用动。
/// ⚠️ 反过来，`props.rs` 里加了属性但这里没有 → 那个属性**样式表设不了**，
/// 而这不会有任何提示。所以 B2 里那条「props 与映射表对齐」的测试会红。
pub(crate) const PROPS: &[Prop] = &[
    // ---- 尺寸 / 间距（影响布局）----
    Prop {
        css: "width",
        key: &props::WIDTH,
    },
    Prop {
        css: "height",
        key: &props::HEIGHT,
    },
    Prop {
        css: "padding",
        key: &props::PADDING,
    },
    Prop {
        css: "spacing",
        key: &props::SPACING,
    },
    Prop {
        css: "collapsed",
        key: &props::COLLAPSED,
    },
    // 溢出处理：`visible` / `hidden` / `scroll` / `auto`。
    Prop {
        css: "overflow",
        key: &props::OVERFLOW,
    },
    // ---- 定位（影响布局）----
    Prop {
        css: "position",
        key: &props::POSITION,
    },
    Prop {
        css: "anchor",
        key: &props::ANCHOR,
    },
    Prop {
        css: "left",
        key: &props::LEFT,
    },
    Prop {
        css: "top",
        key: &props::TOP,
    },
    // ---- Flexbox 容器（影响布局）----
    //
    // `align-items` / `justify-content` / `gap` 与既有名字是**同一个键的两套 CSS
    // 名字**（`cross-align` / `main-align` / `spacing`）—— 语义只有一个，见
    // `zephyra_core::props` 里那段说明。
    Prop {
        css: "flex-direction",
        key: &props::FLEX_DIRECTION,
    },
    Prop {
        css: "flex-wrap",
        key: &props::FLEX_WRAP,
    },
    Prop {
        css: "align-content",
        key: &props::ALIGN_CONTENT,
    },
    Prop {
        css: "align-items",
        key: &props::CROSS_ALIGN,
    },
    Prop {
        css: "justify-content",
        key: &props::MAIN_ALIGN,
    },
    Prop {
        css: "gap",
        key: &props::SPACING,
    },
    // ---- 既有名字（与上面的 CSS 名同键）----
    Prop {
        css: "direction",
        key: &props::DIRECTION,
    },
    Prop {
        css: "cross-align",
        key: &props::CROSS_ALIGN,
    },
    Prop {
        css: "main-align",
        key: &props::MAIN_ALIGN,
    },
    // ---- Flexbox 子项（影响布局）----
    Prop {
        css: "align-self",
        key: &props::ALIGN_SELF,
    },
    Prop {
        css: "flex-grow",
        key: &props::FLEX_GROW,
    },
    Prop {
        css: "flex-shrink",
        key: &props::FLEX_SHRINK,
    },
    Prop {
        css: "flex-basis",
        key: &props::FLEX_BASIS,
    },
    Prop {
        css: "order",
        key: &props::ORDER,
    },
    // ---- 文本（影响布局）----
    Prop {
        css: "text-size",
        key: &props::TEXT_SIZE,
    },
    Prop {
        css: "text-line-height",
        key: &props::TEXT_LINE_HEIGHT,
    },
    Prop {
        css: "text-max-width",
        key: &props::TEXT_MAX_WIDTH,
    },
    // ---- 高级文字：影响测量/断行/基线 ----
    Prop {
        css: "text-letter-spacing",
        key: &props::TEXT_LETTER_SPACING,
    },
    Prop {
        css: "text-word-spacing",
        key: &props::TEXT_WORD_SPACING,
    },
    Prop {
        css: "text-paragraph-spacing",
        key: &props::TEXT_PARAGRAPH_SPACING,
    },
    Prop {
        css: "text-direction",
        key: &props::TEXT_DIRECTION,
    },
    Prop {
        css: "text-scale",
        key: &props::TEXT_SCALE,
    },
    Prop {
        css: "text-max-lines",
        key: &props::TEXT_MAX_LINES,
    },
    Prop {
        css: "text-ellipsis",
        key: &props::TEXT_ELLIPSIS,
    },
    Prop {
        css: "text-hyphens",
        key: &props::TEXT_HYPHENS,
    },
    Prop {
        css: "text-palette",
        key: &props::TEXT_PALETTE,
    },
    Prop {
        css: "text-strut",
        key: &props::TEXT_STRUT,
    },
    Prop {
        css: "text-variations",
        key: &props::TEXT_VARIATIONS,
    },
    Prop {
        css: "font-features",
        key: &props::FONT_FEATURES,
    },
    Prop {
        css: "text-path",
        key: &props::TEXT_PATH,
    },
    // ---- 只重绘 ----
    Prop {
        css: "background",
        key: &props::BACKGROUND,
    },
    Prop {
        css: "corner-radius",
        key: &props::CORNER_RADIUS,
    },
    Prop {
        css: "corner-shape",
        key: &props::CORNER_SHAPE,
    },
    Prop {
        css: "border-color",
        key: &props::BORDER_COLOR,
    },
    Prop {
        css: "border-width",
        key: &props::BORDER_WIDTH,
    },
    Prop {
        css: "opacity",
        key: &props::OPACITY,
    },
    // 绘制时偏移。**只重绘**：声明成 `PropertyKey::new`（不是 `::layout`），
    // 所以每帧改它不会触发重排 —— 那正是"绘制时插值偏移"的定义。
    Prop {
        css: "translate-x",
        key: &props::TRANSLATE_X,
    },
    Prop {
        css: "translate-y",
        key: &props::TRANSLATE_Y,
    },
    // 绘制时缩放（以元素中心为原点，只作用于本节点自身外观）。
    // 与偏移同为 `PropertyKey::new`，改它只重绘。
    Prop {
        css: "scale-x",
        key: &props::SCALE_X,
    },
    Prop {
        css: "scale-y",
        key: &props::SCALE_Y,
    },
    Prop {
        css: "layer",
        key: &props::LAYER,
    },
    Prop {
        css: "filter",
        key: &props::FILTER,
    },
    Prop {
        css: "blend-mode",
        key: &props::BLEND_MODE,
    },
    // 材质：高层表面语义（solid / frosted / glass），与 filter 正交。
    Prop {
        css: "material",
        key: &props::MATERIAL,
    },
    // 材质参数：独立于 material，覆盖 σ/厚度/倒角/色散/折射率。
    Prop {
        css: "material-params",
        key: &props::MATERIAL_PARAMS,
    },
    // 裁剪矩形：`clip-rect: x y w h`（DIP）。只重绘，不触发重排。
    Prop {
        css: "clip-rect",
        key: &props::CLIP_RECT,
    },
    // 滚动偏移（内容相对视口的偏移，DIP）。只重绘，不触发重排 ——
    // 滚动位置变了但盒子没变，正是「只重绘」的定义。
    Prop {
        css: "scroll-offset-x",
        key: &props::SCROLL_OFFSET_X,
    },
    Prop {
        css: "scroll-offset-y",
        key: &props::SCROLL_OFFSET_Y,
    },
    // 动画引用 / 进度 / 播放态：只重绘，不触发重排（时钟归应用侧 driver）。
    Prop {
        css: "animation",
        key: &props::ANIMATION,
    },
    Prop {
        css: "animation-progress",
        key: &props::ANIMATION_PROGRESS,
    },
    Prop {
        css: "animation-play-state",
        key: &props::ANIMATION_PLAY_STATE,
    },
    // 玻璃装饰：高光 / 外投影 / 内阴影（写在哪一侧由契约定，不由样式表定）。
    Prop {
        css: "decoration",
        key: &props::DECORATIONS,
    },
    // ---- 图像 / 图标 ----
    Prop {
        css: "image-texture",
        key: &props::IMAGE_TEXTURE,
    },
    Prop {
        css: "image-size",
        key: &props::IMAGE_SIZE,
    },
    Prop {
        css: "image-tint",
        key: &props::IMAGE_TINT,
    },
    Prop {
        css: "image-src",
        key: &props::IMAGE_SRC,
    },
    Prop {
        css: "icon-name",
        key: &props::ICON_NAME,
    },
    Prop {
        css: "icon-color",
        key: &props::ICON_COLOR,
    },
    // ---- 文本内容 / 颜色 ----
    Prop {
        css: "text",
        key: &props::TEXT,
    },
    Prop {
        css: "text-color",
        key: &props::TEXT_COLOR,
    },
    // ---- 高级文字：只重绘 ----
    Prop {
        css: "text-align",
        key: &props::TEXT_ALIGN,
    },
    Prop {
        css: "text-decoration",
        key: &props::TEXT_DECORATION,
    },
    Prop {
        css: "text-decoration-style",
        key: &props::TEXT_DECORATION_STYLE,
    },
    Prop {
        css: "text-decoration-mode",
        key: &props::TEXT_DECORATION_MODE,
    },
    Prop {
        css: "text-decoration-thickness",
        key: &props::TEXT_DECORATION_THICKNESS,
    },
    Prop {
        css: "text-decoration-offset",
        key: &props::TEXT_DECORATION_OFFSET,
    },
    Prop {
        css: "text-decoration-thickness-multiplier",
        key: &props::TEXT_DECORATION_THICKNESS_MULTIPLIER,
    },
    Prop {
        css: "text-embolden",
        key: &props::TEXT_EMBOLDEN,
    },
    Prop {
        css: "text-shadow",
        key: &props::TEXT_SHADOW,
    },
    Prop {
        css: "text-lcd",
        key: &props::TEXT_LCD,
    },
    Prop {
        css: "text-rotation",
        key: &props::TEXT_ROTATION,
    },
    // ---- 状态（行为，不是外观）----
    // ⚠️ 这三个虽然**不改外观**，但允许出现在样式表里：`:disabled` 一类伪类
    // 需要它们作为「选择依据」。立规禁的是「源码里写样式」，不是「样式表里设状态」。
    Prop {
        css: "interactive",
        key: &props::INTERACTIVE,
    },
    Prop {
        css: "selected",
        key: &props::SELECTED,
    },
    Prop {
        css: "disabled",
        key: &props::DISABLED,
    },
];

/// 按 CSS 名查属性键。
pub(crate) fn lookup(css_name: &str) -> Option<&'static PropertyKey> {
    PROPS.iter().find(|p| p.css == css_name).map(|p| p.key)
}

/// 全部 CSS 名（报错时列最接近的用）。
pub(crate) fn all_css_names() -> impl Iterator<Item = &'static str> {
    PROPS.iter().map(|p| p.css)
}

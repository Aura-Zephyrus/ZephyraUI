//! 布局：Measure / Arrange 协议 + 遍历节点树的两趟驱动。
//!
//! # 两趟而不是一趟
//!
//! WinUI 与 CSS 布局都分「测量」与「排列」两趟，原因是父容器必须在知道每个子项
//! 的**意图尺寸**之后才能决定如何分配空间（父给的约束通常是整块可用空间，
//! 而不是每个子项各自的目标尺寸）。
//!
//! - [`measure`]：给定约束 → 返回内容想要的尺寸（`Size`，可能小于约束）。
//! - [`arrange`]：给定最终矩形 → 摆放自身与子项。
//!
//! # 几何只有一处算法
//!
//! 命中测试（[`crate::hit`]）与绘制（[`crate::paint::render`]）读的是**同一批**
//! 已摆好的矩形，所以不会出现"看得见但点不到"。这要求两趟都写在本模块而不是
//! 各写一份。
//!
//! # 为什么 `Stack` 是静态分发而不是 `dyn`
//!
//! 布局是热路径，且每个节点的布局类型是静态已知的。静态分发让 trait object 的
//! 一次间接调用与一次胖指针解引用消失。代价是无法在运行时混排不同布局容器
//! ——而这本来也不需要（一个容器的子项布局由它自己决定）。

use crate::context::Context;
use crate::core::{Point, Rect, Size, Thickness};
use crate::node::{Dirty, Node};

/// 布局约束。
///
/// 用「允许的尺寸区间」而不是「固定尺寸」表达，因为三种情形都要覆盖：
/// 固定（容器给了确切空间）、最小（`Fill` 要的是下限）、无界（滚动内容测量）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Constraints {
    /// 允许的最小尺寸。
    pub min: Size,
    /// 允许的最大尺寸；`f32::INFINITY` 表示无界。
    pub max: Size,
}

impl Constraints {
    /// 固定尺寸约束。
    pub fn fixed(size: Size) -> Self {
        Self {
            min: size,
            max: size,
        }
    }

    /// 无界约束（滚动内容测量用）。
    pub fn unbounded() -> Self {
        Self {
            min: Size::new(0.0, 0.0),
            max: Size::new(f32::INFINITY, f32::INFINITY),
        }
    }

    /// 在给定轴上是否无界。
    pub fn is_unbounded_width(&self) -> bool {
        self.max.width.is_infinite()
    }

    /// 在给定轴上是否无界。
    pub fn is_unbounded_height(&self) -> bool {
        self.max.height.is_infinite()
    }

    /// 收紧约束（把当前区间与新的上下界求交）。
    pub fn tighten(&self, min: Size, max: Size) -> Constraints {
        Constraints {
            min: Size::new(
                self.min.width.max(min.width),
                self.min.height.max(min.height),
            ),
            max: Size::new(
                self.max.width.min(max.width),
                self.max.height.min(max.height),
            ),
        }
    }

    /// 按内边距收缩可用空间。
    pub fn deflate(&self, padding: Thickness) -> Constraints {
        Constraints {
            min: Size::new(
                (self.min.width - padding.left - padding.right).max(0.0),
                (self.min.height - padding.top - padding.bottom).max(0.0),
            ),
            max: Size::new(
                (self.max.width - padding.left - padding.right).max(0.0),
                (self.max.height - padding.top - padding.bottom).max(0.0),
            ),
        }
    }
}

/// 测量结果。
///
/// # 为什么不存排版结果（`ShapedText`）
///
/// 曾经这里带一个 `shaped: Option<ShapedText>`，让绘制趟从测量缓存里复用排版、
/// 免得每帧重排。但那条路走不通，而且是 2026-10「点击值不更新」bug 的根：
///
/// - 排版结果依赖**文本内容**，而测量缓存的键只有 [`Constraints`]。
/// - `TEXT` 被刻意判成**只重绘**属性（见 `layout_classification_tests`：内容变了
///   不等于盒子要重排），改文本既不清测量缓存、也**不触发重排** —— 于是布局趟
///   根本不跑，缓存里的 `shaped` 停留在旧文本上。
/// - 绘制趟若信它，就把旧字形算给新文本，再把新文本写进 `glyph_quads` 缓存 ⇒
///   旧字形被永久钉住（点按钮数值不变、画面不更新，resize 才自愈）。
///
/// 现在排版归**绘制趟**所有：`paint::text_command` 在 `glyph_quads` 未命中时按
/// 当前文本重排一次。判据是排版输入指纹（文本 / 样式 / `max_width`）+ 图集修订号，
/// 未命中即输入已变、重排是必需的；命中路径（绝大多数帧）零重排。
#[derive(Debug, Clone, PartialEq)]
pub struct MeasureResult {
    /// 内容想要的尺寸。
    pub size: Size,
    /// **子内容**的尺寸（DIP）。
    ///
    /// 对普通容器它约等于 `size` 减去内边距；对**滚动容器**它是内容真正需要
    /// 的空间（可以比视口大），而 `size` 是**视口**。
    ///
    /// 宿主靠它算滚动范围与滚动条拇指长度 —— 内容尺寸是**布局的产物**，
    /// 宿主在搭树时并不知道，只能等布局跑完再读。
    pub content: Size,
}

/// 可测量、可排列的节点。
///
/// 实现者持有子树并按上述两趟协议布局自身与子项。
pub trait LayoutNode {
    /// 在给定约束下测量自身与子树，返回内容想要的尺寸。
    ///
    /// **实现者不应在此处固定自身位置**——那属于 [`LayoutNode::arrange`]。
    fn measure(&mut self, constraints: Constraints) -> MeasureResult;

    /// 在最终矩形内摆放自身与子项。
    fn arrange(&mut self, rect: Rect);
}

/// 水平对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    /// 拉伸填满。
    Stretch,
    /// 起始边对齐。
    #[default]
    Start,
    /// 居中。
    Center,
    /// 末端边对齐。
    End,
}

/// 垂直对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignItems {
    /// 拉伸填满。
    Stretch,
    /// 起始边对齐。
    #[default]
    Start,
    /// 居中。
    Center,
    /// 末端边对齐。
    End,
}

/// 堆叠方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// 水平排列（对应 WinUI `StackPanel` 的 Orientation=Horizontal）。
    Horizontal,
    /// 垂直排列。
    Vertical,
}

/// 主轴对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainAxisAlign {
    /// 从起始边排布。
    #[default]
    Start,
    /// 居中。
    Center,
    /// 末端对齐。
    End,
    /// 首尾对齐（剩余空间均摊到间隔里）。
    SpaceBetween,
    /// 均匀分布（每项两侧留等宽空隙，端部半宽）。
    SpaceAround,
    /// 完全均匀（每项与间隔等宽，端部也等宽）。
    SpaceEvenly,
}

/// 交叉轴多行对齐（flex-wrap 生效时）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignContent {
    /// 拉伸填满。
    #[default]
    Stretch,
    /// 起始边对齐。
    Start,
    /// 居中。
    Center,
    /// 末端边对齐。
    End,
    /// 首尾对齐（行间空隙均匀）。
    SpaceBetween,
    /// 均匀分布（行间空隙等宽，端部半宽）。
    SpaceAround,
    /// 完全均匀（行与空隙等宽，端部也等宽）。
    SpaceEvenly,
}

/// 单项交叉轴对齐（覆盖容器的 `align-items`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignSelf {
    /// 继承容器的 `align-items`。
    #[default]
    Auto,
    /// 拉伸填满。
    Stretch,
    /// 起始边对齐。
    Start,
    /// 居中。
    Center,
    /// 末端边对齐。
    End,
}

/// Flex 换行策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexWrap {
    /// 不换行（单行）。
    #[default]
    NoWrap,
    /// 换行。
    Wrap,
    /// 换行并反向。
    WrapReverse,
}

/// Flex 方向（含反向）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexDirection {
    /// 水平左→右。
    #[default]
    Row,
    /// 水平右→左。
    RowReverse,
    /// 垂直上→下。
    Column,
    /// 垂直下→上。
    ColumnReverse,
}

impl FlexDirection {
    /// 主轴对应的 [`Direction`] —— **不含**反向（反向由 [`Self::is_reverse`] 单独表达）。
    ///
    /// 拆成两个字段，而不是给 [`Direction`] 加反向变体：`Direction` 是既有词汇，
    /// Stack 与全部既有样式表都在读它，为一个新概念去改它 = 让旧路径一起变。
    pub fn axis(self) -> Direction {
        match self {
            FlexDirection::Row | FlexDirection::RowReverse => Direction::Horizontal,
            FlexDirection::Column | FlexDirection::ColumnReverse => Direction::Vertical,
        }
    }

    /// 是否反向（`row-reverse` / `column-reverse`）。
    pub fn is_reverse(self) -> bool {
        matches!(
            self,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        )
    }
}

/// 布局环境：measure/arrange 需要的根级上下文。
///
/// `viewport` 是根/窗口客户端区尺寸（DIP）。`root` 定位与手动项的视口约束
/// 依赖它；把它显式传入而不是读全局，是为了让同一棵树在不同视口下可测、
/// 且 measure 缓存能正确失效。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutEnv {
    /// 根/窗口客户端区尺寸（DIP）。
    pub viewport: Size,
}

impl LayoutEnv {
    /// 从视口尺寸构造。
    pub const fn new(viewport: Size) -> Self {
        Self { viewport }
    }

    /// 无界/测试默认视口。
    pub const fn unbounded() -> Self {
        Self {
            viewport: Size::new(f32::INFINITY, f32::INFINITY),
        }
    }
}

impl Default for LayoutEnv {
    fn default() -> Self {
        Self::unbounded()
    }
}

/// 元素在父布局中的定位模式（D1）。
///
/// - [`Position::Flow`]：参与父布局（Stack/Flex），位置由父布局决定。
/// - [`Position::Parent`]：脱流，手动定位，参照**直接父内容区**左上角。
/// - [`Position::Root`]：脱流，手动定位，参照**根/窗口客户端区** `(0,0)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    /// 正常流（默认）。
    #[default]
    Flow,
    /// 脱流，参照直接父内容区。
    Parent,
    /// 脱流，参照根/窗口客户端区。
    Root,
}

impl Position {
    /// 是否参与正常流。
    pub const fn is_flow(self) -> bool {
        matches!(self, Self::Flow)
    }

    /// 是否为手动定位（脱流）。
    pub const fn is_manual(self) -> bool {
        !self.is_flow()
    }
}

/// 溢出处理方式（CSS `overflow`）。
///
/// # 四档的差别只有两处
///
/// | 档 | 裁剪 | 内容可滚动 |
/// |---|---|---|
/// | [`Overflow::Visible`] | 否 | 否 |
/// | [`Overflow::Hidden`] | 是 | 否（但仍可被程序滚动） |
/// | [`Overflow::Scroll`] | 是 | 是（滚动条常驻） |
/// | [`Overflow::Auto`] | 是 | 是（滚动条按需出现） |
///
/// `visible` 与其余三档的差别是**裁剪**：内容溢出时是否被切掉。
/// `hidden` 与 `scroll` / `auto` 的差别是**能不能滚**。
/// `scroll` 与 `auto` 的差别只在滚动条是否常驻 —— 那是控件层的事，
/// 核心层两者行为完全一致（可滚动 + 裁剪）。
///
/// ⚠ 本仓**没有** `overflow-x` / `overflow-y` 分轴版本：两个轴共用一档。
/// 分轴会让「内容按哪一轴测量」出现四种组合，而本仓的滚动容器
/// 两轴同源（见 [`crate::props::OVERFLOW`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overflow {
    /// 内容溢出时照常画到容器外（默认）。
    #[default]
    Visible,
    /// 内容溢出时裁到容器边界，但不可滚动。
    Hidden,
    /// 内容溢出时裁到容器边界，且可滚动（滚动条常驻）。
    Scroll,
    /// 内容溢出时裁到容器边界，且可滚动（滚动条按需出现）。
    Auto,
}

impl Overflow {
    /// 是否裁剪到容器边界。
    pub const fn clips(self) -> bool {
        !matches!(self, Self::Visible)
    }

    /// 是否可滚动。
    pub const fn scrollable(self) -> bool {
        matches!(self, Self::Scroll | Self::Auto)
    }

    /// 是否**恒有**滚动条（`scroll` 常驻，`auto` 按需）。
    pub const fn always_shows_scrollbar(self) -> bool {
        matches!(self, Self::Scroll)
    }
}

/// 手动定位的自锚点（border box 9 点，D7）。
///
/// 默认 [`Anchor::TopLeft`]，此时 `left/top` 表示元素左上角在参考系中的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Anchor {
    /// 锚点在 border box 内的归一化坐标 `(ax, ay)`，`0 = 起始`，`1 = 末端`。
    pub const fn factors(self) -> (f32, f32) {
        match self {
            Self::TopLeft => (0.0, 0.0),
            Self::TopCenter => (0.5, 0.0),
            Self::TopRight => (1.0, 0.0),
            Self::CenterLeft => (0.0, 0.5),
            Self::Center => (0.5, 0.5),
            Self::CenterRight => (1.0, 0.5),
            Self::BottomLeft => (0.0, 1.0),
            Self::BottomCenter => (0.5, 1.0),
            Self::BottomRight => (1.0, 1.0),
        }
    }

    /// 给定 border box 尺寸，返回锚点相对左上角的偏移。
    pub fn offset(self, size: Size) -> (f32, f32) {
        let (ax, ay) = self.factors();
        (size.width * ax, size.height * ay)
    }
}

/// 一维堆叠布局（WinUI `StackPanel` / CSS flex）。
///
/// 子项在交叉轴上如何摆由**排列时**的 `cross_align` 决定，而不是这里声明——
/// 这正是 `align-items` 与逐项 `align-self` 的区别。工具栏里"图标按钮贴顶、
/// 组容器撑满高度"就依赖这个区分。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stack {
    /// 排列方向。
    pub direction: Direction,
    /// 主轴上相邻子项之间的固定间距。
    pub spacing: f32,
    /// 整组在主轴上的摆放。
    pub main_align: MainAxisAlign,
}

impl Default for Stack {
    fn default() -> Self {
        Self {
            direction: Direction::Horizontal,
            spacing: 0.0,
            main_align: MainAxisAlign::Start,
        }
    }
}

impl Stack {
    /// 主轴尺寸 = 各子项之和 + 间距 × (n−1)。
    ///
    /// 空容器返回 0 而不是 `spacing`：否则调用方无从区分"没有子项"与
    /// "有一个不可见子项"。
    pub fn main_size(&self, sizes: &[Size]) -> f32 {
        if sizes.is_empty() {
            return 0.0;
        }
        let total: f32 = sizes
            .iter()
            .map(|s| match self.direction {
                Direction::Horizontal => s.width,
                Direction::Vertical => s.height,
            })
            .sum();
        total + self.spacing * (sizes.len() as f32 - 1.0)
    }

    /// 主轴起点与间距（`main_offsets` 的规则本体）。
    ///
    /// # 为什么要抽出来
    ///
    /// `main_offsets`（切片版）与 `each_rect`（流式版）需要**同一套**规则。
    /// 写两遍的话，`SpaceBetween` 的单项特例这类边角必然会漂移 —— 而漂移的表现
    /// 是"两个版本在某个对齐方式下排出来不一样"，极难归因。
    fn main_offset_rules(&self, n: usize, content: f32, available_main: f32) -> (f32, f32) {
        match self.main_align {
            MainAxisAlign::Start => (0.0, self.spacing),
            MainAxisAlign::Center => ((available_main - content) * 0.5, self.spacing),
            MainAxisAlign::End => (available_main - content, self.spacing),
            // 单项时 SpaceBetween/SpaceAround/SpaceEvenly 不应留下空隙。
            MainAxisAlign::SpaceBetween if n == 1 => (0.0, self.spacing),
            MainAxisAlign::SpaceBetween => {
                let slack = (available_main - content).max(0.0);
                (0.0, self.spacing + slack / (n as f32 - 1.0))
            }
            MainAxisAlign::SpaceAround if n == 1 => (0.0, self.spacing),
            MainAxisAlign::SpaceAround => {
                // 每项两侧留等宽空隙，端部半宽：总空隙 = 可用 - 内容，每项空隙 = 总空隙 / n
                // 但第一项起点 = 半空隙，间距 = 空隙
                let slack = (available_main - content).max(0.0);
                let gap = slack / n as f32;
                (gap * 0.5, gap)
            }
            MainAxisAlign::SpaceEvenly if n == 1 => (0.0, self.spacing),
            MainAxisAlign::SpaceEvenly => {
                // 每项与间隔等宽，端部也等宽：空隙数 = n + 1
                let slack = (available_main - content).max(0.0);
                let gap = slack / (n as f32 + 1.0);
                (gap, gap)
            }
        }
    }

    /// 该 `Size` 在堆叠轴上的长度。
    fn main_of(&self, size: Size) -> f32 {
        match self.direction {
            Direction::Horizontal => size.width,
            Direction::Vertical => size.height,
        }
    }

    /// 逐个把子项矩形交出去，**不收集中间 `Vec`**（流式版）。
    ///
    /// # 为什么需要 `I: Clone`
    ///
    /// 主轴起点（`Center` / `End` / `SpaceBetween`）依赖**总长**，而总长要第一趟
    /// 才拿得到 —— 所以要遍历两次尺寸。`Clone` 的迭代器几乎总是
    /// `slice::Iter` 或 `Cloned<…>`，克隆零成本。
    ///
    /// 换来的好处：排列趟每节点零分配。改之前是每节点一个 `Vec<Size>` 加一个
    /// 各子项在主轴上的起点（相对本容器起点）。
    ///
    /// 规则本体在 [Self::main_offset_rules] —— 切片版与流式版共用同一套，
    /// 免得 SpaceBetween 的单项特例在两处漂移。
    pub fn main_offsets(&self, sizes: &[Size], available_main: f32) -> Vec<f32> {
        let n = sizes.len();
        if n == 0 {
            return Vec::new();
        }
        let (start, gap) = self.main_offset_rules(n, self.main_size(sizes), available_main);
        let mut offsets = Vec::with_capacity(n);
        let mut cursor = start;
        for s in sizes {
            offsets.push(cursor);
            cursor += self.main_of(*s) + gap;
        }
        offsets
    }

    /// 给**子项**的约束：堆叠轴无穷，交叉轴保持原值。
    ///
    /// # 为什么只放开一根轴
    ///
    /// 交叉轴必须保持有限，否则 `cross-align: stretch` 无从谈起 ——
    /// 子项会退回"内容宽度"，横向排版全散。官方也是只放开一根
    /// （`StackPanel.cpp:24-30`）。
    ///
    /// `min` 与 `max` **都要**改：只把 `max` 设成无穷而留着 `min`，
    /// 子项会拿到"至少要 N 这么宽、上不封顶"这种自相矛盾的约束，
    /// 而 [`pick_axis`] 的下限夹逼会把结果顶到那个 `min` 上。
    ///
    /// # 为什么这一行就是 resize 便宜的**全部**原因
    ///
    /// 测量缓存按「约束 + 视口」精确匹配（见 `Node::cached_measure`）。子项在堆叠轴上拿的是
    /// 无穷，于是**只改窗口高度时，子项的约束一个字都没变** ⇒ 缓存命中 ⇒ 整段子树
    /// 跳过测量，也就不再重新整形那几百行文字。
    /// 门禁 `layout_cache_tests::height_only_change_does_not_remeasure_subtree` 钉的就是这一条。
    pub fn child_constraints(&self, deflated: Constraints) -> Constraints {
        match self.direction {
            Direction::Horizontal => Constraints {
                min: Size::new(0.0, deflated.min.height),
                max: Size::new(f32::INFINITY, deflated.max.height),
            },
            Direction::Vertical => Constraints {
                min: Size::new(deflated.min.width, 0.0),
                max: Size::new(deflated.max.width, f32::INFINITY),
            },
        }
    }

    /// 把主轴偏移落成每个子项的矩形。
    /// 把主轴偏移落成每个子项的矩形。
    ///
    /// `cross_align` 决定子项在交叉轴上的位置与伸展；[`AlignItems::Stretch`] 时
    /// 子项撑满交叉轴（工具栏的"组容器撑满高度"靠它）。
    pub fn arrange(
        &self,
        origin: Point,
        available: Size,
        sizes: &[Size],
        cross_align: AlignItems,
    ) -> Vec<Rect> {
        let available_main = match self.direction {
            Direction::Horizontal => available.width,
            Direction::Vertical => available.height,
        };
        let available_cross = match self.direction {
            Direction::Horizontal => available.height,
            Direction::Vertical => available.width,
        };
        let offsets = self.main_offsets(sizes, available_main);
        sizes
            .iter()
            .zip(offsets)
            .map(|(size, offset_main)| {
                let (cross_pos, cross_extent) = match (self.direction, cross_align) {
                    (Direction::Horizontal, AlignItems::Stretch) => (origin.y, available_cross),
                    (Direction::Horizontal, AlignItems::Start) => (origin.y, size.height),
                    (Direction::Horizontal, AlignItems::Center) => (
                        origin.y + (available_cross - size.height) * 0.5,
                        size.height,
                    ),
                    (Direction::Horizontal, AlignItems::End) => {
                        (origin.y + available_cross - size.height, size.height)
                    }
                    (Direction::Vertical, AlignItems::Stretch) => (origin.x, available_cross),
                    (Direction::Vertical, AlignItems::Start) => (origin.x, size.width),
                    (Direction::Vertical, AlignItems::Center) => {
                        (origin.x + (available_cross - size.width) * 0.5, size.width)
                    }
                    (Direction::Vertical, AlignItems::End) => {
                        (origin.x + available_cross - size.width, size.width)
                    }
                };
                match self.direction {
                    Direction::Horizontal => {
                        Rect::new(origin.x + offset_main, cross_pos, size.width, cross_extent)
                    }
                    Direction::Vertical => {
                        Rect::new(cross_pos, origin.y + offset_main, cross_extent, size.height)
                    }
                }
            })
            .collect()
    }
}

/// Flexbox 容器（CSS 的 `display: flex` / WinUI `FlexLayout` 的那套语义）。
///
/// # 与 [`Stack`] 的分工，以及**为什么不是把 flex 并进 Stack**
///
/// [`Stack`] 是「一次排完、零分配」的快路径，全部既有布局走的都是它 ——
/// 那条路径的每节点零分配是 resize 实测钉住的（见 [`arrange`] 的分配记录）。
/// Flex 需要中间结构（项表、行划分、分配结果），并进 Stack 等于让每个普通容器
/// 都付这笔代价。所以做成**显式声明才进入**的第二条路径：容器上写了
/// `flex-direction` 或 `flex-wrap` 之一才构造本结构（见 `Node::flex`），
/// 否则一次 `props().lookup` 就返回 `None`。
///
/// # CSS 属性对照
///
/// | CSS | 本仓 |
/// |---|---|
/// | `flex-direction` | [`Flex::direction`] + [`Flex::reverse`]（**另一个**键 `direction` 只在未声明 flex-direction 时兜底） |
/// | `flex-wrap` | [`Flex::wrap`] |
/// | `gap` / `justify-content` / `align-items` | [`Flex::spacing`] / [`Flex::justify`] / [`Flex::align_items`] —— 即 `Spacing` / `MainAlign` / `CrossAlign`，CSS 名映射到同一键 |
/// | `align-content` | [`Flex::align_content`] |
/// | `flex-grow` / `flex-shrink` / `flex-basis` / `order` / `align-self` | 项表（`FlexItem`，私有）同名字段 |
///
/// # 两趟之间不留状态
///
/// 测量趟算完内容尺寸就结束；排列趟从**测量缓存**（子项的 `Size`）与属性表
/// 重新推出行划分与分配。中间不落任何字段 —— 代价是两趟各算一遍行划分，
/// 换来的是「属性一改就重排」不需要额外的失效通知：布局相关属性改了会清
/// 测量缓存（见 `crate::node::mark_dirty`），本趟自然重算。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flex {
    /// 主轴方向（**不含**反向；反向由 [`Flex::reverse`] 单独表达）。
    pub direction: Direction,
    /// 主轴反向（`row-reverse` / `column-reverse`）。
    pub reverse: bool,
    /// 换行策略（`nowrap` / `wrap` / `wrap-reverse`）。
    pub wrap: FlexWrap,
    /// 主轴上相邻子项的间距（即 `Spacing`，CSS 的 `gap` 映射到它）。
    pub spacing: f32,
    /// 主轴对齐（即 `MainAlign`，CSS 的 `justify-content` 映射到它）。
    pub justify: MainAxisAlign,
    /// 子项交叉轴默认对齐（即 `CrossAlign`，CSS 的 `align-items` 映射到它）。
    pub align_items: AlignItems,
    /// 多行时行组在交叉轴上的分布（单行时不生效，见私有的 `line_boxes`）。
    pub align_content: AlignContent,
}

/// 一个参与 flex 排布的子项：测量结果 + 它在属性表里的 flex 参数。
///
/// 存**下标**而不是 `&Node`：排列趟要先按 `order` 排序、再按行分组，之后还要
/// 递归 `&mut` 进每个子节点 —— 下标让「读子节点」与「改子节点」两次借用
/// 互不重叠（`&Node` 会把整个 `children` 借死）。
struct FlexItem {
    /// 在 `node.children()` 里的下标（折叠项不入表，所以下标不连续）。
    idx: usize,
    /// 测量尺寸（交叉轴尺寸取自它）。
    size: Size,
    /// flex-basis：`auto`（NaN）已解析成测量出的主轴尺寸。
    basis: f32,
    /// `flex-grow`（默认 0，钳到 ≥ 0）。
    grow: f32,
    /// `flex-shrink`（默认 1，钳到 ≥ 0）。
    shrink: f32,
    /// `align-self`（默认 [`AlignSelf::Auto`] = 跟随容器的 `align-items`）。
    align_self: AlignSelf,
    /// `order`（默认 0；相等时保持文档顺序 —— 排序用的是稳定排序）。
    order: i32,
}

/// 一行 flex 项（[`Flex::lines`] 的产物）。
#[derive(Debug)]
struct FlexLine {
    /// 行内各项（[`FlexItem`] 的下标，已按 `order` 排好）。
    items: Vec<usize>,
    /// 该行的交叉轴内容尺寸 = 行内最大的项交叉尺寸。
    cross: f32,
}

impl Flex {
    /// 主轴是否水平。
    fn main_is_horizontal(&self) -> bool {
        matches!(self.direction, Direction::Horizontal)
    }

    /// 主轴长度。
    fn main_of(&self, size: Size) -> f32 {
        if self.main_is_horizontal() {
            size.width
        } else {
            size.height
        }
    }

    /// 交叉轴长度。
    fn cross_of(&self, size: Size) -> f32 {
        if self.main_is_horizontal() {
            size.height
        } else {
            size.width
        }
    }

    /// 同一套主轴规则（间距 + justify）—— 刻意复用 [`Stack`]，不另写一份。
    ///
    /// 「两套实现在某个对齐方式下排出来不一样」是这类布局最贵的 bug 形态，
    /// 理由与 [`Stack::main_offset_rules`] 上那条相同。
    fn stack(&self) -> Stack {
        Stack {
            direction: self.direction,
            spacing: self.spacing,
            main_align: self.justify,
        }
    }

    /// 给子项的约束：主轴无界（分配由 basis/grow/shrink 决定），交叉轴保持有限。
    ///
    /// 与 [`Stack::child_constraints`] 同一条理由：交叉轴不给有限区间的话
    /// `align-items: stretch` 无从谈起；主轴放开还让子项的测量缓存跨帧复用 ——
    /// 只改容器尺寸时子项约束一个字没变（同
    /// `layout_cache_tests::height_only_change_does_not_remeasure_subtree` 那条）。
    pub fn child_constraints(&self, deflated: Constraints) -> Constraints {
        self.stack().child_constraints(deflated)
    }

    /// 换行判定用的主轴可用长（无界 ⇒ `INFINITY` ⇒ 永不换行）。
    fn available_main(&self, deflated: Constraints) -> f32 {
        if self.main_is_horizontal() {
            deflated.max.width
        } else {
            deflated.max.height
        }
    }

    /// 读一个子项的 flex 参数。测量趟与排列趟**共用这一份读法** ——
    /// 两边读出不同值的症状是「量出来一套、摆出来另一套」，且不会报错。
    fn read_item(&self, idx: usize, size: Size, child: &Node, ctx: Context<'_>) -> FlexItem {
        // `number_or` 会挡掉非有限值再回落到 fallback：这里的 fallback 就是 NaN，
        // 于是「没写 flex-basis」与「写了 auto」同样是 NaN，随后统一解析成内容尺寸。
        let basis = child.number_or(&crate::props::FLEX_BASIS, f32::NAN, ctx);
        FlexItem {
            idx,
            size,
            basis: if basis.is_finite() {
                basis.max(0.0)
            } else {
                self.main_of(size)
            },
            grow: child.number_or(&crate::props::FLEX_GROW, 0.0, ctx).max(0.0),
            shrink: child
                .number_or(&crate::props::FLEX_SHRINK, 1.0, ctx)
                .max(0.0),
            align_self: match child.value(&crate::props::ALIGN_SELF, ctx).as_deref() {
                Some(crate::property::PropertyValue::AlignSelf(a)) => *a,
                _ => AlignSelf::Auto,
            },
            // 样式表侧的数字统一解析成 `Float`（zstylec `value.rs`），Rust 侧可能写
            // `Int` —— 两种都收。只认一种的症状是「样式表里 `order` 生效、代码里
            // `.with(&props::ORDER, …)` 写了没人理」。
            order: match child.value(&crate::props::ORDER, ctx).as_deref() {
                Some(crate::property::PropertyValue::Int(v)) => *v,
                Some(crate::property::PropertyValue::Float(v)) => *v as i32,
                _ => 0,
            },
        }
    }

    /// 把（已按 `order` 排好的）项按 `flex-wrap` 切成行。
    ///
    /// # 无界主轴下**永不换行**
    ///
    /// CSS 的换行判定要的是「容器有确定的主轴长」；无限长里放多少都放得下，
    /// 此时给判定结果只会凭空多出几行 —— 而症状（内容被折成窄条）离原因极远。
    fn lines(&self, items: &[FlexItem], available_main: f32) -> Vec<FlexLine> {
        let wrap = self.wrap != FlexWrap::NoWrap && available_main.is_finite();
        if !wrap {
            if items.is_empty() {
                return Vec::new();
            }
            return vec![FlexLine {
                items: (0..items.len()).collect(),
                cross: items
                    .iter()
                    .map(|i| self.cross_of(i.size))
                    .fold(0.0, f32::max),
            }];
        }

        let mut lines: Vec<FlexLine> = Vec::new();
        let mut cur: Vec<usize> = Vec::new();
        let mut cur_main = 0.0f32;
        let mut cur_cross = 0.0f32;
        for (idx, item) in items.iter().enumerate() {
            let needed = if cur.is_empty() {
                item.basis
            } else {
                cur_main + self.spacing + item.basis
            };
            // 已有内容才折：单个超宽项自己占一整行，而不是先折出一个空行。
            if !cur.is_empty() && needed > available_main {
                lines.push(FlexLine {
                    items: std::mem::take(&mut cur),
                    cross: cur_cross,
                });
                cur_main = 0.0;
                cur_cross = 0.0;
            }
            cur_main = if cur.is_empty() {
                item.basis
            } else {
                cur_main + self.spacing + item.basis
            };
            cur_cross = cur_cross.max(self.cross_of(item.size));
            cur.push(idx);
        }
        if !cur.is_empty() {
            lines.push(FlexLine {
                items: cur,
                cross: cur_cross,
            });
        }
        lines
    }

    /// 一行内各项的最终主轴尺寸：`flex-grow` 分剩余、`flex-shrink` 补缺口。
    ///
    /// # 分配规则（CSS §9.7 的等价实现）
    ///
    /// 1. 先按 `basis` 得到「假设尺寸」；
    /// 2. 有剩余空间 ⇒ 只在 `grow > 0` 的项里按 grow 比例分（`grow = 0` 的项保持
    ///    basis，剩余空间全无时 `justify-content` 再去处理剩下的空隙）；
    /// 3. 空间不足 ⇒ 在 `shrink > 0` 的项里按 `shrink × basis`（**视觉收缩率**，
    ///    CSS 的 scaled shrink factor）分配；缩到 0 的项当场**冻结**，剩余份额
    ///    重分给别的项 —— 这一步必须循环，否则会出现「总和仍超、但没人再缩」
    ///    的残留溢出。每轮至少冻结一项，所以上界就是项数，不会不收敛。
    ///
    /// # 无界主轴直接返回 basis
    ///
    /// `inf − 总长 = inf`，让 grow 去分一个无穷大的剩余空间会把每项撑成 `inf`，
    /// 而症状（整棵子树 NaN、画面无声消失）离原因极远。
    fn resolve_main(&self, items: &[FlexItem], line: &FlexLine, available_main: f32) -> Vec<f32> {
        let idxs = &line.items;
        if idxs.is_empty() {
            return Vec::new();
        }
        let mut sizes: Vec<f32> = idxs.iter().map(|&i| items[i].basis).collect();
        if !available_main.is_finite() {
            return sizes;
        }

        let free = available_main - sizes.iter().sum::<f32>();
        if free > 0.0 {
            let grow_total: f32 = idxs
                .iter()
                .filter(|&&i| items[i].grow > 0.0)
                .map(|&i| items[i].grow)
                .sum();
            if grow_total > 0.0 {
                for (k, &i) in idxs.iter().enumerate() {
                    if items[i].grow > 0.0 {
                        sizes[k] += free * (items[i].grow / grow_total);
                    }
                }
            }
            return sizes;
        }
        if free < 0.0 {
            let mut frozen = vec![false; idxs.len()];
            for _ in 0..=idxs.len() {
                let total: f32 = sizes.iter().sum();
                let free = available_main - total;
                if free >= 0.0 {
                    break;
                }
                let weight: f32 = idxs
                    .iter()
                    .enumerate()
                    .filter(|(k, &i)| !frozen[*k] && items[i].shrink > 0.0)
                    .map(|(_, &i)| items[i].shrink * items[i].basis)
                    .sum();
                if weight <= 0.0 {
                    break;
                }
                let mut clamped = false;
                for (k, &i) in idxs.iter().enumerate() {
                    if frozen[k] || items[i].shrink <= 0.0 {
                        continue;
                    }
                    sizes[k] += free * (items[i].shrink * items[i].basis / weight);
                    if sizes[k] <= 0.0 {
                        sizes[k] = 0.0;
                        frozen[k] = true;
                        clamped = true;
                    }
                }
                // 没有项触底 ⇒ 这一轮已按比例分完，下一轮 free ≥ 0 即退出。
                if !clamped {
                    break;
                }
            }
        }
        sizes
    }

    /// 每行在交叉轴上的 `(起点, 长度)`。
    ///
    /// # 单行的行长度 = 容器内交叉轴长
    ///
    /// `align-items` 的居中 / 末端 / 拉伸都要一个基准：单行时这个基准就是容器
    /// 本身（与 [`Stack`] 路径逐字一致 —— 那条路径的 `Center` 同样按
    /// `available_cross` 算）。多行时才轮到 `align-content` 把行组分配进容器。
    ///
    /// [`AlignContent::Stretch`] 把剩余空间**均摊给每一行**（行随容器变高），
    /// 其余取值只挪动行组、不改行长。
    fn line_boxes(&self, lines: &[FlexLine], available_cross: f32) -> Vec<(f32, f32)> {
        if lines.len() <= 1 {
            return vec![(0.0, available_cross)];
        }
        let n = lines.len() as f32;
        let content: f32 = lines.iter().map(|l| l.cross).sum::<f32>() + self.spacing * (n - 1.0);
        let slack = (available_cross - content).max(0.0);
        let (start, gap, extra) = match self.align_content {
            AlignContent::Start => (0.0, self.spacing, 0.0),
            AlignContent::Center => (slack * 0.5, self.spacing, 0.0),
            AlignContent::End => (slack, self.spacing, 0.0),
            AlignContent::SpaceBetween => (0.0, self.spacing + slack / (n - 1.0), 0.0),
            AlignContent::SpaceAround => {
                let g = slack / n;
                (g * 0.5, g, 0.0)
            }
            AlignContent::SpaceEvenly => {
                let g = slack / (n + 1.0);
                (g, g, 0.0)
            }
            AlignContent::Stretch => (0.0, self.spacing, slack / n),
        };
        let mut out = Vec::with_capacity(lines.len());
        let mut cursor = start;
        for line in lines {
            let extent = line.cross + extra;
            out.push((cursor, extent));
            cursor += extent + gap;
        }
        out
    }

    /// 单项在交叉轴上的 `(起点, 长度)`：`align-self` 覆盖容器的 `align-items`。
    ///
    /// 拉伸落到**行**的长度而不是容器：单行时两者相同，多行时「拉伸到本行最高项」
    /// 才是 CSS 的语义（行高由 [`Flex::line_boxes`] 定）。
    fn item_cross(&self, item: &FlexItem, line_start: f32, line_extent: f32) -> (f32, f32) {
        let align = match item.align_self {
            AlignSelf::Auto => self.align_items,
            AlignSelf::Stretch => AlignItems::Stretch,
            AlignSelf::Start => AlignItems::Start,
            AlignSelf::Center => AlignItems::Center,
            AlignSelf::End => AlignItems::End,
        };
        let measured = self.cross_of(item.size);
        match align {
            AlignItems::Stretch => (line_start, line_extent),
            AlignItems::Start => (line_start, measured),
            AlignItems::Center => (line_start + (line_extent - measured) * 0.5, measured),
            AlignItems::End => (line_start + line_extent - measured, measured),
        }
    }
}

/// 在给定空间内按对齐方式摆放一个已测量的盒子，返回它的最终矩形。
///
/// 抽成自由函数是因为栈布局、横/纵布局、以及各种居中容器全都用同一套算法，
/// 各自实现一遍必然出现细微不一致。
pub fn align_box(content: Size, available: Size, horizontal: Align, vertical: AlignItems) -> Rect {
    let (x, width) = match horizontal {
        Align::Stretch => (0.0, available.width),
        Align::Start => (0.0, content.width),
        Align::Center => ((available.width - content.width) * 0.5, content.width),
        Align::End => (available.width - content.width, content.width),
    };
    let (y, height) = match vertical {
        AlignItems::Stretch => (0.0, available.height),
        AlignItems::Start => (0.0, content.height),
        AlignItems::Center => ((available.height - content.height) * 0.5, content.height),
        AlignItems::End => (available.height - content.height, content.height),
    };
    Rect::new(x, y, width, height)
}

// ==================================================================
// 以下是遍历 `Node` 树的两趟驱动。
// ==================================================================

/// 布局整棵子树：测量 + 按最终矩形摆放每个子节点。
///
/// 这是 [`crate::paint::render`] 的前置步骤——两者必须成对调用，因为 `render` 只读
/// 已摆好的矩形，不再自己算几何。
pub fn layout(node: &mut Node, bounds: Rect, ctx: Context<'_>) {
    let env = LayoutEnv::new(Size::new(bounds.width, bounds.height));
    // 用**上界**而非固定约束测量：固定约束的 min == max，会把每个节点都撑成
    // 窗口大小，于是所有 `Width` / `Height` 全部失效（子树内容再大也不缩）。
    // 根节点最终仍由 `arrange` 铺满 `bounds` —— 那是"填满窗口"的正确语义，
    // 但测量阶段必须允许内容小于窗口。
    measure(
        node,
        Constraints {
            min: Size::new(0.0, 0.0),
            max: Size::new(bounds.width, bounds.height),
        },
        &env,
        ctx,
    );
    arrange(node, bounds, &env, ctx);
}

/// 测量并在必要时重排节点树，返回整棵树的内容尺寸。
///
/// 测量结果按「约束 + 视口 → 结果」缓存：约束与视口不变则直接命中。
pub fn measure(
    node: &mut Node,
    constraints: Constraints,
    env: &LayoutEnv,
    ctx: Context<'_>,
) -> Size {
    if !node.subtree_needs_layout() {
        if let Some(size) = node.cached_measure(constraints, env.viewport) {
            return size;
        }
    }
    let padding = node.padding(ctx);
    let (own_w, own_h) = explicit_size(node, ctx);
    // 滚动容器（`overflow: scroll / auto`）：视口尺寸由**显式** `width` / `height`
    // 决定（没有显式尺寸时容器就等于内容大小，也就没什么可滚的）。
    //
    // ⚠ 这与 CSS 一致：`overflow` 不改变元素自己的尺寸，只改变**内容**的处理方式。
    let scrollable = node.overflow(ctx).scrollable();
    let deflated = constraints.deflate(padding);
    let mut content = match node.flex(ctx) {
        Some(flex) => measure_flex(node, constraints, flex, scrollable, env, ctx),
        None => {
            let stack = node.stack(ctx);
            // ⚠ `measure_stack` 收到的是**已扣内边距**的约束（它自己不扣）。
            let child_c = if scrollable {
                let viewport = Size::new(
                    own_w.unwrap_or(deflated.max.width),
                    own_h.unwrap_or(deflated.max.height),
                );
                scroll_child_constraints(stack.direction, deflated, viewport)
            } else {
                deflated
            };
            measure_stack(node, child_c, env, ctx)
        }
    };

    let shaped = node.shape(ctx);
    if let Some(shaped) = &shaped {
        content = shaped.size;
    }
    let (own_w, own_h) = explicit_size(node, ctx);
    let result = MeasureResult {
        size: Size::new(
            pick_axis(own_w, content.width, Axis::Horizontal, constraints, padding),
            pick_axis(own_h, content.height, Axis::Vertical, constraints, padding),
        ),
        content,
    };
    let size = result.size;
    node.cache_measure(constraints, env.viewport, result);
    node.merge_dirty(Dirty::Layout);
    size
}

/// 按最终矩形摆放整棵子树。
///
/// 顺序无关紧要：绘制顺序由 [`crate::paint::render`] 的 `order` 决定，这里只算几何。
///
/// # 零分配（改之前每节点三次）
///
/// 原写法是「收集可见子项 `Vec<usize>` → 收集尺寸 `Vec<Size>` → 收集矩形
/// `Vec<Rect>`」三步。而这三条**没有一条需要真的收集**：
///
/// - 主轴起点（`Center` / `End` / `SpaceBetween`）只依赖**总长**，一趟折叠就有；
/// - 矩形算出来立刻递归下去用，中间不留存。
///
/// resize 实测每帧 1709 次分配、约合每节点 2.8 次（`--bench-resize --alloc-probe`）。
///
/// # 为什么不用 `Stack::each_rect` 那样的流式回调
///
/// 递归要的是 `&mut Node`，而尺寸要从同一个节点的**子节点**里读 ——
/// 闭包迭代器会同时借走 `&Node`，与随后的 `&mut` 冲突。所以两趟都写成显式循环：
/// 第一趟只读（折叠总长），第二趟可变（算矩形 + 递归）。
///
/// # 缺尺寸时占零而不是消失
///
/// 尺寸取自测量缓存。缓存缺失时填 `0` **占位**而不是 `filter_map` 掉 ——
/// 掉一个会让后面所有下标整体前移，那是 `paint` 侧一个真实踩过的坑
/// （见 `docs/dev-log/task-resize.md` §完成情况）。
pub fn arrange(node: &mut Node, bounds: Rect, env: &LayoutEnv, ctx: Context<'_>) {
    node.set_rect(bounds);
    let padding = node.padding(ctx);
    let inner = bounds.inset(padding);
    if let Some(flex) = node.flex(ctx) {
        // 滚动容器：主轴无界 —— 与测量趟同源（那边 `available_main = INFINITY`）。
        // 两趟不同源的后果是「测量时一行、排列时两行」⇒ 行高算错。
        let inner = if node.overflow(ctx).scrollable() {
            if flex.main_is_horizontal() {
                Rect::new(inner.x, inner.y, f32::INFINITY, inner.height)
            } else {
                Rect::new(inner.x, inner.y, inner.width, f32::INFINITY)
            }
        } else {
            inner
        };
        arrange_flex(node, inner, flex, env, ctx);
        return;
    }
    let stack = node.stack(ctx);
    let cross_align = node.cross_align(ctx);
    let available = Size::new(inner.width, inner.height);
    let origin = Point::new(inner.x, inner.y);

    let mut main_sum = 0.0f32;
    let mut count = 0usize;
    for c in node.children().iter() {
        if c.is_collapsed(ctx) || c.is_manual(ctx) {
            continue;
        }
        main_sum += stack.main_of(cached_size_or_zero(c));
        count += 1;
    }
    if count == 0 {
        arrange_flow(
            node,
            stack,
            cross_align,
            origin,
            available,
            0.0,
            0.0,
            env,
            ctx,
        );
        return;
    }
    let content_main = main_sum + stack.spacing * (count as f32 - 1.0);
    let (start, gap) = stack.main_offset_rules(count, content_main, stack.main_of(available));
    arrange_flow(
        node,
        stack,
        cross_align,
        origin,
        available,
        start,
        gap,
        env,
        ctx,
    );
}

/// 第二趟：按正常流游标摆放子项；绝对子项逐轴赋值。
///
/// # 逐轴语义（「X、Y 可单独赋值或计算」）
///
/// 绝对子项的每个轴独立决定：
///
/// - 该轴设了 `left` / `top` ⇒ **直接赋值**该窗口坐标；
/// - 该轴没设 ⇒ 用它在**正常流里本会落在的位置**（CSS 的 static position）。
///
/// 于是 `position: absolute; left: 120px;`（只给 X）会横向钉到 120、
/// 纵向仍跟着兄弟往下排——这正是 CSS 的行为，也是「X、Y 单独赋值或计算」。
/// 绝对子项**不推进正常流游标**（它不占位）。
// `too_many_arguments` 是刻意放过的：8 个参数是布局一趟的**全部输入**——节点、
// 栈方向/交叉对齐、原点、可用尺寸、游标起点、间隙、上下文。把它们各自包成结构体
// 会新增一层中间类型，且与「arrange 参数就是测量趟的输出」的语义相悖。
#[allow(clippy::too_many_arguments)]
fn arrange_flow(
    node: &mut Node,
    stack: Stack,
    cross_align: AlignItems,
    origin: Point,
    available: Size,
    start: f32,
    gap: f32,
    env: &LayoutEnv,
    ctx: Context<'_>,
) {
    let available_cross = match stack.direction {
        Direction::Horizontal => available.height,
        Direction::Vertical => available.width,
    };
    let mut cursor = start;
    for child in node.children_mut().iter_mut() {
        if child.is_collapsed(ctx) {
            continue;
        }
        let size = cached_size_or_zero(child);
        if child.is_manual(ctx) {
            let rect = manual_rect(child, size, origin, ctx);
            arrange(child, rect, env, ctx);
            continue;
        }
        let (cross_pos, cross_extent) = match (stack.direction, cross_align) {
            (Direction::Horizontal, AlignItems::Stretch) => (origin.y, available_cross),
            (Direction::Horizontal, AlignItems::Start) => (origin.y, size.height),
            (Direction::Horizontal, AlignItems::Center) => (
                origin.y + (available_cross - size.height) * 0.5,
                size.height,
            ),
            (Direction::Horizontal, AlignItems::End) => {
                (origin.y + available_cross - size.height, size.height)
            }
            (Direction::Vertical, AlignItems::Stretch) => (origin.x, available_cross),
            (Direction::Vertical, AlignItems::Start) => (origin.x, size.width),
            (Direction::Vertical, AlignItems::Center) => {
                (origin.x + (available_cross - size.width) * 0.5, size.width)
            }
            (Direction::Vertical, AlignItems::End) => {
                (origin.x + available_cross - size.width, size.width)
            }
        };
        let flow = match stack.direction {
            Direction::Horizontal => {
                Rect::new(origin.x + cursor, cross_pos, size.width, cross_extent)
            }
            Direction::Vertical => {
                Rect::new(cross_pos, origin.y + cursor, cross_extent, size.height)
            }
        };
        arrange(child, flow, env, ctx);
        cursor += stack.main_of(size) + gap;
    }
}

fn manual_rect(child: &Node, size: Size, parent_content_origin: Point, ctx: Context<'_>) -> Rect {
    let pos = child.position(ctx);
    let anchor = child.anchor(ctx);
    let (ax, ay) = anchor.factors();
    let ref_origin = match pos {
        Position::Root => Point::new(0.0, 0.0),
        Position::Parent | Position::Flow => parent_content_origin,
    };
    let explicit_left = child.props().lookup(&crate::props::LEFT).is_some();
    let explicit_top = child.props().lookup(&crate::props::TOP).is_some();
    let x = if explicit_left {
        let coord = child.number_or(&crate::props::LEFT, 0.0, ctx);
        coord - size.width * ax
    } else {
        ref_origin.x
    };
    let y = if explicit_top {
        let coord = child.number_or(&crate::props::TOP, 0.0, ctx);
        coord - size.height * ay
    } else {
        ref_origin.y
    };
    Rect::new(x, y, size.width, size.height)
}

fn arrange_manual_children(
    node: &mut Node,
    parent_content_origin: Point,
    env: &LayoutEnv,
    ctx: Context<'_>,
) {
    for child in node.children_mut().iter_mut() {
        if child.is_collapsed(ctx) || !child.is_manual(ctx) {
            continue;
        }
        let size = cached_size_or_zero(child);
        let rect = manual_rect(child, size, parent_content_origin, ctx);
        arrange(child, rect, env, ctx);
    }
}

/// Stack 快路径的子项测量：主轴一条和 + 交叉轴一条最大边，**不收集中间 `Vec`**。
///
/// # 子项约束只放开堆叠轴（官方依据）
///
/// microsoft-ui-xaml 的 dxaml/xcp/core/core/elements/StackPanel.cpp:24-30：
///
/// ```text
/// XSIZEF childConstraint = (m_orientation == DirectUI::Orientation::Vertical)
///     ? XSIZEF{ availableSize.width - combinedThickness.width, XFLOAT_INF }
///     : XSIZEF{ XFLOAT_INF, availableSize.height - combinedThickness.height };
/// ```
///
/// 而这个子项约束的缓存是**跨帧**存活的（见 `Node::cached_measure` 与
/// [`crate::node::clear_dirty`] 的文档）—— 两条合起来才是 resize 便宜的
/// 全部原因：只改高度时约束一个字没变，于是整段子树跳过测量。
fn measure_stack(
    node: &mut Node,
    deflated: Constraints,
    env: &LayoutEnv,
    ctx: Context<'_>,
) -> Size {
    let stack = node.stack(ctx);
    let child_constraints = stack.child_constraints(deflated);

    let mut main_sum = 0.0f32;
    let mut cross_max = 0.0f32;
    let mut n = 0usize;
    for i in 0..node.children().len() {
        if node.children()[i].is_collapsed(ctx) {
            continue;
        }
        let manual = node.children()[i].is_manual(ctx);
        let s = measure(&mut node.children_mut()[i], child_constraints, env, ctx);
        if manual {
            continue;
        }
        main_sum += stack.main_of(s);
        cross_max = cross_max.max(match stack.direction {
            Direction::Horizontal => s.height,
            Direction::Vertical => s.width,
        });
        n += 1;
    }
    if n == 0 {
        Size::new(0.0, 0.0)
    } else {
        let main = main_sum + stack.spacing * (n as f32 - 1.0);
        match stack.direction {
            Direction::Horizontal => Size::new(main, cross_max),
            Direction::Vertical => Size::new(cross_max, main),
        }
    }
}

/// 滚动容器的**子项**约束。
///
/// - **主轴**（滚动方向）：无界 —— 内容可以比视口大，超出部分被裁剪并可滚动。
/// - **交叉轴**：按容器**自己的视口尺寸**收紧（没有显式尺寸就沿用父给的）。
///
/// # 交叉轴为什么也要收紧
///
/// 视口宽 300 而父宽 800 时，内容必须按 **300** 排版。若沿用父的 800，
/// 内容会比视口宽出两倍多 —— 横向滚动时用户得滚很久才看完一项，
/// 而且**竖直方向**的滚动条也永远不会出现（内容高度被宽度挤出来了）。
///
/// # 与 [`Stack::child_constraints`] 的关系
///
/// 本函数的结果**再过一遍** `Stack::child_constraints` 也不会变：那条路把主轴
/// 设成无界、交叉轴原样保留，而这里给的正是「主轴无界 + 交叉轴已收紧」。
#[must_use]
pub fn scroll_child_constraints(
    direction: Direction,
    deflated: Constraints,
    viewport: Size,
) -> Constraints {
    let cap = |parent_max: f32, viewport: f32| viewport.min(parent_max);
    match direction {
        // 横向滚动：交叉轴是**高**，按视口高度收紧。
        Direction::Horizontal => Constraints {
            min: Size::new(0.0, deflated.min.height),
            max: Size::new(f32::INFINITY, cap(deflated.max.height, viewport.height)),
        },
        // 纵向滚动：交叉轴是**宽**，按视口宽度收紧。
        Direction::Vertical => Constraints {
            min: Size::new(deflated.min.width, 0.0),
            max: Size::new(cap(deflated.max.width, viewport.width), f32::INFINITY),
        },
    }
}

/// flex 容器的测量趟：逐项测量 → 按 `order` 排 → 切行 → 聚合成内容尺寸。
///
/// 与 [`measure_stack`] 的差别只在**聚合方式**：
///
/// - 主轴：不换行 = basis 之和（与 Stack 同型）；换行 = **最大行宽** ——
///   容器宽到装下最宽那行就够，CSS 同义。
/// - 交叉轴：不换行 = 最大项交叉尺寸；换行 = 各行之和 + 行间距。
///
/// `align-content` 不进这里：它分配的是**容器已经给出的**交叉轴空间，
/// 不改变容器想要的尺寸。
///
/// # 分配为什么不能发生在测量趟
///
/// grow/shrink 回答的是「容器给多少就吃多少」，而测量趟只回答「内容想要多大」。
/// 把分配塞进测量趟的症状是：容器尺寸随 grow 变 ⇒ 下一帧约束变 ⇒ 缓存永不
/// 命中 ⇒ 静止界面每帧全树重排。
///
/// # 折行宽度必须与排列趟的 `inner.width` 是同一个数
///
/// 排列趟按父摆进来的矩形折行（[`arrange_flex`] 用 `inner`）。测量趟若拿父
/// 约束的上限，一个 `width: 340px` 的容器就会在测量时按父宽折行、在排列时按
/// 340 折行 —— 两趟行数不同 ⇒ 行高之和算错 ⇒ 容器高度与真实行数不符，
/// 而症状（文字压到下一行上）离原因很远。所以显式主轴长优先，并且**先像
/// [`pick_axis`] 那样夹进父给的区间再扣内边距**：父上限比 `width` 小时摆出来
/// 就是父上限，按一个永远不会出现的宽度折行同样是错的。
fn measure_flex(
    node: &mut Node,
    constraints: Constraints,
    flex: Flex,
    scrollable: bool,
    env: &LayoutEnv,
    ctx: Context<'_>,
) -> Size {
    let padding = node.padding(ctx);
    let deflated = constraints.deflate(padding);
    let (own_w, own_h) = explicit_size(node, ctx);
    let child_constraints = if scrollable {
        // 滚动容器：交叉轴按视口收紧，主轴无界（见 [`scroll_child_constraints`]）。
        let viewport = Size::new(
            own_w.unwrap_or(deflated.max.width),
            own_h.unwrap_or(deflated.max.height),
        );
        let direction = if flex.main_is_horizontal() {
            Direction::Horizontal
        } else {
            Direction::Vertical
        };
        scroll_child_constraints(direction, deflated, viewport)
    } else {
        flex.child_constraints(deflated)
    };
    let mut items: Vec<FlexItem> = Vec::new();
    for i in 0..node.children().len() {
        if node.children()[i].is_collapsed(ctx) {
            continue;
        }
        let manual = node.children()[i].is_manual(ctx);
        let size = measure(&mut node.children_mut()[i], child_constraints, env, ctx);
        if manual {
            continue;
        }
        items.push(flex.read_item(i, size, &node.children()[i], ctx));
    }
    if items.is_empty() {
        return Size::new(0.0, 0.0);
    }
    // 稳定排序：`order` 相等的项保持文档顺序，与 CSS 一致。
    items.sort_by_key(|it| it.order);

    // 折行判定的宽度（见函数文档：必须与排列趟的 `inner` 同源）。
    let axis = if flex.main_is_horizontal() {
        Axis::Horizontal
    } else {
        Axis::Vertical
    };
    let own_main = if flex.main_is_horizontal() {
        own_w
    } else {
        own_h
    };
    let available_main = if scrollable {
        // 滚动容器：主轴**无界** —— 显式的主轴尺寸是**视口**，不是内容高度。
        // 若拿它当折行宽度，纵向 flex 会把内容折成好几列（而不是溢出），
        // 于是永远滚不动。
        f32::INFINITY
    } else {
        match own_main {
            Some(v) => {
                let (min, max) = axis.bounds(constraints);
                let clamped = v.max(min);
                let clamped = if max.is_finite() {
                    clamped.min(max)
                } else {
                    clamped
                };
                (clamped - axis.padding(padding)).max(0.0)
            }
            None => flex.available_main(deflated),
        }
    };
    let lines = flex.lines(&items, available_main);
    let spacing = flex.spacing;
    let line_main = |line: &FlexLine| -> f32 {
        let sum: f32 = line.items.iter().map(|&i| items[i].basis).sum();
        sum + spacing * (line.items.len().saturating_sub(1) as f32)
    };
    let content_main = lines.iter().map(line_main).fold(0.0f32, f32::max);
    let content_cross = if lines.len() > 1 {
        lines.iter().map(|l| l.cross).sum::<f32>() + spacing * (lines.len() as f32 - 1.0)
    } else {
        lines.first().map(|l| l.cross).unwrap_or(0.0)
    };
    if flex.main_is_horizontal() {
        Size::new(content_main, content_cross)
    } else {
        Size::new(content_cross, content_main)
    }
}

/// flex 容器的排列趟：重推行划分与分配 → 逐行落矩形 → 递归。
///
/// # 为什么这条路敢分配 `Vec`
///
/// 只有**显式声明** flex 的容器走得到这里（见 `style::Node::flex`）：项表、
/// 行、分配结果各一个。Stack 路径依旧逐节点零分配 —— 那条的记录见
/// [`arrange`] 开头。
fn arrange_flex(node: &mut Node, inner: Rect, flex: Flex, env: &LayoutEnv, ctx: Context<'_>) {
    let mut items: Vec<FlexItem> = Vec::new();
    for i in 0..node.children().len() {
        if node.children()[i].is_collapsed(ctx) || node.children()[i].is_manual(ctx) {
            continue;
        }
        let size = cached_size_or_zero(&node.children()[i]);
        items.push(flex.read_item(i, size, &node.children()[i], ctx));
    }
    if items.is_empty() {
        arrange_manual_children(node, Point::new(inner.x, inner.y), env, ctx);
        return;
    }
    items.sort_by_key(|it| it.order);

    let available = Size::new(inner.width, inner.height);
    let available_main = flex.main_of(available);
    let available_cross = flex.cross_of(available);
    let lines = flex.lines(&items, available_main);
    let mut boxes = flex.line_boxes(&lines, available_cross);
    if flex.wrap == FlexWrap::WrapReverse {
        for b in boxes.iter_mut() {
            *b = (available_cross - b.0 - b.1, b.1);
        }
    }

    let stack = flex.stack();
    let horizontal = flex.main_is_horizontal();

    for (li, line) in lines.iter().enumerate() {
        let (line_start, line_extent) = boxes[li];
        let sizes = flex.resolve_main(&items, line, available_main);
        let content: f32 =
            sizes.iter().sum::<f32>() + flex.spacing * (line.items.len().saturating_sub(1) as f32);
        let (start, gap) = stack.main_offset_rules(line.items.len(), content, available_main);
        let mut cursor = start;
        for (k, &item_i) in line.items.iter().enumerate() {
            let item = &items[item_i];
            let main_size = sizes[k];
            let main_off = if flex.reverse {
                available_main - cursor - main_size
            } else {
                cursor
            };
            let (cross_pos, cross_extent) = flex.item_cross(item, line_start, line_extent);
            let rect = if horizontal {
                Rect::new(
                    inner.x + main_off,
                    inner.y + cross_pos,
                    main_size,
                    cross_extent,
                )
            } else {
                Rect::new(
                    inner.x + cross_pos,
                    inner.y + main_off,
                    cross_extent,
                    main_size,
                )
            };
            arrange(&mut node.children_mut()[item.idx], rect, env, ctx);
            cursor += main_size + gap;
        }
    }
    arrange_manual_children(node, Point::new(inner.x, inner.y), env, ctx);
}

/// 测量缓存里的尺寸；缺失时返回**零尺寸占位**。
///
/// 见 [`arrange`] 的说明：占位而不是跳过 —— 跳过会让下标错位。
#[inline]
fn cached_size_or_zero(node: &Node) -> Size {
    node.cached_size().unwrap_or(Size::new(0.0, 0.0))
}

/// 显式尺寸（`None` = 由内容决定）。
fn explicit_size(node: &Node, ctx: Context<'_>) -> (Option<f32>, Option<f32>) {
    (
        finite(node.number_or(&crate::props::WIDTH, f32::NAN, ctx)),
        finite(node.number_or(&crate::props::HEIGHT, f32::NAN, ctx)),
    )
}

/// 单轴。约束与内边距都必须按轴取，不能共用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    /// 该轴上的约束区间。
    fn bounds(self, c: Constraints) -> (f32, f32) {
        match self {
            Axis::Horizontal => (c.min.width, c.max.width),
            Axis::Vertical => (c.min.height, c.max.height),
        }
    }

    /// 该轴上的内边距之和。
    fn padding(self, p: Thickness) -> f32 {
        match self {
            Axis::Horizontal => p.left + p.right,
            Axis::Vertical => p.top + p.bottom,
        }
    }
}

/// 单轴上「显式尺寸」与「内容尺寸」如何取值。
///
/// 显式尺寸优先；否则内容尺寸加内边距。三步夹逼各有理由：
///
/// - **不低于下限**：父容器明确要的空间必须给足（`Fill` 语义）。
/// - **不超过上限**：内容溢出时应当被裁剪/滚动，而不是把父容器撑大。没有这一步，
///   一个长列表会把窗口顶出去。
/// - **显式尺寸也要夹逼**：父给的区间是硬边界，`Width` 只是"想要"，不是"命令"。
///
/// 上限为无界时跳过中间那步——`natural.min(INFINITY)` 虽然也等于 `natural`，但
/// 写成显式分支能让"无界"这个意图留在代码里，而不是依赖 `f32::INFINITY` 的语义。
fn pick_axis(
    explicit: Option<f32>,
    content: f32,
    axis: Axis,
    constraints: Constraints,
    padding: Thickness,
) -> f32 {
    let (min, max) = axis.bounds(constraints);
    let natural = match explicit {
        Some(v) => v,
        None => content + axis.padding(padding),
    };
    let clamped = natural.max(min);
    if max.is_finite() {
        clamped.min(max)
    } else {
        clamped
    }
}

/// `NaN` 或负数表示"未设置"，转成 `None`。
fn finite(v: f32) -> Option<f32> {
    (v.is_finite() && v >= 0.0).then_some(v)
}

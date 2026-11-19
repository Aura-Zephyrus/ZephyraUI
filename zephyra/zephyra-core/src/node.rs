//! 保留：一棵 [`Node`] 树持有跨帧存活的状态。
//!
//! # 脏标记的粒度
//!
//! [`Dirty`] 至少要分四级，因为它们触发的工作量差一个数量级：
//!
//! | 等级 | 触发 | 代价 |
//! |---|---|---|
//! | [`Dirty::None`] | — | 零 |
//! | [`Dirty::Paint`] | 颜色变了 | 重生成该子树的 DisplayList 段 |
//! | [`Dirty::Layout`] | 尺寸变了 | 重排该子树 |
//! | [`Dirty::Structure`] | 子节点增删 | 重建该子树 + 重排 |
//!
//! 把「背景色变了」当成 `Layout` 处理，是 UI 卡顿最常见的单一原因。

use crate::core::Rect;
use crate::id::{ElementKey, NodeTag};
use crate::layout::{Constraints, MeasureResult};
use crate::property::{PropertyKey, PropertyStore, PropertyValue};
use std::sync::Arc;

/// 脏标记等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Dirty {
    /// 干净。
    #[default]
    None,
    /// 只需重绘。
    Paint,
    /// 需重排。
    Layout,
    /// 结构变了（子节点增删）。
    Structure,
}

impl Dirty {
    /// 取两者中较重的一个。
    ///
    /// 脏标记沿树向上传播时用这个合并——只要子树里有任何一处结构变化，父节点就
    /// 必须重排。
    pub fn merged(self, other: Dirty) -> Dirty {
        if other > self {
            other
        } else {
            self
        }
    }

    /// 是否需要重排。
    pub fn needs_layout(self) -> bool {
        self >= Dirty::Layout
    }
}

/// 保留节点：树上每个元素对应的持久状态。
pub struct Node {
    tag: NodeTag,
    key: Option<ElementKey>,
    /// 样式分类名（与 [`crate::element::Element::style`] 对应，每帧由 diff 对账）。
    pub(crate) styles: Vec<&'static str>,
    props: PropertyStore,
    children: Vec<Node>,
    /// P2c：构建/对账时对应的 Element 身份。`Arc::ptr_eq` 相同表示子树输入未变，可跳过 diff 下探。
    source: Option<Arc<crate::element::Element>>,
    /// 本帧**绘制变换变过**（translate / scale）。
    ///
    /// # 为什么单独一个标记，而不是只看「当前变换是不是恒等」
    ///
    /// 脏区算的是节点的**新**位置，而「移走」留下的是**旧**位置的像素 ——
    /// 旧矩形没有快照，所以只能退让成整帧。判据若写成「当前变换非恒等」，
    /// 就会漏掉**回到恒等**的那一帧：把手从 20px 滑回 0 时，落点那帧的变换
    /// 恰好是恒等，判据不成立 ⇒ 脏区收窄成把手的新矩形 ⇒ 旧位置的把手留在
    /// 屏幕上（两个把手并排的残影）。
    ///
    /// 所以这里记的是「**变了**」这件事，而不是「变成了什么」——由写属性的那
    /// 一侧（diff 写本地值 / `set_styled_prop` 写样式值）判定。
    moved: bool,
    /// 测量结果缓存（`Dirty >= Layout` 时失效）。
    ///
    /// 缓存键包含 `(Constraints, viewport)`（D15/R2）：`root` 手动项的视口约束
    /// 依赖 viewport，避免 resize 时约束未变但视口变了导致错用旧测量。
    pub(crate) measure_cache: Option<(Constraints, crate::core::Size, MeasureResult)>,
    /// 字形四边形缓存（局部坐标，原点在元素上）。
    ///
    /// # 为什么是 `RefCell`
    ///
    /// 写入方是**绘制趟**（[`crate::paint::render`]），而它只拿到 `&Node`
    /// —— 因为保留树的遍历是共享借用：绘制不能顺手改布局字段。用 `RefCell`
    /// 而不是把 `render` 改成 `&mut Node`：后者会让整条渲染链从借用检查里
    /// 受益变成受阻（`DisplayList` 的写入、`Context` 里那个 `RefCell<TextEngine>`
    /// 都要跟着改借用关系）。
    ///
    /// 借用守卫的**作用域**必须收紧到「读缓存 / 算一次 / 写缓存」三步之内，
    /// 绝不能跨 `render_rec` 的递归调用 —— 那会在第二次进入同一节点时 panic。
    ///
    /// # 失效时机
    ///
    /// **不由脏标记、不由 `MeasureResult` 管**。字形的局部坐标只由两件事决定：
    ///
    /// 1. 排版的**输入**（文本 / 样式 / `max_width`）⇒ 存在 [`GlyphQuadCache`]
    ///    里当指纹逐项比。不用「测量趟重算过没有」当判据：resize 时约束一变，
    ///    `measure` 就会重跑并产出一个**内容相同**的 `MeasureResult`（只是
    ///    `size` 因 `pick_axis` 被夹了一下）—— 用那种判据命中率恒为 0，
    ///    136 KB/帧 一分没省。
    /// 2. 图集修订号（LRU 淘汰会把 uv 整体重排）⇒ 编号对上才用。
    ///
    /// 两条都在使用点就地判定，所以**不需要任何人在改样式时记得手动清它** ——
    /// 那正是「靠人记得」最容易漏的一类缓存。
    pub(crate) glyph_quads: std::cell::RefCell<Option<Box<GlyphQuadCache>>>,
    /// 本节点已摆放的矩形（相对父节点）。`pub(crate)`：只由排列趟写入。
    pub(crate) arranged_rect: Rect,
    /// 当前脏标记。`pub(crate)`：只由 [`crate::diff`] 直接合并（见
    /// [`Node::merge_dirty`]）。
    pub(crate) dirty: Dirty,
}

/// 挂在节点上的字形四边形缓存条目。
///
/// # 为什么字段类型是 `Box<GlyphQuadCache>`
///
/// 内联的话这个结构有一百多字节（`Box<str>` + `TextStyle` + `Option<f32>` +
/// `u64` + `Arc<[GlyphQuad]>` + `TextureRef`），而它会把 `Node` **直接**撑大
/// 同等大小。实测（`demo --bench-resize-cpu --alloc-probe`，与 `1f76d8e`
/// 同一次运行的对照）：
///
/// - 分配**次数** 612 → 576（−36/帧，字形 `Vec` 确实没了）；
/// - 但分配**字节** **+8%**：`Node` 变大 ⇒ 建树时每一次 `Vec<Node>` 重分配都变大。
///
/// 装箱只让 `Node` 多一个指针（8 字节），代价是每次 miss 多一次堆分配 ——
/// 而 miss 极少（实测 100 帧 resize 共 19 次）。
///
/// 见 [`Node::glyph_quads`] 的失效规则（两条都在使用点就地判定）。
pub(crate) struct GlyphQuadCache {
    /// 排版输入指纹：文本。存一份自有副本，命中时与本帧的 `&str` 比。
    /// 排版输入指纹：文本。存一份自有副本，命中时与本帧的 `&str` 比。
    ///
    /// 为什么要存副本而不是存指针：`Node::text()` 借自 `PropertyStore`，而属性
    /// 每帧由 diff 重新对账 —— 指针可能在下一帧指向别处。
    pub(crate) text: Box<str>,
    /// 排版输入指纹：样式。`TextStyle` 派生了 `PartialEq`。
    ///
    /// 里面那个 `Option<String>`（字体族）在命中路径上要克隆一次才能比 ——
    /// 代价是每帧每文本节点一次小字符串克隆，换掉的是每帧每文本节点一次
    /// **重新排版**。那个交换是划算的，而且量级差着两个数量级。
    pub(crate) style: crate::text::TextStyle,
    /// 排版输入指纹：`max_width`。
    pub(crate) max_width: Option<f32>,
    /// 算出这些四边形时的图集修订号（`TextStack::atlas_revision`）。
    ///
    /// LRU 淘汰会**整体重排**图集并把修订号加一 ⇒ 缓存里的 uv 全作废。
    /// 这条判据不能用「已缓存字形数」代替 —— 淘汰一个再放进一个，数量不变而
    /// uv 已经换了位置。宿主那边也是靠修订号来发现图集变化的
    /// （`host.rs` 的 `atlas_revision_uploaded`）。
    pub(crate) revision: u64,
    /// 上面那个修订号对应的图集句柄。
    ///
    /// 一并存下来，是因为命中路径**不能**再调一次 `glyph_quads` 去拿它 ——
    /// 那个调用会把四边形也重算一遍，等于缓存白做。
    /// 同修订号 ⇒ 同句柄，宿主那边也是这么判的（`atlas_revision_uploaded`）。
    pub(crate) atlas: crate::paint::TextureRef,
    /// 局部坐标（原点 = 元素原点）的字形四边形。
    pub(crate) glyphs: std::sync::Arc<[crate::paint::GlyphQuad]>,
}

impl Node {
    /// 由元素构建节点（首次进入树时）。
    pub fn from_element(element: &crate::element::Element) -> Node {
        Node {
            tag: element.tag(),
            key: element.key().cloned(),
            styles: element.style_names().to_vec(),
            props: element.props().clone(),
            children: element.child_elements().map(Node::from_element).collect(),
            source: None,
            moved: false,
            measure_cache: None,
            glyph_quads: std::cell::RefCell::new(None),
            arranged_rect: Rect::default(),
            dirty: Dirty::Structure,
        }
    }

    /// 由共享元素句柄构建节点（保留 Arc 身份，供 P2c diff 子树跳过）。
    pub fn from_element_arc(element: Arc<crate::element::Element>) -> Node {
        Node {
            tag: element.tag(),
            key: element.key().cloned(),
            styles: element.style_names().to_vec(),
            props: element.props().clone(),
            children: element
                .children()
                .iter()
                .cloned()
                .map(Node::from_element_arc)
                .collect(),
            source: Some(element),
            moved: false,
            measure_cache: None,
            glyph_quads: std::cell::RefCell::new(None),
            arranged_rect: Rect::default(),
            dirty: Dirty::Structure,
        }
    }

    pub(crate) fn source_arc(&self) -> Option<&Arc<crate::element::Element>> {
        self.source.as_ref()
    }

    pub(crate) fn set_source_arc(&mut self, source: Option<Arc<crate::element::Element>>) {
        self.source = source;
    }

    /// 类型标签。
    pub fn tag(&self) -> NodeTag {
        self.tag
    }

    /// 稳定 key。
    pub fn key(&self) -> Option<&ElementKey> {
        self.key.as_ref()
    }

    /// 样式分类名。
    pub fn style_names(&self) -> &[&'static str] {
        &self.styles
    }

    /// 属性表。
    pub fn props(&self) -> &PropertyStore {
        &self.props
    }

    /// 可变属性表。
    ///
    /// 单独暴露而不是只留 [`Node::set_prop`]，是因为主题重解析必须**原地**改写
    /// 已存在的条目：那一轮里需要按"当前是否还是资源键"来筛选，逐个走
    /// `set_prop` 会多算一次脏标记，而资源重解析本身并不该把节点标脏——它只是把
    /// 同一个键的字面值换掉，布局不受影响。
    pub fn props_mut(&mut self) -> &mut PropertyStore {
        &mut self.props
    }

    /// 子节点。
    pub fn children(&self) -> &[Node] {
        &self.children
    }

    /// 已摆放的矩形（**绝对坐标**）。
    ///
    /// 只读访问器：写入方是排列趟（`pub(crate)` 语义不变）；命中测试与调试桥直接
    /// 使用这个矩形，因此布局树坐标与指针注入处于同一坐标系。
    pub fn arranged_rect(&self) -> crate::core::Rect {
        self.arranged_rect
    }

    /// 可变子节点。
    ///
    /// 给宿主在布局后调整已存在的子树（改样式、折叠某项）。注意这绕过 diff——
    /// 直接改动不会留下脏标记，所以调用方需要自己 [`Node::mark_dirty`]。
    pub fn children_mut(&mut self) -> &mut Vec<Node> {
        &mut self.children
    }

    /// 脏标记。
    pub fn dirty(&self) -> Dirty {
        self.dirty
    }

    /// 本子树里有没有节点需要重排（含自己）。
    ///
    /// # 为什么测量缓存需要它
    ///
    /// 测量缓存只按 `Constraints` 做键，而**祖先的缓存不会自动知道后代被改了**：
    /// 子节点换尺寸时只有它自己的缓存被清，父级仍会命中旧值，`arrange` 于是拿
    /// 旧尺寸（或缺失时的 0）摆它。症状是样式驱动的尺寸变化把节点压成 0×0，
    /// 只有 resize 这类整树失效才会自愈。
    pub fn subtree_needs_layout(&self) -> bool {
        self.dirty.needs_layout()
            || self
                .children
                .iter()
                .any(|child| child.subtree_needs_layout())
    }

    /// 把该子树标脏（向下传播）。
    pub fn mark_dirty(&mut self, level: Dirty) {
        self.dirty = self.dirty.merged(level);
        if level.needs_layout() {
            // 布局变化会让测量缓存失效。
            self.measure_cache = None;
            //
            // ⚠ 这里**故意不清**字形四边形缓存。字形的局部坐标只由
            // （文本 / 样式 / `max_width`）和图集修订号决定 —— 元素的**摆放矩形**
            // 变了它一个字节都不变，而那个矩形正是 resize 那一趟每帧都在改的东西。
            //
            // 清它的判据是 [`GlyphQuadCache`] 里那份排版输入指纹。
            //
            // 早先这里也清过，于是 resize 时每帧全树清一遍 —— 缓存命中率恒为 0，
            // 136 KB/帧 一分没省。这条注释是那个错的墓碑。
        }
        if level >= Dirty::Structure {
            for child in &mut self.children {
                child.mark_dirty(level);
            }
        }
    }

    /// 本节点已摆放的矩形（绝对坐标）。
    pub fn rect(&self) -> Rect {
        self.arranged_rect
    }

    /// 记下「本帧的绘制变换变过」。
    ///
    /// 写这四个键（translate / scale）的一侧调用它 —— 见 [`crate::props::is_transform_key`]。
    /// 判据是「**变了**」，不是「变成了什么」：回到恒等的那一帧同样会移走。
    pub fn mark_moved(&mut self) {
        self.moved = true;
    }

    /// 本帧绘制变换是否变过（见 [`Self::mark_moved`]）。
    pub fn moved(&self) -> bool {
        self.moved
    }

    /// 按事件身份深搜本子树，返回第一个 key 匹配的节点。
    ///
    /// 用**深度优先 + 文档序**：与 [`crate::input::focus::collect_focusable`] 的遍历
    /// 顺序一致，于是「按 key 查到的元素」与「Tab 到的元素」是同一个。
    pub fn find(&self, key: &ElementKey) -> Option<&Node> {
        if self.key() == Some(key) {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(key))
    }

    /// 按事件身份查该元素的**窗口矩形**；不存在时返回 `None`。
    pub fn find_rect(&self, key: &ElementKey) -> Option<Rect> {
        self.find(key).map(|n| n.rect())
    }

    /// 收集本子树中所有带 key 节点的矩形（P1.2 `InputState::rect_of` 的数据源）。
    ///
    /// 深度优先、文档序；宿主在 `on_input` 之前对已布局的树调用一次，把快照塞进
    /// `InputState`，应用即可按 key 查「控件在哪」，与 `pointer_events` 同一坐标系。
    pub fn collect_keyed_rects(&self, out: &mut Vec<(ElementKey, Rect)>) {
        if let Some(k) = self.key() {
            out.push((k.clone(), self.rect()));
        }
        for child in &self.children {
            child.collect_keyed_rects(out);
        }
    }

    /// 本元素矩形在窗口里的**中心点**（布局几何中心）。
    pub fn center(&self) -> crate::core::Point {
        let r = self.rect();
        crate::core::Point::new(r.x + r.width * 0.5, r.y + r.height * 0.5)
    }

    /// 设置属性并按影响范围标脏。
    ///
    /// 属性自己声明是否影响布局（见 [`crate::property::PropertyChange::affects_layout`]），
    /// 调用方不必知道每个属性的语义。
    pub fn set_prop(&mut self, key: &'static PropertyKey, value: PropertyValue) {
        if let Some(change) = self.props.set_local(key, value) {
            self.dirty = self.dirty.merged(if change.affects_layout() {
                Dirty::Layout
            } else {
                Dirty::Paint
            });
            if self.dirty.needs_layout() {
                self.measure_cache = None;
            }
            // 绘制变换变了 ⇒ 节点「移走」：旧位置的像素没人清（脏区只算新位置）。
            if crate::props::is_transform_key(key) {
                self.mark_moved();
            }
        }
    }

    /// 设置**样式表**给的属性值并按影响范围标脏。
    ///
    /// 写在 `Styled` 层（低于 `Local`），所以应用代码的 `.with(...)` 仍然覆盖它。
    ///
    /// ⚠️ **为什么必须走这个方法而不能 `props_mut()` 直接改**：那样不产生脏标记，
    /// 于是「改了样式但画面不动」——而样式表正是每帧都在改的那一层。这个洞 B2 才补：
    /// 之前 `set_styled` 虽返回 `PropertyChange`，但 `Node` 上没有对应入口，
    /// 唯一的路是 `props_mut()`，而它**绕过了标脏**。
    ///
    /// 与 [`Self::set_prop`] 的差别只有一层：写 `Styled` 而不是 `Local`。
    /// 标脏逻辑**必须完全一致**——两边走偏的症状是
    /// 「同一种属性，源码里改会重排、样式表里改不会」。
    ///
    /// ⚠️ **「值没变就不标脏」这条在这个路径上尤其重要**：`apply` 每帧都会把同一条
    /// 规则再写一遍。若写同值也算变化，静止界面会永远在重绘——这是
    /// 「样式表导致 UI 永远不静止」这类 bug 的直接成因。
    pub fn set_styled_prop(&mut self, key: &'static PropertyKey, value: PropertyValue) {
        if self.props.set_styled_if_changed(key, value) {
            self.dirty = self.dirty.merged(if key.affects_layout() {
                Dirty::Layout
            } else {
                Dirty::Paint
            });
            if self.dirty.needs_layout() {
                self.measure_cache = None;
            }
            // 样式表也能改绘制变换（分类名一变，位移就变）—— 与本地值同一条路。
            if crate::props::is_transform_key(key) {
                self.mark_moved();
            }
        }
    }

    /// 写入样式值（**借用**入参）—— 见 [`Self::set_styled_prop`]。
    ///
    /// # 为什么要分两个
    ///
    /// 按值版在**调用点**就得克隆。而 [`crate::property::PropertyValue::Filter`]
    /// 持一个 `Vec<Filter>` —— 克隆它就是一次堆分配。
    ///
    /// 样式表每帧都把同一条规则再写一遍（`apply` 每次都跑），所以稳态下
    /// 每个带滤镜的节点每帧都要克隆一次滤镜链 —— 哪怕值一个字节都没变。
    ///
    /// 借用版只在**真的要写**的时候才克隆。
    pub fn set_styled_prop_ref(&mut self, key: &'static PropertyKey, value: &PropertyValue) {
        if self.props.set_styled_if_changed_ref(key, value) {
            self.dirty = self.dirty.merged(if key.affects_layout() {
                Dirty::Layout
            } else {
                Dirty::Paint
            });
            if self.dirty.needs_layout() {
                self.measure_cache = None;
            }
            if crate::props::is_transform_key(key) {
                self.mark_moved();
            }
        }
    }

    /// 拆掉**本轮没人再声明**的样式值，按影响范围标脏。
    ///
    /// [`Self::set_styled_prop`] 是「写」，本方法是「拆」—— 两者缺一不可，
    /// 少了拆的那一半，状态类消失后旧值会永久赖在节点上（症状：点一下取消勾选，
    /// 底色还是选中色；见 [`crate::property::PropertyStore::retain_styled`]）。
    ///
    /// 返回被拆下的键，供测试与诊断用。**只有真的拆下了东西才标脏** ——
    /// `apply` 每帧都跑一遍，写同值不算变化（见 [`Self::set_styled_prop`]），
    /// 拆不存在的东西也必须同样安静，否则静止界面会永远在重绘。
    pub fn retain_styled_props(
        &mut self,
        keep: impl Fn(&'static PropertyKey) -> bool,
    ) -> Vec<&'static PropertyKey> {
        let removed = self.props.retain_styled(keep);
        for key in &removed {
            self.dirty = self.dirty.merged(if key.affects_layout() {
                Dirty::Layout
            } else {
                Dirty::Paint
            });
            // 拆掉绘制变换 = 变换**变回默认值** ⇒ 同样是「移走」。
            if crate::props::is_transform_key(key) {
                self.mark_moved();
            }
        }
        if !removed.is_empty() && self.dirty.needs_layout() {
            self.measure_cache = None;
        }
        removed
    }

    /// 拆掉**本帧声明里没有**的本地属性，返回被拆下的键。
    ///
    /// 声明树是**真值**：`.with(...)` 这一帧没写的东西，就不该留在节点上。
    /// 与 [`Self::retain_styled_props`] 同源（只写不清 → 残值静默留存），区别是
    /// 这一层来自源码而不是样式表。
    ///
    /// 症状举例：控件写 `if closable { } else { .with(COLLAPSED, true) }`，
    /// 宿主把 `closable` 翻回 true 之后 `COLLAPSED` 仍赖着 —— 元素永远折叠着。
    pub fn retain_local_props(
        &mut self,
        keep: impl Fn(&'static PropertyKey) -> bool,
    ) -> Vec<&'static PropertyKey> {
        let removed = self.props.retain_local(keep);
        for key in &removed {
            self.dirty = self.dirty.merged(if key.affects_layout() {
                Dirty::Layout
            } else {
                Dirty::Paint
            });
        }
        if !removed.is_empty() && self.dirty.needs_layout() {
            self.measure_cache = None;
        }
        removed
    }

    /// 拆下唯一子节点并返回它。
    ///
    /// 给"只想检查某个已布局节点本身"的调用方用：布局必须从根开始（根决定子项矩形），
    /// 但断言关心的往往只是其中一个子项。
    ///
    /// # 为什么不叫 `only_child`
    ///
    /// 因为它在**子项数量不为 1 时直接 panic**。这不是"取第一个"那种宽容语义——
    /// 那样会让一个写错的测试断言悄悄读到错误的节点，而这里的错误一定来自测试自身。
    pub fn take_only_child(&mut self) -> Node {
        assert_eq!(
            self.children.len(),
            1,
            "take_only_child 要求恰好一个子节点，实得 {}",
            self.children.len()
        );
        self.children.remove(0)
    }

    /// 合并一个脏标记，**不**向子节点传播。返回是否真的变脏了。
    ///
    /// 专供 [`crate::diff`] 用。它已经**递归地对每个子节点各调一次**
    /// [`Node::mark_dirty`]，所以这里必须只改自己 —— 否则会把这次变化错误地盖到
    /// 那些其实没变的兄弟节点上（症状是"改一行文字，整棵列表都重排"）。
    pub(crate) fn merge_dirty(&mut self, level: Dirty) -> bool {
        let before = self.dirty;
        self.dirty = self.dirty.merged(level);
        self.dirty != before
    }

    /// 测量缓存里的尺寸（`None` = 没测过或已失效）。
    ///
    /// 排列趟用它复用刚测出的结果，而不是重跑一遍测量。
    pub(crate) fn cached_size(&self) -> Option<crate::core::Size> {
        self.measure_cache.as_ref().map(|(_, _, r)| r.size)
    }

    /// **子内容**的尺寸（DIP）。
    ///
    /// 布局跑完后才有值（没量过时是零）。滚动容器用它算「内容比视口大多少」
    /// —— 那是滚动范围与滚动条拇指长度的唯一来源。
    pub fn content_size(&self) -> crate::core::Size {
        self.measure_cache
            .as_ref()
            .map(|(_, _, r)| r.content)
            .unwrap_or_default()
    }

    /// 命中缓存则返回尺寸（键：约束 + 视口，D15/R2）。
    pub(crate) fn cached_measure(
        &self,
        constraints: Constraints,
        viewport: crate::core::Size,
    ) -> Option<crate::core::Size> {
        self.measure_cache
            .as_ref()
            .and_then(|(c, vp, r)| (*c == constraints && *vp == viewport).then_some(r.size))
    }

    /// 写入测量缓存（键包含 viewport，D15/R2）。
    pub(crate) fn cache_measure(
        &mut self,
        constraints: Constraints,
        viewport: crate::core::Size,
        result: MeasureResult,
    ) {
        self.measure_cache = Some((constraints, viewport, result));
    }

    /// 写入已摆放的矩形。只应由 [`crate::layout`] 的排列趟调用。
    pub(crate) fn set_rect(&mut self, rect: Rect) {
        self.arranged_rect = rect;
    }
}

/// 清除脏标记（重排 + 渲染完成后由宿主调用）。
///
/// **必须有这一步**，否则脏标记只增不减，`needs_layout` 会永久停在 true，
/// 「本帧要不要重排」这个判断就失去意义。真实宿主在 layout+render 走完后调用它。
///
/// # ⚠ 这里**故意不清** `measure_cache`
///
/// 本函数早先会递归清掉测量缓存，那是**错的**，而且错得很贵：
///
/// - 官方把缓存放在 `CLayoutStorage::m_previousAvailableSize`
///   （`uielement.cpp:4223-4226`），它是**跨帧存活**的 —— 静止时每一帧
///   `Measure()` 都会拿它做一次精确比较然后整个子树跳过测量；
/// - 我们每帧调一次 `clear_dirty`，于是缓存每帧被清 ⇒ **每一帧都重测整棵树**、
///   重新整形所有文字。那是布局里最贵的一步，而它本来一次都不该发生。
///
/// 缓存的失效时机只有一处：**某个布局相关属性被改**。那已经由
/// [`Node::mark_dirty`] / [`Node::set_prop`] / [`Node::set_styled_prop`] /
/// `retain_*` 各自负责（它们都在 `needs_layout()` 时清缓存）。
/// 换句话说，"脏标记清空"与"测量结果过期"是两件事 ——
/// 前者每帧都发生，后者只在属性变化时发生。
///
/// 门禁：`zephyra-core/tests/layout_cache_tests.rs::steady_state_does_not_remeasure`
/// （改之前它是红的，每帧 6 次整形）。
pub fn clear_dirty(node: &mut Node) {
    node.dirty = Dirty::None;
    node.moved = false;
    for child in node.children_mut() {
        clear_dirty(child);
    }
}

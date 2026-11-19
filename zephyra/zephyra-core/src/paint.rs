//! 绘制契约：[`DisplayList`] ——核心与 GPU 后端之间**唯一**的接口。
//!
//! # 为什么这是整个框架最关键的契约
//!
//! 布局、控件、主题最终全部降到一个**排序后的绘制命令列表**。后端只消费它。
//! 这意味着：
//!
//! - 换宿主（宿主 → winit+D3D11）时，**核心一行都不用改**——因为核心从不认识
//!   宿主；
//! - 宿主可以自由选择批次策略：全程一个 draw call，或按图层切若干 pass；
//! - 「这一帧重算了什么」可以直接从 DisplayList 上读出来，回答 GPU 占用率问题；
//! - 单元测试不需要 GPU：断言 DisplayList 的内容即可验证整个降级过程。
//!
//! # 排序
//!
//! 命令按 `(layer, z_index, order)` 稳定排序。`layer` 是粗粒度的合成层
//! （背景 / 内容 / 装饰 / 浮层），后端据此切 render pass；`z_index` 是层内顺序；
//! `order` 是同 z 内的稳定次序（**稳定**排序保证同样输入产出同样列表，
//! 否则每次 diff 出来的顺序都不同，纹理缓存命中率会随机抖动）。
//!
//! # 为什么不是「绘制回调」
//!
//! 另一种常见设计是给后端一个闭包列表（`Vec<Box<dyn Fn(&mut Renderer)>>`）。
//! 那样无法跨线程、无法缓存、无法测试——DisplayList 用数据换来了这三样。
//!
//! 本模块同时提供 [`render`]：把一棵**已布局**的节点树降级成本模块的数据结构。
//! 它是核心对外的唯一产出 —— 宿主拿到 [`DisplayList`] 之后就不再关心树的存在。
//! 几何必须由 [`crate::layout`] 先算好，本函数只读已摆好的矩形。

use crate::context::Context;
use crate::core::{Color, CornerRadius, Point, Rect};
use crate::layout::Position;
use crate::node::Node;
use crate::visual;

pub use zephyra_vector::paint::*;

/// 布局 + 降级：把一棵已布局的节点树变成 [`DisplayList`]。
///
/// 调用前必须先 [`crate::layout::layout`] —— 本函数只读已摆好的矩形，不算几何。
/// 几何只有一处算法（[`crate::layout`]），命中测试与绘制共用同一批矩形，所以不会
/// 出现"看得见但点不到"。
pub fn render(node: &Node, ctx: Context<'_>, out: &mut DisplayList) {
    out.clear();
    let mut order = 0u32;
    render_rec(node, ctx, &mut order, out, PaintState::ROOT);
}

/// 绘制趟沿树传递的状态。
///
/// # 为什么不靠属性继承
///
/// [`crate::props::OPACITY`] 声明成 `inheritable`，而
/// [`crate::property::resolve`] 的 `inherited` 参数在**生产代码里没有任何地方
/// 传过非 `None` 的值**（`style.rs::value` 一律传 `None`）—— 也就是说
/// `node.opacity(ctx)` 取的是**本节点**的值，不是祖先链的乘积。
///
/// 去接属性继承会牵动 `text_color` 等一整批 `inheritable` 属性，那是独立一轮。
/// 绘制趟自己累加是**自洽且正确**的做法：半透明天然沿父子链相乘、位移天然相加，
/// 而这里只需要这两种。
///
/// ⚠️ 这也意味着 `opacity()` 作为公开 API 的语义是"**本节点**的不透明度"，
/// 与 [`crate::property::PropertyKey::inheritable`] 的名字不一致。记账见
/// `docs/dev-log/task-anim-wire.md` §0。
#[derive(Debug, Clone, Copy, PartialEq)]
struct PaintState {
    /// 祖先链的不透明度之积。
    opacity: f32,
    /// 祖先链的位移之和（绘制时偏移，**不影响布局与命中测试**）。
    translate: Point,
    /// 有效合成层（D18：祖先上下文传播，显式 `layer` 属性优先）。
    layer: Layer,
}

impl PaintState {
    /// 根状态：什么都不乘、什么都不移。
    const ROOT: PaintState = PaintState {
        opacity: 1.0,
        translate: Point::ZERO,
        layer: Layer::Content,
    };

    /// 推进到本节点：把**本节点**的不透明度与位移并入。
    ///
    /// D11: `position: root` 清除祖先累计 translate，自身 translate 仍生效；
    /// `opacity` 仍沿祖先链相乘。D18: 显式 `layer` 属性优先，否则继承祖先有效层。
    ///
    /// ⚠ **本节点自己的盒子不含滚动偏移**：滚动容器自身的背景 / 边框留在视口
    /// 原位，被滚动的是**子内容**。所以滚动偏移在 [`Self::descend_children`]
    /// 里施加，不在这里。
    fn descend(&self, node: &Node, ctx: Context<'_>) -> Self {
        let own = node.opacity(ctx);
        let own = if own.is_finite() { own } else { 1.0 };
        let opacity = self.opacity * own;
        // D11: root-positioned node does not inherit ancestor accumulated translate.
        let base_translate = if node.position(ctx) == Position::Root {
            Point::ZERO
        } else {
            self.translate
        };
        let (tx, ty) = (
            base_translate.x + node.translate_x(ctx),
            base_translate.y + node.translate_y(ctx),
        );
        let layer = visual::effective_layer(node, ctx).unwrap_or(self.layer);
        PaintState {
            opacity: if opacity.is_finite() {
                opacity.clamp(0.0, 1.0)
            } else {
                1.0
            },
            translate: if tx.is_finite() && ty.is_finite() {
                Point::new(tx, ty)
            } else {
                base_translate
            },
            layer,
        }
    }

    /// 推进到本节点的**子节点**：滚动容器的子内容按 `-scroll_offset` 平移。
    ///
    /// # 为什么滚动偏移只作用在子节点上
    ///
    /// 滚动容器的盒子就是**视口** —— 它的背景、边框、滚动条轨道都必须留在
    /// 视口原位。若把偏移也加到容器自身上，滚动一下整个容器（含背景）就跟着
    /// 移走，用户看到的是「容器在动」而不是「内容在动」。
    ///
    /// 命中测试按同一条规则反向平移（见 [`crate::hit`]），所以两边看到的
    /// 「指针落在内容哪一项上」永远一致。
    fn descend_children(&self, node: &Node, ctx: Context<'_>) -> Self {
        let scroll = node.scroll_offset(ctx);
        if scroll.x == 0.0 && scroll.y == 0.0 {
            return *self;
        }
        PaintState {
            translate: Point::new(self.translate.x - scroll.x, self.translate.y - scroll.y),
            ..*self
        }
    }
}

/// 一个节点在绘制趟里的**本节点**变换：祖先位移 + 本节点绘制缩放。
///
/// # 为什么缩放不累乘
///
/// 位移沿父链**相加**（嵌套容器带着子树一起动），而缩放是**本节点自己的
/// 外观**（见 [`crate::props::SCALE_X`]）—— 子节点要缩放就自己再写一遍。
/// 把它做成累乘会需要"子树变换栈"，而文字字形按元素原点缓存、目前没有
/// 缩放通道，那半套语义比明说"只缩放本节点"更难查。
#[derive(Debug, Clone, Copy, PartialEq)]
struct NodeDraw {
    /// 已含祖先位移的绘制偏移。
    offset: Point,
    /// 本节点自身的绘制缩放。
    scale: Point,
    /// **未平移**矩形（布局盒）的中心。
    center: Point,
}

impl NodeDraw {
    fn new(node: &Node, offset: Point, ctx: Context<'_>) -> Self {
        let r = node.rect();
        Self {
            offset,
            scale: Point::new(node.scale_x(ctx), node.scale_y(ctx)),
            center: Point::new(r.x + r.width * 0.5, r.y + r.height * 0.5),
        }
    }

    /// 是否恒等（不移动也不缩放）。
    fn is_identity(&self) -> bool {
        self.offset.x == 0.0 && self.offset.y == 0.0 && self.scale.x == 1.0 && self.scale.y == 1.0
    }

    /// 把一个**布局坐标**映射到画面上：先绕本节点中心缩放，再加偏移。
    fn point(&self, p: Point) -> Point {
        if self.is_identity() {
            return p;
        }
        Point::new(
            (p.x - self.center.x) * self.scale.x + self.center.x + self.offset.x,
            (p.y - self.center.y) * self.scale.y + self.center.y + self.offset.y,
        )
    }

    /// 映射一个矩形。宽高按两轴分别缩放。
    fn rect(&self, r: Rect) -> Rect {
        if self.is_identity() {
            return r;
        }
        let p = self.point(Point::new(r.x, r.y));
        Rect::new(p.x, p.y, r.width * self.scale.x, r.height * self.scale.y)
    }

    /// 缩放一个标量（圆角半径 / 描边宽度 / 图像边长）。
    ///
    /// 非等比缩放取**两轴较小值**：圆角与描边只有一个标量，用较大值会让
    /// 细长形状的边看起来比形状本身还粗。
    fn scalar(&self, v: f32) -> f32 {
        if self.is_identity() {
            v
        } else {
            v * self.scale.x.min(self.scale.y)
        }
    }
}

fn render_rec(
    node: &Node,
    ctx: Context<'_>,
    order: &mut u32,
    out: &mut DisplayList,
    state: PaintState,
) {
    if node.is_collapsed(ctx) {
        return;
    }
    // 先进"本节点"再画：累积的不透明度与位移既属于本节点，也往下传。
    let state = state.descend(node, ctx);
    let offset = state.translate;
    // 本节点自己的绘制变换：祖先位移 + 本节点的绘制缩放。缩放**不沿父链
    // 相乘**（语义见 `props::SCALE_X`），所以这里每次从本节点取值。
    let draw = NodeDraw::new(node, offset, ctx);
    let rect = draw.rect(node.rect());
    let opacity = state.opacity;
    let layer = state.layer;
    // 效果与混合模式取自节点自身，不沿父链继承（见 props.rs 的理由）。
    let filter = node.filter(ctx);
    let blend = node.blend_mode(ctx);

    // 自身外观：背景 + 描边合成到一条 Quad（描边复用同一条命令的 stroke 字段，
    // 于是圆角矩形永远是一个 draw call —— 这正是本模块选 Quad 图元的原因）。
    //
    // ⚠ `fill` / `stroke_color` 都过 `with_alpha(opacity)`：**在加上这个之前，
    // `opacity` 只作用于 `image_tint`**，所以"给容器设半透明"在画面上只对图像
    // 生效，而背景、描边、文字全都不透明 —— 那与"没有这个属性"看起来一模一样，
    // 只会让人以为它坏了。
    let fill = node.background(ctx).with_alpha(opacity);
    let stroke_color = node.border_color(ctx).with_alpha(opacity);
    let stroke_width = draw.scalar(node.border_width(ctx));
    let radius = draw.scalar(node.corner_radius(ctx));
    let has_stroke = stroke_color.a > 0.0 && stroke_width > 0.0;
    let decorations = node.decorations(ctx);
    // 背景全透明（或根本没有背景）**不等于**这条命令不存在：滤镜与玻璃装饰
    // 描述的是"元素背后那一层"，与元素自己的 fill 无关。漏掉这一条的症状是
    // 「按住时开关把手整块消失」—— 白罩 alpha 随按压归零后，同时承载
    // `BackdropBlur` / `Lens` / `InnerShadow` 的那条 Quad 也不再发出，于是越按
    // 越空，而不是变成玻璃。
    //
    // 但**不能无条件补**：图标 / 图像 / 文字 / 自绘源已经在它们自己的命令上带
    // 了同一个 `filter`，再补一条空的 Quad 会让同一层开两次离屏（`effect_contract`
    // 里 `blurred_vector_icon_yields_offscreen_requirement` 直接按数量断言）。
    // 装饰没有别的载体（高光/内阴影必须有一条 Quad 才有离屏块的尾部时机），
    // 所以它单独算一条要补的理由。
    let has_own_content = node.image_texture(ctx).is_some()
        || node.icon_name(ctx).is_some_and(|n| !n.is_empty())
        || node.path_data(ctx).is_some_and(|p| !p.is_empty())
        || node.text().is_some()
        || !node.render_source(ctx).is_none();
    let filter_needs_quad = !filter.is_identity() && !has_own_content;
    let decorations_need_quad = !decorations.is_identity();
    if fill.a > 0.0 || has_stroke || filter_needs_quad || decorations_need_quad {
        out.push(
            SortKey {
                layer,
                z: 0,
                order: next_order(order),
            },
            Command::Quad {
                rect,
                radius: CornerRadius { radius },
                shape: node.corner_shape(ctx),
                fill,
                stroke: has_stroke.then_some((stroke_color, stroke_width)),
                filter: filter.clone(),
                blend,
                decorations,
            },
        );
    }

    // 图像内容
    if let Some(texture) = node.image_texture(ctx) {
        // `image_size` 是一个标量（正方边长），非等比缩放时取两轴较小的那个 ——
        // 与圆角 / 描边同一条降级规则：宁可小一点，不要拉伸到变形。
        let size = draw.scalar(node.image_size(ctx));
        let dst = if size > 0.0 {
            // 指定边长：在节点矩形内居中。
            let dx = rect.x + (rect.width - size) * 0.5;
            let dy = rect.y + (rect.height - size) * 0.5;
            Rect::new(dx, dy, size, size)
        } else {
            rect
        };
        out.push(
            SortKey {
                layer,
                z: 0,
                order: next_order(order),
            },
            Command::Image {
                texture,
                src: node.image_src(ctx),
                dst,
                radius: CornerRadius {
                    radius: draw.scalar(node.corner_radius(ctx)),
                },
                shape: node.corner_shape(ctx),
                tint: node.image_tint(ctx).with_alpha(opacity),
                filter: filter.clone(),
                blend,
            },
        );
    }

    // 矢量图标内容（CPU 三角化 → `Command::Path`）。
    //
    // ⚠ **图像优先于图标**：两者都设了时只画图像，不画图标。
    //
    // 这是本函数里唯一一处"两个属性指向同一个视觉位置"的歧义，判据必须写死，
    // 否则症状是"图标压在图像上"或"图像不出现"——两者都看不出是选择规则的问题。
    //
    // 为什么图像胜出：它已经有位图缓存（不需要每帧三角化），且**位图自带抗锯齿**；
    // 而路径图元要走 MSAA 才能有干净的边缘。在两者都给定时，先用便宜且已就绪的那个
    // 是唯一不会退化画面的选择。
    //
    // 换句话说：**同时设两者是调用方的 bug**，而这个 bug 的最佳表现方式是
    // "图像正常显示"（少画一个图标），而不是崩溃或什么都不画。
    if node.image_texture(ctx).is_none() {
        if let Some(name) = node.icon_name(ctx) {
            if !name.is_empty() {
                // 这里传的是**未平移**的 `node.rect()`，不是上面那个 `rect` ——
                // 理由见 [`icon_command`] 里"几何缓存查找"那一段的警告。
                if let Some(cmd) =
                    icon_command(&name, node.rect(), node, ctx, &filter, blend, opacity, draw)
                {
                    out.push(
                        SortKey {
                            layer,
                            z: 0,
                            order: next_order(order),
                        },
                        cmd,
                    );
                }
            }
        }
    }

    // 宿主动态路径（Canvas 式动画，如旋转弧线）→ `Command::Path`。
    //
    // 与图标路径同律：颜色取 `icon-color`，顶点已是局部坐标，只套节点变换。
    // 排在图标之后 —— 三者（图像 / 图标 / 动态路径）指向同一视觉位置时，
    // 优先级与「图像胜出」那条规则保持一致：先便宜/已就绪的，最后才是每帧重建的。
    if node.image_texture(ctx).is_none() && node.icon_name(ctx).is_none_or(|n| n.is_empty()) {
        if let Some(data) = node.path_data(ctx) {
            if !data.is_empty() {
                // 局部坐标先套上节点矩形左上，再走与图标路径相同的变换链。
                let placed: std::sync::Arc<[Vertex]> = if data.local {
                    std::sync::Arc::from(
                        data.vertices
                            .iter()
                            .map(|v| Vertex {
                                position: Point::new(v.position.x + rect.x, v.position.y + rect.y),
                                uv: v.uv,
                            })
                            .collect::<Vec<_>>(),
                    )
                } else {
                    data.vertices.clone()
                };
                out.push(
                    SortKey {
                        layer,
                        z: 0,
                        order: next_order(order),
                    },
                    Command::Path {
                        vertices: transform_vertices(&placed, draw),
                        fill: node.icon_color(ctx).with_alpha(opacity),
                        filter: filter.clone(),
                        blend,
                    },
                );
            }
        }
    }

    // 文本内容
    //
    // 排在背景之后、子节点之前：文本属于节点自身的"外观"，与背景同一层；而子节点
    // 是叠在它上面的内容。若把文本排到最后，压在文本节点之上的子元素会被文字盖住。
    if let Some(mut cmd) = text_command(node, ctx, offset, opacity) {
        // `text_command` 造出命令后才补效果 —— 那时它已不再关心 style，只管排版。
        match &mut cmd {
            Command::Text {
                filter: f,
                blend: b,
                ..
            } => {
                *f = filter.clone();
                *b = blend;
            }
            _ => unreachable!("text_command 只产出 Command::Text"),
        }
        out.push(
            SortKey {
                layer,
                z: 0,
                order: next_order(order),
            },
            cmd,
        );
    }

    // 外部渲染源（拓展接口）。
    //
    // 排在节点自身外观（背景 / 图像 / 图标 / 文本）之后、子节点之前：它与文本
    // 同属"节点自己的内容平面"，而子节点叠在它上面。没挂源的节点（绝大多数）
    // 在这里零成本——一次空串判断。
    let source = node.render_source(ctx);
    if !source.is_none() {
        out.push(
            SortKey {
                layer,
                z: 0,
                order: next_order(order),
            },
            Command::Custom {
                bounds: rect,
                source,
                resource_key: None,
                filter: filter.clone(),
                blend,
            },
        );
    }

    // 裁剪：在子节点之前发出 PushClip、之后发出 PopClip。
    //
    // 裁剪区有两个来源（见 [`crate::style::Node::clip_rect`]）：显式的
    // `clip-rect` 属性，以及 `overflow: hidden / scroll / auto`（裁到视口）。
    // 两者都用**本节点自身**的位移（`draw.rect`）换算到屏幕坐标 —— 滚动偏移
    // **不能**进来：视口是不动的，动的是内容。
    //
    // PushClip/PopClip 参与 (layer, z, order) 排序，order 递增保证 PushClip 永远
    // 在子节点命令之前、PopClip 永远在子节点命令之后，嵌套 clip 也不会被拆散。
    let clip = node.clip_rect(ctx);
    if let Some(clip) = clip {
        let abs_clip = draw.rect(crate::core::Rect::new(
            node.rect().x + clip.x,
            node.rect().y + clip.y,
            clip.width,
            clip.height,
        ));
        out.push(
            SortKey {
                layer,
                z: 0,
                order: next_order(order),
            },
            Command::PushClip(abs_clip),
        );
    }

    // 子节点。
    //
    // ⚠ 传**已经推进过本节点**的 `state` —— 这是"容器不透明度影响整棵子树"
    // 与"祖先位移带着后代一起动"的唯一实现点。
    //
    // ⚠ 再叠一层 `descend_children`：滚动容器的子内容按 `-scroll_offset` 平移。
    // 容器自身的盒子（背景 / 边框 / 滚动条）不受影响。
    let child_state = state.descend_children(node, ctx);
    for child in node.children() {
        render_rec(child, ctx, order, out, child_state);
    }

    // 结束裁剪。
    if clip.is_some() {
        out.push(
            SortKey {
                layer,
                z: 0,
                order: next_order(order),
            },
            Command::PopClip,
        );
    }
}

/// 文本绘制命令（`None` = 不产出）。
///
/// 抽成自由函数而不是 [`Node`] 的方法：它需要 `&Node`（只读），而其余样式 getter
/// 都是 `&self` 的公开 API。把它留在 `impl Node` 上会让"公开词汇表"里混进一个
/// 只有绘制趟需要的东西。
///
/// # `offset` 为什么直接进 `origin` 而不是事后平移字形
///
/// [`crate::text::TextRender::glyph_quads`] 是 `(shaped, origin, style)` 的纯函数、
/// **没有缓存**（见 `text/mod.rs`），所以把平移量折进 `origin` 是零成本的。
/// 事后平移字形反而要重新分配一个 `Vec<GlyphQuad>` —— 那是"为了省一个加法而付一次分配"。
///
/// # `opacity` 为什么在这里就乘进文字色
///
/// 文字色是 `Color`（含 alpha），而 `Command::Text` 的字形颜色**只有一个** `color`
/// 字段 —— 没有任何"每字形不透明度"的机制。要让半透明容器里的文字也半透明，
/// 只能在**造命令时**就把因子乘进颜色。放在 [`render_rec`] 里回填是做不到的。
/// # 字形四边形缓存：为什么**按元素原点**存，而不是按屏幕位置
///
/// 元素原点 `origin` 每个 resize 帧都在变（那正是本轮优化要快的那一趟），
/// 而字形的**局部**位置一个像素都不会变。把原点折进 `glyph_quads` 的参数里，
/// 就等于把「每帧都要重算」变成了「每帧都要重算」——正是要消掉的那笔开销。
///
/// 官方对应物：`CLayoutStorage` 存的是相对元素原点的结果，变换在绘制趟叠加。
///
/// 代价是 [`Command::Text`] 多一个 `offset` 字段，且**三处**必须同步用它：
/// `text_command`（写入）、`command_content_rect`（滤镜范围）、`zephyra-fx::quad`
/// （顶点展开）。少改一处 ⇒ 那条路径上的几何整体错位，且不报任何错。
///
/// # 为什么文字不吃 `scale-x` / `scale-y`
///
/// 这里只收到 `offset`，**没有**收到 [`NodeDraw`]。理由是字形缓存按元素原点存
/// （见上一节），而 `Command::Text` 也没有缩放字段 —— 让文字跟着缩放要改
/// `Command::Text` + 三后端顶点展开 + 离屏范围，属于独立一轮。
/// 语义宣告在 [`crate::props::SCALE_X`] 的文档里："只作用于本节点自己的外观"，
/// 文字节点自己写 `scale-x` 才是"本节点的外观"，而这里说的是**容器的**缩放
/// 不该顺带放大子文字节点。
fn text_command(node: &Node, ctx: Context<'_>, offset: Point, opacity: f32) -> Option<Command> {
    let text = node.text()?;
    let engine = ctx.engine?;
    let style = node.text_style(ctx);
    let max_width = node.text_max_width(ctx);
    let base = node.rect();
    let origin = Point::new(base.x + offset.x, base.y + offset.y);
    let mut engine = engine.borrow_mut();
    // uv 与图集句柄由 `TextRender::glyph_quads` 给出 —— core 有字形图集
    // （`GlyphAtlas`，纯 CPU 数据），所以能算出 uv；但 core 不碰 GPU 纹理句柄，
    // 于是只用那个不透明的 `TextureRef`。
    //
    // ⚠ 缓存命中时**只**克隆 `Arc`；`atlas` 必须重新取一次，因为它可能随图集
    // 增长而变（`GlyphAtlas` 满了会重建纹理）。克隆 `Arc` 是引用计数 +1，
    // 零分配 —— 这就是这一段存在的全部意义。
    let revision = engine.atlas_revision();
    // ⚠ 判据是**排版输入指纹**（文本 / 样式 / `max_width`）+ 图集修订号。
    //
    // 不要改成「测量趟这帧有没有重算」：resize 时约束一变 `measure` 就会重跑，
    // 产出一个**内容相同**的 `MeasureResult`（只有 `size` 被 `pick_axis` 夹了一下）。
    // 用那种判据，命中率恒为 0 —— 这正是早先那版的错误，136 KB/帧 一分没省。
    let cached = {
        let slot = node.glyph_quads.borrow();
        match slot.as_ref() {
            Some(c)
                if c.revision == revision
                    && c.text.as_ref() == text
                    && c.style == style
                    && c.max_width == max_width =>
            {
                Some((c.atlas.clone(), std::sync::Arc::clone(&c.glyphs)))
            }
            _ => None,
        }
    };
    let (atlas, glyphs) = match cached {
        Some(hit) => hit,
        None => {
            // ⚠ 缓存未命中时**必须按当前文本重新排一次**，绝不能复用
            // `measure_cache` 里的 `shaped`（`node.cached_shaped()`）。
            //
            // 判据是「排版输入指纹」（文本 / 样式 / `max_width`）+ 图集修订号，
            // 所以未命中 ⟺ 这几样里至少一样变了。此刻 `text` 是**新的**，而
            // `cached_shaped()` 存的是**上一次测量趟**的结果。
            //
            // 两者的分歧不是纯理论：`TEXT` 被刻意判成**只重绘**属性
            // （`layout_classification_tests`：「内容变了不等于盒子要重排」），
            // 于是改文本**不清** `measure_cache`、也**不重跑** layout。若在这里
            // 拿旧 `shaped` 去算新 `text` 的字形，会连环两步错：
            //
            // 1. 算出的是旧文本的字形位置（数字从 48→44，字形仍是「48」那套）；
            // 2. 紧接着把**新** `text` 写回 `glyph_quads` 缓存（见下）——
            //    于是下一帧指纹对上了、命中，旧字形被永久钉住。
            //
            // 症状就是「点一下按钮值不变、画面不更新」，且要等一次 resize /
            // 换页（约束变了 → measure 重跑 → `shaped` 才跟上）才自愈。
            // 实测：gallery 玻璃页连点 3 次减号，读数一直停在 48，resize 后
            // 跳成正确的 40。
            let shaped = engine.shape(text, &style, max_width);
            let (atlas, v) = engine.glyph_quads(&shaped, Point::new(0.0, 0.0), &style)?;
            let g: std::sync::Arc<[crate::paint::GlyphQuad]> = v.into();
            // 借用守卫在**写回之前**就必须放掉：`glyph_quads` 已经是 `RefCell`，
            // 在还持着读守卫时 `borrow_mut()` 会 panic。上面的读借用写在
            // `let cached = { ... }` 的**独立作用域**里，正是为此。
            *node.glyph_quads.borrow_mut() = Some(Box::new(crate::node::GlyphQuadCache {
                text: text.into(),
                style: style.clone(),
                max_width,
                revision,
                atlas: atlas.clone(),
                glyphs: std::sync::Arc::clone(&g),
            }));
            (atlas, g)
        }
    };
    Some(Command::Text {
        atlas,
        glyphs,
        offset: origin,
        color: node
            .color(&crate::props::TEXT_COLOR, ctx)
            .unwrap_or(Color::rgb(1.0, 1.0, 1.0))
            .with_alpha(opacity),
        // 效果由 [`render_rec`] 回填 —— 它已经算好了 `filter`/`blend`，而本函数
        // 只该关心排版。给空链占位是为了让类型完整，值不参与渲染。
        filter: crate::filter::FilterChain::none(),
        blend: crate::filter::BlendMode::default(),
    })
}

fn next_order(order: &mut u32) -> u32 {
    let o = *order;
    *order += 1;
    o
}

/// 曲线展平的容差（DIP）。
///
/// # 为什么是 0.25 而不是"更精细"的值
///
/// 容差决定"曲线被折线逼近到多准"，代价是**三角形个数** —— 而三角化是**每帧**做的
/// （[`Command::Path`] 的顶点进了 `DisplayList`，下一帧重建时重新算）。
///
/// 对 24×24 的图标，0.25 DIP 意味着圆弧上大约每 1.5° 一个顶点：一个典型图标
/// 30~60 个三角形。足够光滑（比一个像素细），而量不到性能代价。
///
/// 再往下收（0.05）会让三角形数翻几倍，收益却看不见 —— 屏幕上仍然是一个 16~24 像素
/// 宽的图标，肉眼分辨不出 0.05 DIP 的曲率。
const FLATTEN_TOLERANCE: f32 = 0.25;
#[allow(clippy::too_many_arguments)]
// 参数是画一次所需的全部输入，彼此没有可成组打包的语义 —— 凑一个 struct 只是把
// 8 个参数搬进另一个名字，读的人反而要多跳一层。见 AGENTS.md「代码自解释优先」。
/// 图标 → [`Command::Path`]。
///
/// # 这条链是几何层唯一的生产入口
///
/// ```text
/// props::ICON_NAME  →  IconSet::get  →  flatten（曲线 → 折线）
///                    →  tessellate（多孔三角化）  →  Command::Path
/// ```
///
/// 核心只做中间两步 —— **"名字 → 数据"是宿主的事**（就像纹理句柄一样），
/// 所以这里从 [`IconSet`] 取，而 [`crate::icon::assets::REGISTRY`] 只是内置的
/// 一份默认表。宿主登记自己的图标走同一条路。
///
/// # 三角化缓存（在 [`IconSet`] 上，不在这里）
///
/// `flatten` + `triangulate` 的结果只取决于（图标, 目标矩形），与时间无关。
/// 缓存在 [`IconSet`] 上而不是本函数里，因为缓存要跨帧存活，而本函数每帧被调、
/// 手上什么都没有。
///
/// # ⚠ 这里曾经因为"估错了成本"而不做缓存，而那次估算的教训值得留着
///
/// 原文写的是：
///
/// > 而实测每帧成本只有几十微秒（`demo --bench` 的 `paint` 阶段中位数 78 µs，
/// > 其中图标占比很小），远低于缓存的复杂度与失效风险。所以**现在不做**。
///
/// 两处都错：
///
/// 1. **"图标占比很小"是估的，不是量的。** 那个 `paint` 阶段的中位数之所以只有
///    78 µs，是因为**当时 demo 里还没有矢量图标**（只有 8 条路径，是后来
///    接通的）。接通之后实测：`paint` 中位 **603 µs**，其中三角化
///    **50 µs/个 × 8 个 = 400 µs**，占 66%、占整帧 39%。
/// 2. **"几十微秒"恰好落在"值不值得"的边界上**，而边界附近的估算错误最贵 ——
///    它是唯一一种看起来有依据的猜测。
///
/// 修掉之后：`paint` 603 → **107 µs**，整帧 1021 → **504 µs**。
///
/// 教训与本 crate 里另外两处同源（`glyph_quads` 的重复栅格化、`PropertyChange`
/// 的多余 `Arc::new`）：**性能判断必须来自计时，"估算 + 论证"不算**。
/// 论证只会让猜测看起来更可信。
///
/// # 返回 `None` 的两种情况
///
/// - 名字未登记 → [`Icon::EMPTY`]，无路径段 → 不产出命令；
/// - 三角化失败（自相交等）→ 不产出命令。
///
/// 两者都**不** panic：一个拼错的图标名不该让整个界面崩掉，而"某个图标不显示"
/// 是可见、可对比的。
fn icon_command(
    name: &str,
    rect: Rect,
    node: &Node,
    ctx: Context<'_>,
    filter: &crate::filter::FilterChain,
    blend: crate::filter::BlendMode,
    opacity: f32,
    draw: NodeDraw,
) -> Option<Command> {
    let set = ctx.icon_set()?;
    let icon = set.get(name);
    if icon.is_empty() {
        return None;
    }
    // ⚠ 命中缓存就直接返回 —— 这一步在稳态下把每个图标的 50 µs 三角化降到
    // 一次哈希查找。判据是**整个映射的输入**：图标数据 + 目标矩形（见
    // `IconSet::cached_mesh`）。
    let (vx, vy, vw, vh) = icon.view_box;
    if vw <= 0.0 || vh <= 0.0 {
        return None;
    }

    // ---- 几何缓存查找 ----
    //
    // 必须放在**展平之前**：展平 + 三角化是整条链里唯一贵的部分（实测 50 µs/个），
    // 而它只由（图标，目标矩形）决定 —— 见 [`crate::icon::IconSet`] 的
    // `GeometryCache` 文档。
    //
    // ⚠ 这里用的必须是**未平移**的 `rect`。若拿平移后的矩形当键，入场动画的
    // 每一帧位移都不同 ⇒ 缓存次次落空 ⇒ demo 的 19 个图标 × 50 µs = **每帧 1 ms**，
    // 而动画只有 333 ms —— 也就是整个动画期间都在为"省一次顶点加法"付三角化的钱。
    // 平移挪到产出之后做（见 `shift_vertices`），静止时它连分配都没有。
    if let Some((vertices, _triangles)) = set.cached_mesh(icon, rect) {
        if vertices.is_empty() {
            return None;
        }
        // 只加 refcount，不拷贝顶点 —— 这正是"命中缓存"该有的成本。
        //
        // ⚠ 早于本轮，`Command::Path` 的 `vertices` 是 `Vec<Vertex>`，所以这里
        // 无论缓存命中与否都要 `mesh.vertices.clone()`：demo 的 8 个图标
        // 519 个顶点每帧拷一遍，拷完当帧丢掉。改成 `Arc<[Vertex]>` 之后
        // 缓存命中那条路变成零分配 —— 而那正是缓存存在的意义。
        return Some(Command::Path {
            vertices: transform_vertices(&vertices, draw),
            fill: node.icon_color(ctx).with_alpha(opacity),
            filter: filter.clone(),
            blend,
        });
    }

    // viewBox → 节点矩形：等比缩放并居中。
    //
    // 等比是必须的（非等比会把圆形图标压成椭圆），居中是因为节点矩形与图标
    // 宽高比不一致时不该把图形拉偏 —— 那看起来是"图标歪了"。
    let scale = (rect.width / vw).min(rect.height / vh);
    let w = vw * scale;
    let h = vh * scale;
    let ox = rect.x + (rect.width - w) * 0.5 - vx * scale;
    let oy = rect.y + (rect.height - h) * 0.5 - vy * scale;
    let map = |p: Point| Point::new(ox + p.x * scale, oy + p.y * scale);

    // 展平：`Vec<Contour>`（见 [`crate::geom::FlatPath`]），每条轮廓是闭合的。
    let flat = crate::geom::flatten(&icon.segments, FLATTEN_TOLERANCE);
    if flat.is_empty() {
        return None;
    }
    // viewBox → 节点矩形的映射，展平之后立刻套上（三角化只关心最终坐标）。
    let contours: Vec<crate::geom::Contour> = flat
        .into_iter()
        .map(|c| crate::geom::Contour::new(c.points().iter().copied().map(map).collect()))
        .filter(|c| c.is_fillable())
        .collect();
    if contours.is_empty() {
        return None;
    }

    let rule = match icon.paint.rule {
        crate::icon::FillRule::NonZero => crate::geom::FillRule::NonZero,
        crate::icon::FillRule::EvenOdd => crate::geom::FillRule::EvenOdd,
    };
    let mesh = crate::geom::triangulate(&contours, rule).ok()?;
    if mesh.vertices.is_empty() {
        return None;
    }
    let vertices = set.cache_mesh(icon, rect, mesh);

    Some(Command::Path {
        vertices: transform_vertices(&vertices, draw),
        fill: node.icon_color(ctx).with_alpha(opacity),
        filter: filter.clone(),
        blend,
    })
}

/// 把整条路径的顶点套上绘制变换（绕元素中心缩放 + 平移）。
///
/// # 静止时**不分配**
///
/// 恒等变换直接返回原引用（`Arc` 克隆只是 refcount +1）。这不是优化洁癖：
/// 入场动画之外的所有绘制都走这条路，而 Gallery 每帧有 19 个图标 ——
/// 每帧 19 次 `Vec` + 19 次 `Arc` 分配换 0 像素位移，是笔亏本买卖。
///
/// ⚠ `uv` **不动**：图集的归一化坐标与屏幕位置无关，平移 / 缩放屏幕几何
/// 不该动它。
#[inline]
fn transform_vertices(
    vertices: &std::sync::Arc<[Vertex]>,
    draw: NodeDraw,
) -> std::sync::Arc<[Vertex]> {
    if draw.is_identity() {
        return vertices.clone();
    }
    std::sync::Arc::from(
        vertices
            .iter()
            .map(|v| Vertex {
                position: draw.point(v.position),
                uv: v.uv,
            })
            .collect::<Vec<Vertex>>(),
    )
}

//! Shared visual/damage geometry (R2: D2/D8/D11/D17/D18).
//!
//! Paint, frame damage, and hit testing must use the same transform and bounds
//! rules. Keeping them in one place avoids "looks painted but not hittable" or
//! damage/residual artifacts.

use crate::context::Context;
use crate::core::{Point, Rect};
use crate::node::Node;
use crate::paint::{union_of_quads, Layer};

/// Traversal state for visual geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualState {
    /// Accumulated ancestor translate (cleared at `position: root` boundary per D11).
    pub translate: Point,
    /// Effective compositing layer (D18: ancestor context propagation).
    pub layer: Layer,
    /// Node-level preorder rank for paint-equivalent ordering (D13).
    pub preorder: u32,
}

impl VisualState {
    pub const ROOT: Self = Self {
        translate: Point::ZERO,
        layer: Layer::Content,
        preorder: 0,
    };
}

/// Enter a node: returns `(own_state, own_visual_rect, is_root_translate_boundary)`.
///
/// D11: entering `position: root` clears ancestor accumulated translate; node's
/// own translate still applies. Opacity still multiplies (handled by paint).
///
/// # 两个 translate 不一样：自己的 vs 子节点的
///
/// 返回的 `Point`（**自己的**位移）用于本节点的视觉矩形；
/// [`VisualState::translate`]（**传给子节点的**基准位移）额外扣掉了本节点的
/// 滚动偏移。
///
/// 这正是滚动容器要的形状：容器自身的盒子（背景 / 边框 / 滚动条轨道）留在视口
/// 原位，被滚动的是**子内容**。绘制趟用同一条规则（见
/// `crate::paint::PaintState` 的 `descend_children` 方法），命中测试与脏区因此与画面
/// 一致 —— 不会出现"看得见却点不到"或"滚过去留残影"。
pub fn enter(node: &Node, ctx: Context<'_>, parent: VisualState) -> (VisualState, Point) {
    let base_translate = if node.position(ctx) == crate::layout::Position::Root {
        Point::ZERO
    } else {
        parent.translate
    };
    let own_translate = Point::new(
        base_translate.x + sanitize(node.translate_x(ctx)),
        base_translate.y + sanitize(node.translate_y(ctx)),
    );
    // 滚动容器：子内容按 -scroll_offset 平移（容器自身不平移）。
    let scroll = node.scroll_offset(ctx);
    let child_translate = Point::new(own_translate.x - scroll.x, own_translate.y - scroll.y);
    let layer = effective_layer(node, ctx).unwrap_or(parent.layer);
    let preorder = parent.preorder;
    (
        VisualState {
            translate: child_translate,
            layer,
            preorder,
        },
        own_translate,
    )
}

/// Effective layer for a node (D18 option ②: explicit node layer wins, else ancestor).
pub fn effective_layer(node: &Node, ctx: Context<'_>) -> Option<Layer> {
    if node.props().lookup(&crate::props::LAYER).is_some() {
        Some(node.layer(ctx))
    } else {
        None
    }
}

/// Visual rectangle for paint/hit (D2/D8/D17): transformed border box union text bounds.
/// Does not include filter/decoration outsets (those belong to damage bounds).
pub fn visual_rect(node: &Node, ctx: Context<'_>, translate: Point) -> Rect {
    let box_rect = transformed_border_box(node, ctx, translate);
    if let Some(text) = text_visual_bounds(node, ctx, translate) {
        union_rects(box_rect, text)
    } else {
        box_rect
    }
}

/// Damage bounds for frame dirty regions: visual rect plus filter/decoration outsets.
pub fn damage_bounds(node: &Node, ctx: Context<'_>, translate: Point) -> Rect {
    let mut b = visual_rect(node, ctx, translate);
    let filter = node.filter(ctx);
    if !filter.is_identity() {
        b = filter.bounds_for(b);
    }
    let outsets = node.decorations(ctx).outsets();
    if !outsets.is_zero() {
        b = b.outset_by(outsets);
    }
    b
}

/// Border box transformed by self scale (around layout center) and accumulated translate.
/// Scale is self-only (D17/paint semantics).
pub fn transformed_border_box(node: &Node, ctx: Context<'_>, translate: Point) -> Rect {
    let r = node.rect();
    let sx = sanitized_scale(node.scale_x(ctx));
    let sy = sanitized_scale(node.scale_y(ctx));
    let cx = r.x + r.width * 0.5;
    let cy = r.y + r.height * 0.5;
    let x0 = cx + (r.x - cx) * sx;
    let y0 = cy + (r.y - cy) * sy;
    Rect::new(
        x0 + translate.x,
        y0 + translate.y,
        (r.width * sx).abs(),
        (r.height * sy).abs(),
    )
}

/// Text visual bounds fallback (D17).
///
/// Prefers valid glyph quad cache; falls back to shaping when engine is available;
/// returns `None` when no text or no safe bounds can be produced.
pub fn text_visual_bounds(node: &Node, ctx: Context<'_>, translate: Point) -> Option<Rect> {
    let text = node.text()?;
    let style = node.text_style(ctx);
    let max_width = node.text_max_width(ctx);
    let base = node.rect();
    let origin = Point::new(base.x + translate.x, base.y + translate.y);

    // 1) Valid glyph cache (paint may have filled it; hit/damage before paint may not).
    if let Some(engine) = ctx.engine {
        let revision = engine.borrow().atlas_revision();
        let cached = node.glyph_quads.borrow();
        if let Some(c) = cached.as_ref() {
            if c.revision == revision
                && c.text.as_ref() == text
                && c.style == style
                && c.max_width == max_width
                && !c.glyphs.is_empty()
            {
                return Some(union_of_quads(&c.glyphs, origin));
            }
        }
        drop(cached);
        // 2) Conservative shape fallback when engine available.
        let shaped = engine.borrow_mut().shape(text, &style, max_width);
        if shaped.size.width > 0.0 || shaped.size.height > 0.0 {
            return Some(Rect::new(
                origin.x,
                origin.y,
                shaped.size.width,
                shaped.size.height,
            ));
        }
    }
    None
}

fn union_rects(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = a.right().max(b.right());
    let y1 = a.bottom().max(b.bottom());
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

fn sanitized_scale(v: f32) -> f32 {
    if v.is_finite() && v > 0.0 {
        v
    } else {
        1.0
    }
}

fn sanitize(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

#[cfg(test)]
#[path = "../tests/unit/visual.rs"]
mod tests;

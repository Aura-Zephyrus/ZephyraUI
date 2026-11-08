//! 命中测试：把指针位置解析成「落在哪个控件上」(R2: D2/D8/D11/D13/D14/D17/D18)。

use crate::context::Context;
use crate::core::{Point, Rect};
use crate::id::ElementKey;
use crate::node::Node;
use crate::paint::Layer;
use crate::visual::{self, VisualState};

/// 事件路径的最大深度。
pub const MAX_DEPTH: usize = 16;

/// 命中测试请求。
#[derive(Debug, Clone, Copy)]
pub struct HitTestRequest {
    /// 指针位置（窗口/DIP 坐标）。
    pub point: Point,
}

/// 一次命中测试的结果。
#[derive(Debug, Default, Clone)]
pub struct HitResult {
    pub path: Vec<HitNode>,
    pub truncated: bool,
}

impl HitResult {
    pub fn clear(&mut self) {
        self.path.clear();
        self.truncated = false;
    }

    /// 路径上最后一个 `interactive && inside` 的节点。
    pub fn target(&self) -> Option<&HitNode> {
        self.path.iter().rev().find(|n| n.interactive && n.inside)
    }

    pub fn hit_any(&self) -> bool {
        self.path.iter().any(|n| n.inside)
    }
}

/// 命中路径中的一个节点。
#[derive(Debug, Clone, PartialEq)]
pub struct HitNode {
    pub id: Option<ElementKey>,
    /// 视觉矩形（D2/D8/D17）。
    pub rect: Rect,
    pub interactive: bool,
    /// 该节点自身的视觉矩形是否包含测试点。
    pub inside: bool,
}

pub fn hit_test(node: &Node, req: HitTestRequest, ctx: Context<'_>, out: &mut HitResult) -> bool {
    out.clear();
    let mut best: Option<(Layer, i32, u32, Vec<HitNode>)> = None;
    let mut preorder: u32 = 0;
    let mut ancestors: Vec<HitNode> = Vec::new();
    walk(
        node,
        req.point,
        ctx,
        VisualState::ROOT,
        &mut preorder,
        &mut ancestors,
        &mut best,
    );
    if let Some((_, _, _, path)) = best {
        if path.len() > MAX_DEPTH {
            out.truncated = true;
            out.path.extend(path.into_iter().take(MAX_DEPTH));
        } else {
            out.path = path;
        }
        true
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    node: &Node,
    point: Point,
    ctx: Context<'_>,
    parent: VisualState,
    preorder: &mut u32,
    ancestors: &mut Vec<HitNode>,
    best: &mut Option<(Layer, i32, u32, Vec<HitNode>)>,
) {
    if node.is_collapsed(ctx) {
        return;
    }
    let (state, own_translate) = visual::enter(node, ctx, parent);
    let vrect = visual::visual_rect(node, ctx, own_translate);
    let inside = rect_contains(vrect, point);
    let order = *preorder;
    *preorder = preorder.saturating_add(1);

    ancestors.push(HitNode {
        id: node.key().cloned(),
        rect: vrect,
        interactive: node.is_interactive(ctx),
        inside,
    });

    if inside {
        let rank = (state.layer, 0i32, order);
        let replace = match best {
            None => true,
            Some((bl, bz, bo, _)) => rank > (*bl, *bz, *bo),
        };
        if replace {
            *best = Some((rank.0, rank.1, rank.2, ancestors.clone()));
        }
    }

    // 子节点不剪到父矩形（D14/溢出/手动定位可见即可命中）。
    for child in node.children() {
        walk(child, point, ctx, state, preorder, ancestors, best);
    }
    ancestors.pop();
}

pub fn rect_contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.x < rect.x + rect.width
        && point.y >= rect.y
        && point.y < rect.y + rect.height
}

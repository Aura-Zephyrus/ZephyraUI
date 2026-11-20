//! 回归：图集淘汰重排发生时，一帧 render 产出的所有文本 quad 必须属于
//! **同一份**图集布局（同一 revision）。
//!
//! 这是 ViewStage「设置页部分文字错乱」的根因测试。formerly：paint 逐节点调
//! `glyph_quads`，中途图集满触发淘汰重排 → 先算节点的 uv 失效 → 与最终上传的
//! 图集错位 → 前段文字串到别的字形槽位。修复后 render 会在 revision 变化时
//! 重跑到收敛。

use std::cell::RefCell;
use std::collections::HashMap;

use zephyra_core::context::Context;
use zephyra_core::core::Point;
use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::paint::{Command, DisplayList, GlyphQuad, TextureRef};
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_core::text::atlas::AtlasRaster;
use zephyra_core::text::sdf::{from_coverage, SDF_BASE_PX, SDF_PADDING, SDF_SPREAD};
use zephyra_core::text::{GlyphSource, PlacedGlyph, TextEngine, TextRender, TextStyle, TextStack};
use zephyra_core::theme::{Theme, ThemeVariant};

/// 每个不同字符 → 不同字形 id，逼图集淘汰重排。
#[derive(Debug)]
struct DistinctSource;

impl GlyphSource for DistinctSource {
    fn glyph_id(&mut self, ch: char) -> u32 {
        ch as u32
    }
    fn advance(&mut self, _glyph_id: u32, _px_size: f32) -> f32 {
        10.0
    }
    fn rasterize(&mut self, _glyph_id: u32, px_size: u32) -> Option<AtlasRaster> {
        let n = (px_size / 4).max(8);
        let cov = vec![255u8; (n * n) as usize];
        Some(AtlasRaster {
            offset: Point::new(0.0, 0.0),
            width: n,
            height: n,
            sdf: from_coverage(&cov, n, n, SDF_PADDING),
            spread: SDF_SPREAD,
            base_px: SDF_BASE_PX as f32,
        })
    }
    fn font_id(&self) -> u32 {
        0
    }
}

/// 包装真实 TextStack，记录每个文本最后一次 `glyph_quads` 时的图集 revision。
struct TrackingEngine {
    inner: TextStack<DistinctSource>,
    /// 文本标识 → 最后一次算出其 quad 时的 revision（调用后读）。
    last_rev: RefCell<HashMap<String, u64>>,
    /// 本趟观察到的最大 revision。
    max_rev: RefCell<u64>,
}

impl TrackingEngine {
    fn reset(&self) {
        self.last_rev.borrow_mut().clear();
        *self.max_rev.borrow_mut() = 0;
    }
}

impl TextEngine for TrackingEngine {
    fn shape(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: Option<f32>,
    ) -> zephyra_core::text::ShapedText {
        self.inner.shape(text, style, max_width)
    }
    fn advance(&mut self, ch: char, style: &TextStyle) -> f32 {
        self.inner.advance(ch, style)
    }
    fn line_height(&mut self, style: &TextStyle) -> f32 {
        self.inner.line_height(style)
    }
}

impl TextRender for TrackingEngine {
    fn glyph_quads(
        &mut self,
        shaped: &zephyra_core::text::ShapedText,
        origin: Point,
        style: &TextStyle,
    ) -> Option<(TextureRef, Vec<GlyphQuad>)> {
        let key: String = shaped
            .lines
            .iter()
            .flat_map(|l| l.glyphs.iter())
            .map(|g: &PlacedGlyph| char::from_u32(g.glyph_id).unwrap_or('?'))
            .collect();
        let out = self.inner.glyph_quads(shaped, origin, style);
        if out.is_some() {
            let rev = self.inner.atlas_revision();
            self.last_rev.borrow_mut().insert(key, rev);
            let mut m = self.max_rev.borrow_mut();
            if rev > *m {
                *m = rev;
            }
        }
        out
    }
    fn atlas_revision(&self) -> u64 {
        self.inner.atlas_revision()
    }
}

/// 一组每个节点文本都不同的树；`base` 决定用哪段 Unicode 区间的字符。
fn many_texts(n: usize, base: u32) -> Element {
    let children: Vec<Element> = (0..n)
        .map(|i| {
            let ch = char::from_u32(base + i as u32).unwrap();
            Element::new(NodeTag::Box).with(
                &props::TEXT,
                PropertyValue::static_str(Box::leak(ch.to_string().into_boxed_str())),
            )
        })
        .collect();
    Element::new(NodeTag::Box)
        .with_key("root")
        .with_children(children)
}

fn render_tree(element: &Element, theme: &Theme, engine: &RefCell<TrackingEngine>) {
    let node = zephyra_core::node::Node::from_element(element);
    let ctx = Context::with_engine(theme, engine);
    let mut out = DisplayList::new();
    zephyra_core::paint::render(&node, ctx, &mut out);
    assert!(out.iter().any(|i| matches!(i.command, Command::Text { .. })));
}

#[test]
fn all_text_quads_share_final_revision_after_eviction() {
    // 图集容量：px=14 ⇒ tier 64 ⇒ padded 32×32=1024px²/字形。
    // 224² ≈ 50K ⇒ 约 49 个字形。每帧 40 个能装下，但两帧累计 80 个会淘汰。
    let inner = TextStack::with_atlas_size(DistinctSource, 224);
    let engine = RefCell::new(TrackingEngine {
        inner,
        last_rev: RefCell::new(HashMap::new()),
        max_rev: RefCell::new(0),
    });
    let theme = Theme::builtin(ThemeVariant::Dark);

    // 第一帧：用 0x4E00 段填满图集（制造历史残留）。
    render_tree(&many_texts(40, 0x4E00), &theme, &engine);

    // 第二帧：换 0x5000 段 —— 新字形挤掉历史残留，触发淘汰重排。
    engine.borrow().reset();
    render_tree(&many_texts(40, 0x5000), &theme, &engine);

    let final_rev = engine.borrow().inner.atlas_revision();
    let last_rev = engine.borrow().last_rev.borrow().clone();
    let max_rev = *engine.borrow().max_rev.borrow();

    assert!(max_rev > 0, "第二帧应触发图集淘汰重排");

    // 核心回归判据：第二帧所有文本节点最后一次算 quad 时的 revision 必须
    // 等于最终值。若有节点停留在旧 revision，它的 uv 就属于旧布局 —— 正是
    // 截图里串位的文字。
    let bad: Vec<_> = last_rev
        .iter()
        .filter(|(_, rev)| **rev != final_rev)
        .collect();
    assert!(
        bad.is_empty(),
        "有 {} 个文本节点的 quad 停留在旧图集 revision（最终 {}）: {:?}",
        bad.len(),
        final_rev,
        bad
    );
}

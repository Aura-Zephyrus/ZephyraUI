//! 把样式表应用到树上。
//!
//! # 时机：在 diff **之后**、在 layout **之前**
//!
//! 这是硬契约。`apply` 可能写布局键（标 `Dirty::Layout`），`FrameCache`
//! 会据此决定是否重排；见 core `frame` 与本模块的历史约束。

use std::collections::HashMap;
use std::sync::Arc;
use zephyra_core::id::NodeTag;
use zephyra_core::node::Node;
use zephyra_core::property::{PropertyKey, PropertyValue};

use crate::ir::StyleSheet;

type MatchOut = Arc<[(&'static PropertyKey, PropertyValue)]>;

/// 命中缓存：`(tag, 分类名)` → 命中的声明。
///
/// # 为什么分两层
///
/// 缓存的键必须**拥有**分类名（`Box<[&'static str]>`），但**查找不能分配**。
/// `HashMap` 的借用查找要求 `K: Borrow<Q>` 且 `Q` 能从 `&K` 借出 —— 组合键
/// `(NodeTag, Vec<..>)` 做不到（`Borrow` 不给元组实现，而自引用的借用键在
/// Rust 里也造不出来）。
///
/// 所以拆成「`NodeTag` → 该标签下的分类名表」两层：
/// 外层 `HashMap<NodeTag, _>` 的键是 `Copy` 枚举（查找零分配），
/// 内层是一张**短表**，线性扫 —— 同一标签下的分类名组合通常只有几个
/// （`nav-item selected` / `nav-item` / …），比一次堆分配便宜得多。
///
/// # 旧实现错在哪
///
/// 旧实现是 `HashMap<(NodeTag, Vec<&'static str>), _>`，查找前先
/// `s.names.clone()` —— 于是**每节点每次分配**。实测（gallery，release，
/// 稳态）这一处独占每帧 390 次分配中的约 88 次。
type MatchCache = HashMap<NodeTag, Vec<(Box<[&'static str]>, MatchOut)>>;

impl StyleSheet {
    /// 把整份样式表应用到这棵子树。
    pub fn apply_all(&self, root: &mut Node) {
        self.apply_to(root);
    }

    /// 应用到一个节点及其全部后代（无匹配缓存）。
    pub fn apply_to(&self, node: &mut Node) {
        let mut s = Scratch {
            names: Vec::new(),
            want: Vec::new(),
        };
        self.apply_to_inner(node, &mut s, None);
    }

    fn apply_to_inner(&self, node: &mut Node, s: &mut Scratch, mut cache: Option<&mut MatchCache>) {
        apply_node(self, node, s, cache.as_deref_mut());
        for child in node.children_mut() {
            self.apply_to_inner(child, s, cache.as_deref_mut());
        }
    }

    fn match_decls(
        &self,
        tag: NodeTag,
        names: &[&'static str],
    ) -> Vec<(&'static PropertyKey, PropertyValue)> {
        let mut want: Vec<(&'static PropertyKey, PropertyValue)> = Vec::new();
        for rule in self.rules() {
            if rule.is_empty() {
                continue;
            }
            if !rule.selector.matches(tag, names) {
                continue;
            }
            for decl in &rule.declarations {
                match want.iter_mut().find(|(k, _)| std::ptr::eq(*k, decl.key)) {
                    Some(slot) => slot.1 = decl.value.clone(),
                    None => want.push((decl.key, decl.value.clone())),
                }
            }
        }
        want
    }
}

/// 让样式表能装进 [`zephyra_core::frame::FrameCache::set_styler`]。
pub struct SheetStyler {
    pub sheet: Arc<StyleSheet>,
    cache: MatchCache,
}

impl SheetStyler {
    pub fn new(sheet: Arc<StyleSheet>) -> Self {
        Self {
            sheet,
            cache: HashMap::new(),
        }
    }
}

impl From<Arc<StyleSheet>> for SheetStyler {
    fn from(sheet: Arc<StyleSheet>) -> Self {
        Self::new(sheet)
    }
}

impl std::ops::Deref for SheetStyler {
    type Target = StyleSheet;
    fn deref(&self) -> &Self::Target {
        &self.sheet
    }
}

impl zephyra_core::styler::Styler for SheetStyler {
    fn apply(&mut self, root: &mut Node) {
        let mut s = Scratch {
            names: Vec::new(),
            want: Vec::new(),
        };
        self.sheet
            .apply_to_inner(root, &mut s, Some(&mut self.cache));
    }
}

/// 跨节点复用的暂存缓冲。
struct Scratch {
    names: Vec<&'static str>,
    want: Vec<(&'static PropertyKey, PropertyValue)>,
}

fn apply_node(
    sheet: &StyleSheet,
    node: &mut Node,
    s: &mut Scratch,
    cache: Option<&mut MatchCache>,
) {
    let tag = node.tag();
    s.names.clear();
    s.names.extend(node.style_names().iter().copied());

    if let Some(cache) = cache {
        // 查找**零分配**：外层按 `NodeTag` 哈希，内层线性扫分类名表。
        let bucket = cache.entry(tag).or_default();
        let hit = bucket
            .iter()
            .position(|(names, _)| names.as_ref() == s.names.as_slice());
        let decls = match hit {
            Some(i) => bucket[i].1.clone(),
            None => {
                let out: MatchOut = Arc::from(sheet.match_decls(tag, &s.names).into_boxed_slice());
                bucket.push((s.names.as_slice().into(), out.clone()));
                out
            }
        };
        for (key, value) in decls.iter() {
            // ⚠ 借用版：`PropertyValue::Filter` 持一个 `Vec<Filter>`，
            // 按值版在调用点就要克隆它 —— 那是一次堆分配，而 `apply` 每帧
            // 都把同一条规则再写一遍。
            node.set_styled_prop_ref(key, value);
        }
        node.retain_styled_props(|key| decls.iter().any(|(k, _)| std::ptr::eq(*k, key)));
    } else {
        s.want.clear();
        s.want.extend(sheet.match_decls(tag, &s.names));
        for (key, value) in &s.want {
            node.set_styled_prop(key, value.clone());
        }
        node.retain_styled_props(|key| s.want.iter().any(|(k, _)| std::ptr::eq(*k, key)));
    }
}

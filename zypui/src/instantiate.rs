//! 实例化：声明树 + [`Host`] → `Element`。每帧一跑。
//!
//! # 宿主契约 [`Host`]
//!
//! | 方法 | 给什么 |
//! |---|---|
//! | [`Host::binding`] | `@name` 绑定：路由、开关、选中项……应用状态 |
//! | [`Host::tr`] | `tr("key")`：i18n 文案 |
//! | [`Host::hovers`] / [`Host::holds`] | `hover` / `press` 内置词的输入状态 |
//!
//! 事件（click 之后干什么）**不在契约里**：key 就是事件路由的身份，
//! 宿主拿 `InputState` 的 click key 自己分派 —— `.zypui` 只声明结构，
//! 行为留在 Rust（2026-10-25 定稿）。
//!
//! # `hover` / `press` 的「当前实例 key」
//!
//! 只在**组件实例**边界（[`Node::Use`]）更新：实例 key 既是根元素的身份，
//! 也是模板内所有节点（含固定部件 key 的图标位/文字位）的输入状态基准 ——
//! 与 Rust 控件「构造器把 hovered 扇出给子节点」同一条数据流。
//! 模板内部元素自己的 key **不**改写它；页面层裸元素因此也不该写 `{hover}`
//! （交互态只住在组件模板里，这是规矩不是限制）。
//!
//! # 组件展开为什么在每帧做
//!
//! 组件实参是**表达式**（`checked="@pill_on"`），逐帧变值；实参进作用域层、
//! 模板体带着它实例化，一次遍历同时完成展开与求值。加载期做语法级替换
//! 的方案被否掉：两份表示 + 克隆一整棵树，换来的只是省一次名字查表。

use crate::eval::{eval, Scope};
use crate::intern::Interner;
use crate::ir::{ClassTok, Node, RawUi};
use crate::val::Val;
use crate::ZypuiError;
use zephyra_core::element::Element;
use zephyra_core::id::ElementKey;
use zephyra_core::layout::Overflow;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// 宿主为 `.zypui` 提供的运行期数据。见模块文档的契约表。
pub trait Host {
    /// `@name` 绑定。未绑定的名字求值为假（界面直接可见，不会静默错值）。
    fn binding(&self, name: &str) -> Option<Val>;
    /// i18n：按消息键取当前语言文案（查不到返回键名本身，与 gallery 现状一致）。
    ///
    /// 键恒为 `&'static`（文案键本来就要求编译期常量 —— 运行期拼出来的键
    /// 每帧都在泄漏，本框架的宿主按这个约定返回零分配的 `&'static str`）。
    fn tr(&self, key: &'static str) -> &'static str;
    /// `key` 是否正被悬停。
    fn hovers(&self, key: &str) -> bool;
    /// `key` 是否正被按住。
    fn holds(&self, key: &str) -> bool;
    /// `key` 是否拥有键盘焦点（供模板发 `focused` 分类名做焦点环）。
    ///
    /// 与 [`Self::hovers`] / [`Self::holds`] 同构：模板据此扇出 `focused` 类，
    /// `.zstyle` 才能按分类名给它加焦点边框——`.zstyle` 选择器只认类型名与分类名，
    /// 不匹配 [`zephyra_core::props::FOCUSED`] 属性。
    fn is_focused(&self, key: &str) -> bool {
        let _ = key;
        false
    }

    /// 按 `.zypui` 的 `render-source="kind"` 查找宿主拥有的载荷。
    ///
    /// 模板只声明 kind，不构造或修改 payload；返回值应复用宿主持有的稳定
    /// `RenderSource`，避免每帧创建新载荷触发 diff。
    fn render_source(&self, kind: &str) -> Option<zephyra_core::paint::RenderSource> {
        let _ = kind;
        None
    }

    /// 宿主自定义属性键（`PropertyKey` 相等按**地址**，模板写不了应用键 ——
    /// 键的真相在宿主手里）。`url` 属性由此解析；返回 `None` 时该属性被跳过。
    fn prop_key(&self, name: &str) -> Option<&'static zephyra_core::property::PropertyKey> {
        let _ = name;
        None
    }

    /// 可选：全局状态修订号。`None` = 不提供，缓存退化为逐依赖求值比较。
    ///
    /// 宿主在可能影响 `.zypui` 求值结果的状态变化时递增它。它只是加速，
    /// **不能单独作为正确性来源**；debug/test 可在 revision 命中时抽样比对。
    fn state_revision(&self) -> Option<u64> {
        None
    }

    /// 可选：单个 `@binding(name)` 的修订号。`None` = 不提供，缓存求值该绑定并比较 `Val`。
    ///
    /// 适用于 `binding()` 每帧会分配（如构造 `Vec<Rec>`）但值语义稳定的场景。
    /// 返回值与 `binding()` 的实际结果不一致是宿主 bug；debug/test 应尽可能断言。
    fn binding_revision(&self, _name: &str) -> Option<u64> {
        None
    }

    /// 可选：宿主实例作用域。共享 `UiRuntime` 缓存时，不同宿主实例应返回不同值，
    /// 避免相同 revision/快照形状下跨实例复用陈旧结果（测试/多窗口场景）。
    fn cache_scope(&self) -> u64 {
        0
    }
}

/// 一个加载好的 `.zypui` 工件：IR + 组件名索引。应用持一份，跨帧复用。
pub struct Ui {
    /// 解码/解析出的原始 IR。
    pub raw: RawUi,
    /// 组件名 → `raw.components` 下标（`Ui::new` 建一次，每帧查 O(1)）。
    pub(crate) idx: HashMap<&'static str, usize>,
}

impl Ui {
    /// 建索引。组件重名在此报错（zypuic 也查，这里是运行时的最后防线）。
    pub fn new(raw: RawUi) -> Result<Self, ZypuiError> {
        let mut idx = HashMap::with_capacity(raw.components.len());
        for (i, c) in raw.components.iter().enumerate() {
            if idx.insert(c.name, i).is_some() {
                return crate::err(format!("组件 `{}` 重名", c.name));
            }
        }
        Ok(Self { raw, idx })
    }

    /// 按页名实例化。页名不存在是**错**（拼错页名要响，不能静默空白）。
    pub fn build(
        &self,
        page: &str,
        host: &dyn Host,
        it: &mut Interner,
    ) -> Result<Vec<Element>, ZypuiError> {
        let page = self
            .raw
            .pages
            .iter()
            .find(|p| p.name == page)
            .ok_or_else(|| ZypuiError {
                msg: format!("页 `{page}` 不存在"),
            })?;
        let mut out = Vec::new();
        let mut ctx = Ctx {
            host,
            cur_key: "",
            it,
            comps: &self.idx,
            raw: &self.raw,
            depth: 0,
            cache: None,
        };
        build_list(
            &page.root,
            &Scope::ROOT,
            &mut ctx,
            None,
            &mut None,
            1,
            &mut out,
        )?;
        Ok(out)
    }

    /// P1：带 per-element cache 的构建路径。`cache: None` 时语义同 [`Self::build`]。
    pub(crate) fn build_cached(
        &self,
        page: &str,
        host: &dyn Host,
        it: &mut Interner,
        masks: Option<&crate::runtime::UiMasks>,
        store: Option<&mut crate::runtime::ElementCacheStore>,
    ) -> Result<Vec<Element>, ZypuiError> {
        let page = self
            .raw
            .pages
            .iter()
            .find(|p| p.name == page)
            .ok_or_else(|| ZypuiError {
                msg: format!("页 `{page}` 不存在"),
            })?;
        let mut out = Vec::new();
        let cache = match (masks, store) {
            (Some(m), Some(s)) => Some(CacheCtx {
                masks: m,
                store: s,
                parent_id: 0,
            }),
            _ => None,
        };
        let mut ctx = Ctx {
            host,
            cur_key: "",
            it,
            comps: &self.idx,
            raw: &self.raw,
            depth: 0,
            cache,
        };
        build_list(
            &page.root,
            &Scope::ROOT,
            &mut ctx,
            None,
            &mut None,
            1,
            &mut out,
        )?;
        Ok(out)
    }
}

/// children 槽位回填时需要的调用方现场：作用域、子树、当时的实例 key。
pub(crate) struct SlotFrame<'a> {
    scope: &'a Scope<'a>,
    children: &'a [Node],
    /// 调用方当时的实例 key（恒 `&'static`，同 `Ctx::cur_key`）。
    cur_key: &'static str,
}

/// 等第一个模板元素认领的实例级补丁（Use 的 key 与追加分类名）。
pub(crate) struct RootPatch<'a> {
    key: Option<&'static str>,
    classes: &'a [ClassTok],
    caller: &'a Scope<'a>,
}

/// 求值 + 分派的遍历上下文。
pub(crate) struct Ctx<'a> {
    pub(crate) host: &'a dyn Host,
    /// 当前实例 key（`hover`/`press` 基准）。只在 Use 边界更新。
    pub(crate) cur_key: &'static str,
    pub(crate) it: &'a mut Interner,
    pub(crate) comps: &'a HashMap<&'static str, usize>,
    pub(crate) raw: &'a RawUi,
    /// 组件嵌套深度（防引用环打穿栈）。
    pub(crate) depth: u32,
    /// P1 per-element cache（可选）；`None` = 全量 oracle 路径。
    pub(crate) cache: Option<CacheCtx<'a>>,
}

pub(crate) struct CacheCtx<'a> {
    pub(crate) masks: &'a crate::runtime::UiMasks,
    pub(crate) store: &'a mut crate::runtime::ElementCacheStore,
    pub(crate) parent_id: u64,
}

impl<'a> Ctx<'a> {
    /// 给 eval 用的窄视图（借用拆分：`it` 独占，其余只读）。
    fn ev(&mut self) -> crate::eval::Ctx<'_> {
        crate::eval::Ctx {
            host: self.host,
            cur_key: self.cur_key,
            it: self.it,
        }
    }
}

pub(crate) fn build_list(
    nodes: &[Node],
    scope: &Scope,
    ctx: &mut Ctx,
    slot: Option<&SlotFrame>,
    root_patch: &mut Option<RootPatch>,
    site_scope: u64,
    out: &mut Vec<Element>,
) -> Result<(), ZypuiError> {
    for (i, n) in nodes.iter().enumerate() {
        let child_site = crate::runtime::mix_site(site_scope, i as u64, node_kind_tag(n));
        build_node(n, scope, ctx, slot, root_patch, child_site, out)?;
    }
    Ok(())
}

pub(crate) fn build_node(
    node: &Node,
    scope: &Scope,
    ctx: &mut Ctx,
    slot: Option<&SlotFrame>,
    root_patch: &mut Option<RootPatch>,
    site_scope: u64,
    out: &mut Vec<Element>,
) -> Result<(), ZypuiError> {
    match node {
        Node::El(el) => {
            let mut el_el = Element::new(el.tag);

            // 1. 元素 key：实例补丁优先（Use 的 key 是实例身份），其次自己的。
            let mut own_key: Option<&'static str> = None;
            if let Some(patch) = root_patch.take() {
                if let Some(k) = patch.key {
                    own_key = Some(k);
                }
            }
            if own_key.is_none() {
                if let Some(e) = &el.key {
                    own_key = eval(e, scope, &mut ctx.ev()).as_str();
                }
            }
            if let Some(k) = own_key {
                el_el = el_el.with_key(ElementKey::Name(k));
            }

            // 2. 分类名：静态词 + 条件词 + 动态词（外观的名字，不是值）。
            for tok in &el.classes {
                match tok {
                    ClassTok::Lit(s) => el_el = el_el.style(s),
                    ClassTok::If { cond, tok } => {
                        if eval(cond, scope, &mut ctx.ev()).truthy() {
                            el_el = el_el.style(tok);
                        }
                    }
                    ClassTok::Dyn(e) => {
                        if let Some(s) = eval(e, scope, &mut ctx.ev()).as_str() {
                            // 空串不发（`extra=""`、三元空臂都落在这一路）——
                            // Rust 侧 `if !extra.is_empty()` 同语义。
                            if !s.is_empty() {
                                el_el = el_el.style(s);
                            }
                        }
                    }
                }
            }
            // 实例级追加分类名（Use 处写的 class），按**调用方**作用域求值。
            if let Some(patch) = root_patch {
                for tok in patch.classes {
                    match tok {
                        ClassTok::Lit(s) => el_el = el_el.style(s),
                        ClassTok::If { cond, tok } => {
                            if eval(cond, patch.caller, &mut ctx.ev()).truthy() {
                                el_el = el_el.style(tok);
                            }
                        }
                        ClassTok::Dyn(e) => {
                            if let Some(s) = eval(e, patch.caller, &mut ctx.ev()).as_str() {
                                if !s.is_empty() {
                                    el_el = el_el.style(s);
                                }
                            }
                        }
                    }
                }
            }

            // 3. 内容类属性（白名单见 ir::BARE_ATTRS；名字已在 zypuic 校验）。
            for (name, expr) in &el.attrs {
                let v = eval(expr, scope, &mut ctx.ev());
                el_el = match *name {
                    "text" => match v.as_str() {
                        Some(s) => el_el.with(&props::TEXT, PropertyValue::StaticStr(s)),
                        None => el_el,
                    },
                    "icon" => match v.as_str() {
                        Some(s) => el_el.with(&props::ICON_NAME, PropertyValue::StaticStr(s)),
                        None => el_el,
                    },
                    "texture" => match v.as_str() {
                        Some(s) => el_el.with(&props::IMAGE_TEXTURE, PropertyValue::StaticStr(s)),
                        None => el_el,
                    },
                    "collapsed" => el_el.with(&props::COLLAPSED, PropertyValue::bool(v.truthy())),
                    "interactive" => {
                        el_el.with(&props::INTERACTIVE, PropertyValue::bool(v.truthy()))
                    }
                    "focusable" => el_el.with(&props::FOCUSABLE, PropertyValue::bool(v.truthy())),
                    "activatable" => {
                        el_el.with(&props::ACTIVATABLE, PropertyValue::bool(v.truthy()))
                    }
                    // 禁用：core 的命中测试与 `ActivationState` 都认这个属性，
                    // 样式表则按它把控件画成灰的。
                    "disabled" => el_el.with(&props::DISABLED, PropertyValue::bool(v.truthy())),
                    "uv" => match uv_of(&v) {
                        Some(r) => el_el.with(&props::IMAGE_SRC, PropertyValue::UvRect(r)),
                        None => el_el,
                    },
                    "width" => match v.as_f32() {
                        Some(f) => el_el.with(&props::WIDTH, PropertyValue::float(f)),
                        None => el_el,
                    },
                    "height" => match v.as_f32() {
                        Some(f) => el_el.with(&props::HEIGHT, PropertyValue::float(f)),
                        None => el_el,
                    },
                    // 自定义数据键（hyperlink 的 url）：键由宿主给（地址即身份）。
                    "url" => match (v.as_str(), ctx.host.prop_key("url")) {
                        (Some(s), Some(k)) => el_el.with(k, PropertyValue::StaticStr(s)),
                        _ => el_el,
                    },
                    // 滤镜链（结构值，见 `Val::Raw`）：表达式语言没有它的字面量，
                    // 唯一来源是宿主绑定 —— 绑定给错类型时按「没设滤镜」降级
                    // （与 Rust 版「这帧不算滤镜」同形，不崩）。
                    "filter" => match v {
                        Val::Raw(pv) => el_el.with(&props::FILTER, pv),
                        _ => el_el,
                    },
                    // 混合模式（结构值，与 `filter` 同一条路）。
                    "blend" => match v {
                        Val::Raw(pv) => el_el.with(&props::BLEND_MODE, pv),
                        _ => el_el,
                    },
                    // 玻璃装饰（结构值，与 `filter` 同一条路 —— 表达式语言没有
                    // 它的字面量；绑定给错类型时按「不装饰」降级）。装饰是
                    // **运行期可变**的（按压动画改高光 / 内阴影的强度），
                    // 所以与 filter 一样走宿主绑定而不是 `.zstyle`。
                    "decoration" => match v {
                        Val::Raw(pv) => el_el.with(&props::DECORATIONS, pv),
                        _ => el_el,
                    },
                    // 逐帧颜色绑定（结构值，与 `decoration` 同一条路）：它不是
                    // 「外观不进模板」的后门 —— 表达式语言没有颜色字面量，
                    // 字符串/数字一律报错（见 `render-source` 的形状）。
                    // 静态颜色仍然住在 `.zstyle`。
                    "background" => match v {
                        Val::Raw(pv) => el_el.with(&props::BACKGROUND, pv),
                        _ => {
                            return crate::err(format!(
                                "`background` 必须求值为结构值（宿主逐帧绑定的颜色），实际为 {}",
                                v.type_name()
                            ));
                        }
                    },
                    // 绘制时位移 / 缩放（动态通道：动画每帧由绑定写值，
                    // 与布局属性不同、只重绘不重排）。
                    // 绑定给错类型时与 width/height 同形：按"不设"降级
                    // （core 的默认值即不位移 / 不缩放）。
                    "translate-x" => match v.as_f32() {
                        Some(f) => el_el.with(&props::TRANSLATE_X, PropertyValue::float(f)),
                        None => el_el,
                    },
                    "translate-y" => match v.as_f32() {
                        Some(f) => el_el.with(&props::TRANSLATE_Y, PropertyValue::float(f)),
                        None => el_el,
                    },
                    "scale-x" => match v.as_f32() {
                        Some(f) => el_el.with(&props::SCALE_X, PropertyValue::float(f)),
                        None => el_el,
                    },
                    "scale-y" => match v.as_f32() {
                        Some(f) => el_el.with(&props::SCALE_Y, PropertyValue::float(f)),
                        None => el_el,
                    },
                    // 溢出处理：表达式语言没有枚举字面量，按**字符串**传
                    // （`"scroll"` / `"hidden"` / `"auto"` / `"visible"`）。
                    // 认不出来的字串按「不裁剪」降级 —— 与 `filter` 的降级同形。
                    "overflow" => {
                        match v.as_str() {
                            Some("visible") => el_el
                                .with(&props::OVERFLOW, PropertyValue::overflow(Overflow::Visible)),
                            Some("hidden") => el_el
                                .with(&props::OVERFLOW, PropertyValue::overflow(Overflow::Hidden)),
                            Some("scroll") => el_el
                                .with(&props::OVERFLOW, PropertyValue::overflow(Overflow::Scroll)),
                            Some("auto") => el_el
                                .with(&props::OVERFLOW, PropertyValue::overflow(Overflow::Auto)),
                            _ => el_el,
                        }
                    }
                    // 滚动偏移（内容相对视口的偏移，DIP）。只重绘。
                    "scroll-offset-x" => match v.as_f32() {
                        Some(f) => el_el.with(&props::SCROLL_OFFSET_X, PropertyValue::float(f)),
                        None => el_el,
                    },
                    "scroll-offset-y" => match v.as_f32() {
                        Some(f) => el_el.with(&props::SCROLL_OFFSET_Y, PropertyValue::float(f)),
                        None => el_el,
                    },
                    "render-source" => match v.as_str() {
                        Some("") => el_el,
                        Some(kind) => {
                            let Some(source) = ctx.host.render_source(kind) else {
                                return crate::err(format!(
                                    "宿主没有注册 `.zypui` 渲染源 kind `{kind}`"
                                ));
                            };
                            if source.kind != kind {
                                return crate::err(format!(
                                    "宿主为渲染源 `{kind}` 返回了不同 kind `{}`",
                                    source.kind
                                ));
                            }
                            el_el.with(&props::RENDER_SOURCE, PropertyValue::custom(source))
                        }
                        None => {
                            return crate::err(format!(
                                "`render-source` 必须求值为字符串，实际为 {}",
                                v.type_name()
                            ));
                        }
                    },
                    // 编译期白名单拦过；到这里说明 IR 被绕过 zypuic 直构。
                    other => {
                        return crate::err(format!("未知属性名 `{other}`（应被 zypuic 拦下）"))
                    }
                };
            }

            // 4. 子树（槽位帧与实例补丁原样传下去，直到被消费）。
            if !el.children.is_empty() {
                let mut kids = Vec::with_capacity(el.children.len());
                build_list(
                    &el.children,
                    scope,
                    ctx,
                    slot,
                    root_patch,
                    site_scope,
                    &mut kids,
                )?;
                el_el = el_el.with_children(kids);
            }

            out.push(el_el);
        }

        Node::If(node) => {
            if eval(&node.cond, scope, &mut ctx.ev()).truthy() {
                let s = crate::runtime::mix_site(site_scope, 0, 21);
                build_list(&node.then, scope, ctx, slot, root_patch, s, out)?;
            } else {
                let s = crate::runtime::mix_site(site_scope, 1, 22);
                build_list(&node.els, scope, ctx, slot, root_patch, s, out)?;
            }
        }

        Node::For(node) => {
            let list = eval(&node.list, scope, &mut ctx.ev());
            let items = match list {
                Val::L(items) => items,
                _ => Vec::new(),
            };
            let for_base = crate::runtime::mix_site(site_scope, 0, 30);
            let body_mask = if ctx.cache.is_some() {
                Some(crate::runtime::mask_nodes(&node.body, ctx.comps, ctx.raw))
            } else {
                None
            };
            let can_cache_for = body_mask.as_ref().is_some_and(|m| {
                m.can_cache()
                    && !m.dynamic_interaction
                    && (!m.uses_tr || ctx.host.state_revision().is_some())
                    && !m.uses_render_source_attr
            });
            for (i, item) in items.into_iter().enumerate() {
                let mut vars: Vec<(&'static str, Val)> = Vec::with_capacity(2);
                vars.push((node.var, item));
                if let Some(ix) = node.index {
                    vars.push((ix, Val::F(i as f32)));
                }
                let inner = scope.push(&vars);
                let iter_scope = crate::runtime::mix_site(for_base, i as u64, 31);
                let variant = if let Some(tk) = &node.track_key {
                    let kv = eval(tk, &inner, &mut ctx.ev());
                    let ks = crate::runtime::val_to_cache_key(kv, ctx.it);
                    crate::runtime::CacheVariant::Key(ks)
                } else {
                    crate::runtime::CacheVariant::Index(i)
                };
                if can_cache_for {
                    let host = ctx.host;
                    let cur_key = ctx.cur_key;
                    let snap = crate::runtime::take_snapshot(
                        body_mask.as_ref().unwrap(),
                        host,
                        cur_key,
                        vars.clone(),
                    );
                    let id = crate::runtime::CacheId {
                        parent: ctx.cache.as_ref().unwrap().parent_id,
                        site: crate::runtime::SiteId(for_base),
                        variant,
                    };
                    if let Some(hit) = ctx.cache.as_mut().unwrap().store.get(&id, &snap) {
                        for e in hit {
                            out.push(e.as_ref().clone());
                        }
                        continue;
                    }
                    let saved_parent = ctx.cache.as_ref().unwrap().parent_id;
                    ctx.cache.as_mut().unwrap().parent_id = cache_id_hash(&id);
                    let mut built = Vec::new();
                    build_list(
                        &node.body, &inner, ctx, slot, root_patch, iter_scope, &mut built,
                    )?;
                    ctx.cache.as_mut().unwrap().parent_id = saved_parent;
                    let arcs: Vec<Arc<Element>> = built.iter().cloned().map(Arc::new).collect();
                    ctx.cache.as_mut().unwrap().store.insert(id, snap, arcs);
                    out.extend(built);
                    continue;
                }
                build_list(&node.body, &inner, ctx, slot, root_patch, iter_scope, out)?;
            }
        }

        Node::Use(u) => {
            // 实参按**调用方**作用域求值（表达式写在哪就在哪求值）。
            let mut arg_vars: Vec<(&'static str, Val)> = Vec::with_capacity(u.args.len());
            for (name, expr) in &u.args {
                let v = eval(expr, scope, &mut ctx.ev());
                arg_vars.push((name, v));
            }
            let inner = scope.push(&arg_vars);

            // 实例 key：子树的 hover/press 基准，也是根元素的身份。
            let saved = ctx.cur_key;
            let inst_key = match &u.key {
                Some(e) => eval(e, scope, &mut ctx.ev()).as_str(),
                None => None,
            };
            let comp_cur_key = inst_key.unwrap_or(saved);

            let ci = *ctx.comps.get(u.comp).ok_or_else(|| ZypuiError {
                msg: format!("组件 `{}` 不存在", u.comp),
            })?;
            let comp = &ctx.raw.components[ci];

            // 防引用环：模板嵌套深度有界（zypuic 也查，这里是运行时防线）。
            ctx.depth += 1;
            if ctx.depth > 32 {
                ctx.cur_key = saved;
                return crate::err(format!("组件 `{}` 展开超过 32 层（引用环？）", u.comp));
            }

            // 模板没写 <children/> 槽位却传了子树 → 报错（现在查，别等画面静默吞子树）。
            let comp_has_slot = count_slots(&comp.body) > 0;
            if !u.children.is_empty() && !comp_has_slot {
                ctx.depth -= 1;
                return crate::err(format!(
                    "组件 `{}` 没有 <children/> 槽位却传了 {} 个子节点",
                    u.comp,
                    u.children.len()
                ));
            }

            // P1 cache: 仅缓存无 slot 回填的 Use 边界（slot 语义并入调用方）。
            let mut cache_hit: Option<Vec<Arc<Element>>> = None;
            let mut cache_key: Option<(crate::runtime::CacheId, crate::runtime::DepSnapshot)> =
                None;
            let has_dynamic_class_extra =
                u.class_extra.iter().any(|t| !matches!(t, ClassTok::Lit(_)));
            let host_has_state_rev = ctx.host.state_revision().is_some();
            let comp_mask_ok = ctx
                .cache
                .as_ref()
                .and_then(|c| c.masks.comp_masks.get(ci))
                .is_some_and(|m| {
                    m.can_cache()
                        && !m.dynamic_interaction
                        && (!m.uses_tr || host_has_state_rev)
                        && !m.uses_render_source_attr
                });
            let can_try_cache = ctx.cache.is_some()
                && u.children.is_empty()
                && !comp_has_slot
                && !has_dynamic_class_extra
                && comp_mask_ok;
            if can_try_cache {
                let host = ctx.host;
                let (parent_id, variant, snapshot) = {
                    let c = ctx.cache.as_ref().unwrap();
                    let variant = match inst_key {
                        Some(k) => crate::runtime::CacheVariant::Key(k),
                        None => crate::runtime::CacheVariant::Single,
                    };
                    let parent_id = c.parent_id;
                    let mask = c.masks.comp_masks.get(ci);
                    let snap = if let Some(m) = mask {
                        crate::runtime::take_snapshot(m, host, comp_cur_key, arg_vars.clone())
                    } else {
                        crate::runtime::DepSnapshot {
                            host_scope: host.cache_scope(),
                            state_rev: None,
                            bindings: Vec::new(),
                            hover: false,
                            press: false,
                            focused: false,
                            cur_key: comp_cur_key,
                            args: arg_vars.clone(),
                        }
                    };
                    (parent_id, variant, snap)
                };
                let id = crate::runtime::CacheId {
                    parent: parent_id,
                    site: crate::runtime::SiteId(site_scope),
                    variant,
                };
                if let Some(hit) = ctx.cache.as_mut().unwrap().store.get(&id, &snapshot) {
                    cache_hit = Some(hit.to_vec());
                } else {
                    cache_key = Some((id, snapshot));
                }
            }

            if let Some(hit) = cache_hit {
                ctx.depth -= 1;
                for e in hit {
                    out.push((*e).clone());
                }
            } else {
                let saved_parent = ctx.cache.as_ref().map(|c| c.parent_id);
                if let (Some((id, _)), Some(c)) = (&cache_key, ctx.cache.as_mut()) {
                    c.parent_id = cache_id_hash(id);
                }
                if let Some(k) = inst_key {
                    ctx.cur_key = k;
                }
                let patch = RootPatch {
                    key: inst_key,
                    classes: &u.class_extra,
                    caller: scope,
                };
                let frame = SlotFrame {
                    scope,
                    children: &u.children,
                    cur_key: saved,
                };
                let mut built = Vec::new();
                let r = build_list(
                    &comp.body,
                    &inner,
                    ctx,
                    Some(&frame),
                    &mut Some(patch),
                    site_scope,
                    &mut built,
                );
                ctx.depth -= 1;
                ctx.cur_key = saved;
                if let Some(p) = saved_parent {
                    if let Some(c) = ctx.cache.as_mut() {
                        c.parent_id = p;
                    }
                }
                r?;

                if let Some((id, snapshot)) = cache_key {
                    let arcs: Vec<Arc<Element>> = built.iter().cloned().map(Arc::new).collect();
                    if let Some(c) = ctx.cache.as_mut() {
                        c.store.insert(id, snapshot, arcs);
                    }
                }
                out.append(&mut built);
            }
        }

        Node::Slot => match slot {
            Some(f) => {
                let saved = ctx.cur_key;
                ctx.cur_key = f.cur_key;
                let mut dummy = None;
                // slot 内容的 site 语义归属调用方；P1 下含 slot 的 Use 不独立缓存。
                let r = build_list(f.children, f.scope, ctx, None, &mut dummy, site_scope, out);
                ctx.cur_key = saved;
                r?;
            }
            None => return crate::err("children 槽位出现在组件模板之外"),
        },
    }
    Ok(())
}

/// 模板里 Slot 的个数（校验用；>1 时第二个 Slot 在实例化期无帧可用而报错，
/// zypuic 应在编译期就给出更准的报错）。
fn count_slots(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|n| match n {
            Node::Slot => 1,
            Node::El(el) => count_slots(&el.children),
            Node::If(f) => count_slots(&f.then) + count_slots(&f.els),
            Node::For(f) => count_slots(&f.body),
            Node::Use(_) => 0,
        })
        .sum()
}

/// 组件模板是否包含 `<children/>` 槽位（P1 缓存边界判定用）。
pub(crate) fn component_has_slot(comp: &crate::ir::Component) -> bool {
    count_slots(&comp.body) > 0
}

fn cache_id_hash(id: &crate::runtime::CacheId) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

fn node_kind_tag(n: &Node) -> u64 {
    match n {
        Node::El(_) => 1,
        Node::If(_) => 2,
        Node::For(_) => 3,
        Node::Use(_) => 4,
        Node::Slot => 5,
    }
}

/// `uv` 属性的取值：`"x,y,w,h"` 字面串或四元素列表。
fn uv_of(v: &Val) -> Option<zephyra_core::core::UvRect> {
    fn f(s: &str) -> Option<f32> {
        s.trim().parse().ok()
    }
    match v {
        Val::S(s) => {
            let mut it = s.split(',');
            let x = f(it.next()?)?;
            let y = f(it.next()?)?;
            let w = f(it.next()?)?;
            let h = f(it.next()?)?;
            Some(zephyra_core::core::UvRect::new(x, y, w, h))
        }
        Val::L(items) if items.len() == 4 => {
            let x = items[0].as_f32()?;
            let y = items[1].as_f32()?;
            let w = items[2].as_f32()?;
            let h = items[3].as_f32()?;
            Some(zephyra_core::core::UvRect::new(x, y, w, h))
        }
        _ => None,
    }
}

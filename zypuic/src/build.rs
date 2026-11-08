//! XML 树 → 校验后的 [`RawUi`]。
//!
//! # 校验清单（全部在这里，漏一条就是静默错误）
//!
//! - 顶层只认 `component` / `page`；
//! - 组件名必须 PascalCase（元素分派靠大小写区分「组件引用 vs 裸元素」）；
//! - 裸标签只认 `box` / `text` / `image` / `control` / `if` / `for` /
//!   `children` / `else`；
//! - 裸元素属性白名单（见 `zypui::ir::BARE_ATTRS` + `key`/`class`），
//!   未知属性报错，绝不静默忽略（与 zstylec 的 props_map 同一条规矩）；
//! - 组件实参名 ⊆ params；params 不得与保留字（key/class/hover/press）撞名；
//! - 表达式里的标识符必须是参数 / for 变量 / 内置词（hover/press）之一；
//! - 调用名只认 `tr`；
//! - `<children/>` 槽位只能在组件模板里、每个组件至多一个；
//! - 组件/页面重名报错；Use 传子节点而模板没有槽位报错。
//!
//! # 两遍构建
//!
//! 组件可以引用组件（Button 里用 Badge），且引用可能出现在定义之前 ——
//! 第一遍收全部组件的**签名**（名 + 参数表），第二遍才构建模板体与页面。
//! 引用环不在 build 查（运行期有深度防线），交给 `zypuic check` 的
//! 深度探测试跑 —— 便宜且不阻碍合法的相互引用场景。

use crate::expr_parse::parse_expr;
use crate::xml::{Diag, Level, XEl, XNode};
use zephyra_core::id::NodeTag;
use zypui::expr::{Expr, BUILTINS, CALLS};
use zypui::ir::BARE_ATTRS;
use zypui::ir::{ClassTok, Component, ElementNode, ForNode, IfNode, Node, Page, RawUi, UseNode};

/// 保留字：不能当组件参数名。
const RESERVED: &[&str] = &["key", "class", "hover", "press", "focused"];

/// 把全部文件的 XML 树构建成一个 [`RawUi`]。
///
/// 出错时返回**全部**诊断（一次看到所有错，而不是改一个跑一趟）。
pub fn build(files: &[(&'static str, Vec<XEl>)]) -> Result<RawUi, Vec<Diag>> {
    let mut diags: Vec<Diag> = Vec::new();

    // ---- 第一遍：签名 ----
    let mut signatures: Vec<(String, Vec<String>)> = Vec::new();
    for (file, roots) in files {
        for el in roots {
            match el.name.as_str() {
                "component" => {
                    let Some(name) = el.attr("name") else {
                        diags.push(Diag::error(file, el.line, el.col, "component 缺 name"));
                        continue;
                    };
                    if !is_pascal(name) {
                        diags.push(Diag::error(
                            file,
                            el.line,
                            el.col,
                            format!("组件名 `{name}` 必须 PascalCase（首字母大写）—— 元素分派靠它区分组件引用与裸元素"),
                        ));
                    }
                    let params = parse_params(
                        el.attr("params").unwrap_or(""),
                        file,
                        el.line,
                        el.col,
                        &mut diags,
                    );
                    signatures.push((name.to_string(), params));
                }
                "page" => {
                    if el.attr("name").is_none() {
                        diags.push(Diag::error(file, el.line, el.col, "page 缺 name"));
                    }
                }
                other => diags.push(Diag::error(
                    file,
                    el.line,
                    el.col,
                    format!("顶层只认 <component> / <page>，遇到 <{other}>"),
                )),
            }
        }
    }
    // 重名（组件与页面一起查：两类名字共用「实例化入口」的命名空间心智）。
    let mut seen: Vec<&str> = Vec::new();
    for (file, roots) in files {
        for el in roots {
            let Some(name) = el.attr("name") else {
                continue;
            };
            if seen.contains(&name) {
                diags.push(Diag::error(file, el.line, el.col, format!("`{name}` 重名")));
            } else {
                seen.push(Box::leak(name.to_string().into_boxed_str()));
            }
        }
    }

    // ---- 第二遍：构建 ----
    let mut components: Vec<Component> = Vec::new();
    let mut pages: Vec<Page> = Vec::new();
    for (file, roots) in files {
        for el in roots {
            match el.name.as_str() {
                "component" => {
                    let Some(name) = el.attr("name") else {
                        continue;
                    };
                    let params = parse_params(
                        el.attr("params").unwrap_or(""),
                        file,
                        el.line,
                        el.col,
                        &mut diags,
                    );
                    let (body, mut d) = build_nodes(&el.children, file, &params, true, &signatures);
                    diags.append(&mut d);
                    // <children/> 至多一个：多了「子树回填给谁」就没答案，
                    // 实例化期第二个槽位必然报错 —— 这里提前给更准的定位。
                    if count_slots(&body) > 1 {
                        diags.push(Diag::error(
                            file,
                            el.line,
                            el.col,
                            format!("组件 `{name}` 的模板里有多个 <children/> 槽位（至多一个）"),
                        ));
                    }
                    components.push(Component {
                        name: leak(name),
                        params: params.iter().map(|p| leak(p)).collect(),
                        body,
                    });
                }
                "page" => {
                    let Some(name) = el.attr("name") else {
                        continue;
                    };
                    let (root, mut d) = build_nodes(&el.children, file, &[], false, &signatures);
                    diags.append(&mut d);
                    pages.push(Page {
                        name: leak(name),
                        root,
                    });
                }
                _ => {}
            }
        }
    }

    if diags.iter().any(|d| d.level == Level::Error) {
        return Err(diags);
    }
    Ok(RawUi { components, pages })
}

/// 构建一组节点。`visible` 是当前作用域的已知标识符（组件参数），
/// `in_template` 决定 `<children/>` 槽位合不合法。
#[allow(clippy::too_many_arguments)]
fn build_nodes(
    xels: &[XNode],
    file: &'static str,
    visible: &[String],
    in_template: bool,
    signatures: &[(String, Vec<String>)],
) -> (Vec<Node>, Vec<Diag>) {
    let mut out = Vec::new();
    let mut diags = Vec::new();
    for x in xels {
        build_into(
            x,
            file,
            visible,
            in_template,
            signatures,
            &mut out,
            &mut diags,
        );
    }
    (out, diags)
}

fn build_into(
    x: &XNode,
    file: &'static str,
    visible: &[String],
    in_template: bool,
    signatures: &[(String, Vec<String>)],
    out: &mut Vec<Node>,
    diags: &mut Vec<Diag>,
) {
    let XNode::El(el) = x else {
        let (l, c, t) = match x {
            XNode::Text(t, l, c) => (*l, *c, t),
            _ => unreachable!(),
        };
        diags.push(Diag::error(
            file,
            l,
            c,
            format!(
                "这里出现文本 `{}`（文本内容只属于 <text> 元素，属性写法是 text=\"…\"）",
                t
            ),
        ));
        return;
    };
    match el.name.as_str() {
        "if" => build_if(el, file, visible, in_template, signatures, out, diags),
        "for" => build_for(el, file, visible, in_template, signatures, out, diags),
        "children" => {
            if !in_template {
                diags.push(Diag::error(
                    file,
                    el.line,
                    el.col,
                    "<children/> 槽位只能出现在组件模板里",
                ));
                return;
            }
            if !el.attrs.is_empty() {
                diags.push(Diag::error(file, el.line, el.col, "<children/> 不接受属性"));
            }
            out.push(Node::Slot);
        }
        "else" => {
            diags.push(Diag::error(
                file,
                el.line,
                el.col,
                "<else> 只能作为 <if> 的直接子元素出现",
            ));
        }
        "box" | "text" | "image" | "control" => {
            build_bare(el, file, visible, in_template, signatures, out, diags)
        }
        "container" => diags.push(Diag::error(
            file,
            el.line,
            el.col,
            "unknown element <container>; use <box> instead (clean-break migration)".to_string(),
        )),
        name if is_pascal(name) => build_use(el, name, file, visible, signatures, out, diags),
        name => diags.push(Diag::error(
            file,
            el.line,
            el.col,
            format!(
                "unknown element <{name}> (bare elements: box/text/image/control; component names are PascalCase)"
            ),
        )),
    }
}

fn build_if(
    el: &XEl,
    file: &'static str,
    visible: &[String],
    in_template: bool,
    signatures: &[(String, Vec<String>)],
    out: &mut Vec<Node>,
    diags: &mut Vec<Diag>,
) {
    let Some(cond_src) = el.attr("cond") else {
        diags.push(Diag::error(file, el.line, el.col, "<if> 缺 cond 属性"));
        return;
    };
    let cond = match parse_expr(cond_src, file, el.line, el.col + 6) {
        Ok(e) => {
            validate_expr(&e, visible, file, el.line, el.col, diags);
            e
        }
        Err(d) => {
            diags.push(d);
            return;
        }
    };
    // 直接子元素里挑 else（至多一个）。
    let (plain, elses): (Vec<&XNode>, Vec<&XEl>) =
        el.children
            .iter()
            .fold((Vec::new(), Vec::new()), |(mut p, mut e), n| match n {
                XNode::El(x) if x.name == "else" => {
                    e.push(x);
                    (p, e)
                }
                other => {
                    p.push(other);
                    (p, e)
                }
            });
    if elses.len() > 1 {
        diags.push(Diag::error(
            file,
            elses[1].line,
            elses[1].col,
            "<if> 只能有一个 <else>",
        ));
    }
    if elses.iter().any(|e| !e.attrs.is_empty()) {
        diags.push(Diag::error(
            file,
            elses[0].line,
            elses[0].col,
            "<else> 不接受属性",
        ));
    }
    let mut then = Vec::new();
    for x in &plain {
        build_into(x, file, visible, in_template, signatures, &mut then, diags);
    }
    let mut els = Vec::new();
    if let Some(e) = elses.first() {
        for x in &e.children {
            build_into(x, file, visible, in_template, signatures, &mut els, diags);
        }
    }
    out.push(Node::If(IfNode { cond, then, els }));
}

fn build_for(
    el: &XEl,
    file: &'static str,
    visible: &[String],
    in_template: bool,
    signatures: &[(String, Vec<String>)],
    out: &mut Vec<Node>,
    diags: &mut Vec<Diag>,
) {
    let Some(var) = el.attr("var") else {
        diags.push(Diag::error(file, el.line, el.col, "<for> 缺 var 属性"));
        return;
    };
    if !is_ident(var) {
        diags.push(Diag::error(
            file,
            el.line,
            el.col,
            format!("for 变量名 `{var}` 不是合法标识符"),
        ));
        return;
    }
    let index = el.attr("index");
    if let Some(ix) = index {
        if !is_ident(ix) {
            diags.push(Diag::error(
                file,
                el.line,
                el.col,
                format!("for 下标名 `{ix}` 不是合法标识符"),
            ));
            return;
        }
    }
    let Some(list_src) = el.attr("list") else {
        diags.push(Diag::error(file, el.line, el.col, "<for> 缺 list 属性"));
        return;
    };
    let list = match parse_expr(list_src, file, el.line, el.col + 6) {
        Ok(e) => {
            validate_expr(&e, visible, file, el.line, el.col, diags);
            e
        }
        Err(d) => {
            diags.push(d);
            return;
        }
    };
    // 体内可见：外层参数 + 本层变量。
    let mut inner: Vec<String> = visible.to_vec();
    inner.push(var.to_string());
    if let Some(ix) = index {
        inner.push(ix.to_string());
    }
    let (body, mut d) = build_nodes(&el.children, file, &inner, in_template, signatures);
    diags.append(&mut d);
    out.push(Node::For(ForNode {
        var: leak(var),
        index: index.map(leak),
        list,
        body,
    }));
}

#[allow(clippy::too_many_arguments)]
fn build_bare(
    el: &XEl,
    file: &'static str,
    visible: &[String],
    in_template: bool,
    signatures: &[(String, Vec<String>)],
    out: &mut Vec<Node>,
    diags: &mut Vec<Diag>,
) {
    let tag = match el.name.as_str() {
        "box" => NodeTag::Box,
        "text" => NodeTag::Text,
        "image" => NodeTag::Image,
        "control" => NodeTag::Control,
        other => {
            diags.push(Diag::error(
                file,
                el.line,
                el.col,
                format!("internal: unexpected bare element <{other}>"),
            ));
            NodeTag::Control
        }
    };
    let mut key: Option<Expr> = None;
    let mut classes: Vec<ClassTok> = Vec::new();
    let mut attrs: Vec<(&'static str, Expr)> = Vec::new();

    for (name, value, l, c) in &el.attrs {
        match name.as_str() {
            "key" => match attr_to_expr(value, file, *l, *c) {
                Ok(e) => {
                    validate_expr(&e, visible, file, *l, *c, diags);
                    key = Some(e);
                }
                Err(d) => diags.push(d),
            },
            "class" => match class_tokens(value, file, *l, *c) {
                Ok(toks) => {
                    for t in &toks {
                        validate_tok(t, visible, file, *l, *c, diags);
                    }
                    classes = toks;
                }
                Err(d) => diags.push(d),
            },
            other if BARE_ATTRS.contains(&other) => match attr_to_expr(value, file, *l, *c) {
                Ok(e) => {
                    validate_expr(&e, visible, file, *l, *c, diags);
                    attrs.push((leak(other), e));
                }
                Err(d) => diags.push(d),
            },
            other => diags.push(Diag::error(
                file,
                *l,
                *c,
                format!(
                    "<{}> 不认识属性 `{other}`（外观值归 .zstyle；可用：key/class/{}）",
                    el.name,
                    BARE_ATTRS.join("/")
                ),
            )),
        }
    }

    // `width`/`height` 是「内容几何」例外：只许写在 `fill:` 开头的 key 上
    // （滑条/进度条/滚动条的填充）。与 Rust 侧门禁
    // `only_fill_nodes_carry_local_width` 同一条规矩 —— 黑名单会烂，形状不会。
    if attrs.iter().any(|(n, _)| *n == "width" || *n == "height") {
        let fill_ok = matches!(&key, Some(Expr::LitS(k)) if k.starts_with("fill:"));
        if !fill_ok {
            diags.push(Diag::error(
                file,
                el.line,
                el.col,
                "width/height 只许写在 key 以 `fill:` 开头的节点上（内容几何例外，版式几何归 .zstyle）",
            ));
        }
    }

    // <text>字面内容</text> ≡ text 属性。两者同时给 = 歧义，报错。
    if !el.text.is_empty() {
        if el.name == "text" {
            if el.attrs.iter().any(|(n, ..)| n == "text") {
                diags.push(Diag::error(
                    file,
                    el.line,
                    el.col,
                    "文本内容与 text 属性同时给了，二选一",
                ));
            } else {
                attrs.push(("text", Expr::LitS(leak(&el.text))));
            }
        } else {
            diags.push(Diag::error(
                file,
                el.line,
                el.col,
                format!("<{}> 不接受文本内容（只有 <text> 元素可以）", el.name),
            ));
        }
    }

    let (children, mut d) = build_nodes(&el.children, file, visible, in_template, signatures);
    diags.append(&mut d);
    out.push(Node::El(ElementNode {
        tag,
        key,
        classes,
        attrs,
        children,
    }));
}

#[allow(clippy::too_many_arguments)]
fn build_use(
    el: &XEl,
    comp: &str,
    file: &'static str,
    visible: &[String],
    signatures: &[(String, Vec<String>)],
    out: &mut Vec<Node>,
    diags: &mut Vec<Diag>,
) {
    let Some((_, params)) = signatures.iter().find(|(n, _)| n == comp) else {
        diags.push(Diag::error(
            file,
            el.line,
            el.col,
            format!("组件 `{comp}` 不存在（名字写错了，或忘了在某个 .zypui 里定义）"),
        ));
        return;
    };
    let mut key: Option<Expr> = None;
    let mut class_extra: Vec<ClassTok> = Vec::new();
    let mut args: Vec<(&'static str, Expr)> = Vec::new();
    for (name, value, l, c) in &el.attrs {
        match name.as_str() {
            "key" => match attr_to_expr(value, file, *l, *c) {
                Ok(e) => {
                    validate_expr(&e, visible, file, *l, *c, diags);
                    key = Some(e);
                }
                Err(d) => diags.push(d),
            },
            "class" => match class_tokens(value, file, *l, *c) {
                Ok(toks) => {
                    for t in &toks {
                        validate_tok(t, visible, file, *l, *c, diags);
                    }
                    class_extra = toks;
                }
                Err(d) => diags.push(d),
            },
            other => {
                if !params.iter().any(|p| p == other) {
                    diags.push(Diag::error(
                        file,
                        *l,
                        *c,
                        format!(
                            "组件 `{comp}` 没有参数 `{other}`（有的：{}）",
                            params.join(" ")
                        ),
                    ));
                    continue;
                }
                match attr_to_expr(value, file, *l, *c) {
                    Ok(e) => {
                        validate_expr(&e, visible, file, *l, *c, diags);
                        args.push((leak(other), e));
                    }
                    Err(d) => diags.push(d),
                }
            }
        }
    }
    let (children, mut d) = build_nodes(&el.children, file, visible, false, signatures);
    diags.append(&mut d);
    out.push(Node::Use(UseNode {
        comp: leak(comp),
        key,
        class_extra,
        args,
        children,
    }));
}

/// 属性值 → 表达式。三种形状：纯字面量 / 整体 `{expr}` / 文本与 `{expr}` 混排。
fn attr_to_expr(v: &str, file: &'static str, line: u32, col: u32) -> Result<Expr, Diag> {
    if !v.contains('{') {
        return Ok(Expr::LitS(leak(v)));
    }
    let parts = split_interp(v, file, line, col)?;
    if parts.len() == 1 {
        return match &parts[0] {
            Part::Lit(s) => Ok(Expr::LitS(leak(s))),
            Part::Expr(src, off) => parse_expr(src, file, line, col + off),
        };
    }
    // 混排 → 拼接链。空段跳过。
    let mut chain: Option<Expr> = None;
    for p in &parts {
        let e = match p {
            Part::Lit(s) if s.is_empty() => continue,
            Part::Lit(s) => Expr::LitS(leak(s)),
            Part::Expr(src, off) => parse_expr(src, file, line, col + off)?,
        };
        chain = Some(match chain {
            None => e,
            Some(prev) => Expr::Bin(zypui::expr::BinOp::Add, Box::new(prev), Box::new(e)),
        });
    }
    Ok(chain.unwrap_or(Expr::LitS(leak(""))))
}

/// 分类名序列：按空白切词，`{…}` 段保持完整。
fn class_tokens(v: &str, file: &'static str, line: u32, col: u32) -> Result<Vec<ClassTok>, Diag> {
    let mut toks = Vec::new();
    for tok in split_ws_keep_braces(v) {
        if !tok.contains('{') {
            toks.push(ClassTok::Lit(leak(&tok)));
            continue;
        }
        let e = attr_to_expr(&tok, file, line, col)?;
        // `{hover ? 'hovered'}` → ClassTok::If（编译期可判的常见形状有专属表示，
        // 实例化就不用给整个表达式求值再切词）。
        toks.push(match &e {
            Expr::Cond(c, b, z) => match (&**b, &**z) {
                (Expr::LitS(s), Expr::LitS("")) => ClassTok::If {
                    cond: (**c).clone(),
                    tok: s,
                },
                (Expr::LitS(""), Expr::LitS(s)) => ClassTok::If {
                    cond: Expr::Not(Box::new((**c).clone())),
                    tok: s,
                },
                _ => ClassTok::Dyn(e),
            },
            _ => ClassTok::Dyn(e),
        });
    }
    if toks.is_empty() {
        return Err(Diag::error(file, line, col, "class 属性是空的"));
    }
    Ok(toks)
}

/// 分类名词里的表达式校验。
fn validate_tok(
    t: &ClassTok,
    visible: &[String],
    file: &'static str,
    l: u32,
    c: u32,
    diags: &mut Vec<Diag>,
) {
    match t {
        ClassTok::Lit(_) => {}
        ClassTok::If { cond, .. } => validate_expr(cond, visible, file, l, c, diags),
        ClassTok::Dyn(e) => validate_expr(e, visible, file, l, c, diags),
    }
}

/// 标识符校验：参数 / for 变量 / 内置词，或 `@绑定`（宿主的地盘，不查）。
fn validate_expr(
    e: &Expr,
    visible: &[String],
    file: &'static str,
    l: u32,
    c: u32,
    diags: &mut Vec<Diag>,
) {
    match e {
        Expr::Ref(name) => {
            if !visible.iter().any(|v| v == name) && !BUILTINS.contains(name) {
                diags.push(Diag::error(
                    file,
                    l,
                    c,
                    format!("未知标识符 `{name}`（可以是组件参数、for 变量或内置词 hover/press）"),
                ));
            }
        }
        Expr::Call(name, args) => {
            if !CALLS.contains(name) {
                diags.push(Diag::error(
                    file,
                    l,
                    c,
                    format!("未知调用 `{name}`（可用：{}）", CALLS.join("/")),
                ));
            }
            for a in args {
                validate_expr(a, visible, file, l, c, diags);
            }
        }
        Expr::Bin(_, a, b) => {
            validate_expr(a, visible, file, l, c, diags);
            validate_expr(b, visible, file, l, c, diags);
        }
        Expr::Not(a) | Expr::Neg(a) => validate_expr(a, visible, file, l, c, diags),
        Expr::Cond(a, b, d) => {
            validate_expr(a, visible, file, l, c, diags);
            validate_expr(b, visible, file, l, c, diags);
            validate_expr(d, visible, file, l, c, diags);
        }
        // 字段名本身不做静态校验（记录形状在宿主手里），底表达式照常递归。
        Expr::Field(base, _) => validate_expr(base, visible, file, l, c, diags),
        _ => {}
    }
}

/// `params="a b c"` → 参数名表。
fn parse_params(
    src: &str,
    file: &'static str,
    line: u32,
    col: u32,
    diags: &mut Vec<Diag>,
) -> Vec<String> {
    let mut out = Vec::new();
    for p in src.split_ascii_whitespace() {
        if !is_ident(p) {
            diags.push(Diag::error(
                file,
                line,
                col,
                format!("参数名 `{p}` 不是合法标识符"),
            ));
            continue;
        }
        if RESERVED.contains(&p) {
            diags.push(Diag::error(
                file,
                line,
                col,
                format!("参数名 `{p}` 是保留字（key/class 是实例通道，hover/press 是内置词）"),
            ));
            continue;
        }
        if BUILTINS.contains(&p) {
            diags.push(Diag::error(
                file,
                line,
                col,
                format!("参数名 `{p}` 与内置词撞名"),
            ));
            continue;
        }
        out.push(p.to_string());
    }
    out
}

fn is_pascal(s: &str) -> bool {
    let mut ch = s.chars();
    ch.next().is_some_and(|c| c.is_ascii_uppercase())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_ident(s: &str) -> bool {
    let mut ch = s.chars();
    ch.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 属性值的插值段。
enum Part {
    /// 字面段（原文）。
    Lit(String),
    /// `{expr}` 段：表达式文本 + 列偏移（报错定位）。
    Expr(String, u32),
}

/// 把属性值切成字面段与 `{…}` 段。扫 `{…}` 时跳过引号串（表达式里的
/// `'}'` 字面量不能提前断段）。
fn split_interp(v: &str, file: &'static str, line: u32, col: u32) -> Result<Vec<Part>, Diag> {
    let mut out = Vec::new();
    let mut lit = String::new();
    let b = v.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'{' {
            // 找配对 `}`，跳过引号串。
            let start = i + 1;
            let mut j = start;
            while j < b.len() && b[j] != b'}' {
                if b[j] == b'\'' || b[j] == b'"' {
                    let q = b[j];
                    j += 1;
                    while j < b.len() && b[j] != q {
                        if b[j] == b'\\' {
                            j += 1;
                        }
                        j += 1;
                    }
                }
                j += 1;
            }
            if j >= b.len() {
                return Err(Diag::error(file, line, col, "`{` 没有配对的 `}`"));
            }
            if !lit.is_empty() {
                out.push(Part::Lit(std::mem::take(&mut lit)));
            }
            out.push(Part::Expr(v[start..j].to_string(), (start + 1) as u32));
            i = j + 1;
        } else {
            // UTF-8 整字符前进。
            let mut end = i + 1;
            while end < b.len() && (b[end] & 0xC0) == 0x80 {
                end += 1;
            }
            lit.push_str(&v[i..end]);
            i = end;
        }
    }
    if !lit.is_empty() {
        out.push(Part::Lit(lit));
    }
    Ok(out)
}

/// 按空白切词，但 `{…}` 段算一个词的一部分。
fn split_ws_keep_braces(v: &str) -> Vec<String> {
    let mut toks = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    for c in v.chars() {
        match c {
            '{' => {
                depth += 1;
                cur.push(c);
            }
            '}' if depth > 0 => {
                depth -= 1;
                cur.push(c);
            }
            c if c.is_ascii_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    toks.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    toks
}

/// 模板里 `<children/>` 的个数。
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

/// 解析产物里的字符串泄漏成 `&'static`（加载期一次）。
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

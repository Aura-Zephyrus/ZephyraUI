//! [`RawUi`] → 二进制。`zypui::decode` 的镜像 —— **改一处必改另一处**，
//! 两边各带 round-trip 测试钉住（zypui 测 decode 自洽，这里测 encode 产物
//! decode 得回同一棵树）。
//!
//! 格式表见 `zypui::decode` 的模块文档。

use zephyra_core::id::NodeTag;
use zypui::decode::MAGIC;
use zypui::expr::{Expr, CALLS};
use zypui::ir::{ClassTok, Node, RawUi};

/// 编码整份工件。
pub fn encode(raw: &RawUi) -> Vec<u8> {
    let mut w = Writer::default();
    w.bytes(MAGIC);
    w.u32(raw.components.len() as u32);
    for c in &raw.components {
        w.str(c.name);
        w.u32(c.params.len() as u32);
        for p in &c.params {
            w.str(p);
        }
        w.u32(c.body.len() as u32);
        for n in &c.body {
            node(&mut w, n);
        }
    }
    w.u32(raw.pages.len() as u32);
    for p in &raw.pages {
        w.str(p.name);
        w.u32(p.root.len() as u32);
        for n in &p.root {
            node(&mut w, n);
        }
    }
    w.out
}

fn node(w: &mut Writer, n: &Node) {
    match n {
        Node::El(el) => {
            w.u8(0);
            w.u8(match el.tag {
                NodeTag::Text => 0,
                NodeTag::Image => 1,
                NodeTag::Box => 2,
                NodeTag::Control => 3,
            });
            opt_expr(w, &el.key);
            w.u32(el.classes.len() as u32);
            for t in &el.classes {
                classtok(w, t);
            }
            w.u32(el.attrs.len() as u32);
            for (name, e) in &el.attrs {
                w.str(name);
                expr(w, e);
            }
            w.u32(el.children.len() as u32);
            for c in &el.children {
                node(w, c);
            }
        }
        Node::If(f) => {
            w.u8(1);
            expr(w, &f.cond);
            w.u32(f.then.len() as u32);
            for c in &f.then {
                node(w, c);
            }
            w.u32(f.els.len() as u32);
            for c in &f.els {
                node(w, c);
            }
        }
        Node::For(f) => {
            w.u8(2);
            w.str(f.var);
            opt_str(w, f.index);
            expr(w, &f.list);
            w.u32(f.body.len() as u32);
            for c in &f.body {
                node(w, c);
            }
        }
        Node::Use(u) => {
            w.u8(3);
            w.str(u.comp);
            opt_expr(w, &u.key);
            w.u32(u.class_extra.len() as u32);
            for t in &u.class_extra {
                classtok(w, t);
            }
            w.u32(u.args.len() as u32);
            for (name, e) in &u.args {
                w.str(name);
                expr(w, e);
            }
            w.u32(u.children.len() as u32);
            for c in &u.children {
                node(w, c);
            }
        }
        Node::Slot => w.u8(4),
    }
}

fn classtok(w: &mut Writer, t: &ClassTok) {
    match t {
        ClassTok::Lit(s) => {
            w.u8(0);
            w.str(s);
        }
        ClassTok::If { cond, tok } => {
            w.u8(1);
            expr(w, cond);
            w.str(tok);
        }
        ClassTok::Dyn(e) => {
            w.u8(2);
            expr(w, e);
        }
    }
}

fn expr(w: &mut Writer, e: &Expr) {
    match e {
        Expr::LitS(s) => {
            w.u8(0);
            w.str(s);
        }
        Expr::LitF(f) => {
            w.u8(1);
            w.bytes(&f.to_le_bytes());
        }
        Expr::LitB(b) => {
            w.u8(2);
            w.u8(*b as u8);
        }
        Expr::Ref(s) => {
            w.u8(3);
            w.str(s);
        }
        Expr::Binding(s) => {
            w.u8(4);
            w.str(s);
        }
        Expr::Call(name, args) => {
            debug_assert!(CALLS.contains(name), "zypuic 校验过的调用名");
            w.u8(5);
            w.str(name);
            w.u32(args.len() as u32);
            for a in args {
                expr(w, a);
            }
        }
        Expr::Bin(op, l, r) => {
            w.u8(6);
            w.u8(match op {
                zypui::expr::BinOp::Add => 0,
                zypui::expr::BinOp::Sub => 1,
                zypui::expr::BinOp::Mul => 2,
                zypui::expr::BinOp::Div => 3,
                zypui::expr::BinOp::Eq => 4,
                zypui::expr::BinOp::Ne => 5,
                zypui::expr::BinOp::Lt => 6,
                zypui::expr::BinOp::Le => 7,
                zypui::expr::BinOp::Gt => 8,
                zypui::expr::BinOp::Ge => 9,
                zypui::expr::BinOp::And => 10,
                zypui::expr::BinOp::Or => 11,
                zypui::expr::BinOp::Mod => 12,
            });
            expr(w, l);
            expr(w, r);
        }
        Expr::Not(a) => {
            w.u8(7);
            expr(w, a);
        }
        Expr::Neg(a) => {
            w.u8(8);
            expr(w, a);
        }
        Expr::Cond(c, a, b) => {
            w.u8(9);
            expr(w, c);
            expr(w, a);
            expr(w, b);
        }
        Expr::Field(base, name) => {
            w.u8(10);
            expr(w, base);
            w.str(name);
        }
    }
}

fn opt_expr(w: &mut Writer, e: &Option<Expr>) {
    match e {
        Some(e) => {
            w.u8(1);
            expr(w, e);
        }
        None => w.u8(0),
    }
}

fn opt_str(w: &mut Writer, s: Option<&'static str>) {
    match s {
        Some(s) => {
            w.u8(1);
            w.str(s);
        }
        None => w.u8(0),
    }
}

#[derive(Default)]
struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn bytes(&mut self, b: &[u8]) {
        self.out.extend_from_slice(b);
    }

    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }

    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.bytes(s.as_bytes());
    }
}

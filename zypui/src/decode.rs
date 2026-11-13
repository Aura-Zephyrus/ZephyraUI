//! 二进制 → [`RawUi`]。零解析：只有长度前缀的反序列化。
//!
//! # 格式（与 `zypuic::encode` 一一对应，改一处必改另一处 + round-trip 测试）
//!
//! ```text
//! magic   "ZYP1"
//! u32     组件数 × (str 名; u32 参数数 × str; u32 模板节点数 × node)
//! u32     页数   × (str 名; u32 根节点数 × node)
//!
//! node    u8 kind: 0=El 1=If 2=For 3=Use 4=Slot
//!   El    u8 tag(0=Text 1=Image 2=Box 3=Control); opt(expr) key;
//!         u32 × classtok; u32 × (str 属性名, expr); u32 × node
//!   If    expr; u32 × node(then); u32 × node(else)
//!   For   str var; opt(str) index; expr list; u32 × node
//!   Use   str 组件名; opt(expr) key; u32 × classtok; u32 × (str 实参名, expr);
//!         u32 × node(调用方子树)
//!
//! classtok u8 kind: 0=Lit(str) 1=If(expr, str) 2=Dyn(expr)
//! expr    u8 kind: 0=LitS(str) 1=LitF(f32 LE) 2=LitB(u8) 3=Ref(str) 4=Binding(str)
//!                  5=Call(str, u32 × expr) 6=Bin(u8 op, expr, expr)
//!                  7=Not(expr) 8=Neg(expr) 9=Cond(expr, expr, expr)
//! str     u32 字节长 + UTF-8（内容 Box::leak —— 每次加载泄一次，不是每帧）
//! opt(x)  u8 有无(0/1) + x
//! ```
//!
//! # 损坏输入
//!
//! 全部读入都查边界，越界/非法标签/UTF-8 错误一律 `Err`，绝不 panic ——
//! `include_bytes!` 的产物理论上是自己编的，但「反序列化器对任意字节全函数」
//! 是便宜且一劳永逸的保险。

use crate::expr::{BinOp, Expr};
use crate::ir::{ClassTok, Component, ElementNode, ForNode, IfNode, Node, Page, RawUi, UseNode};
use crate::ZypuiError;
use zephyra_core::id::NodeTag;

/// 识别魔数（`b"ZYP1"`）。
pub const MAGIC: &[u8; 4] = b"ZYP1";

/// 反序列化。见模块文档的格式表。
pub fn decode(bytes: &[u8]) -> Result<RawUi, ZypuiError> {
    let mut r = Reader { b: bytes, p: 0 };
    r.expect(MAGIC)?;
    let n_comp = r.u32()?;
    let mut components = Vec::with_capacity(r.items(n_comp)?);
    for _ in 0..n_comp {
        let name = r.str()?;
        let n_params = r.u32()?;
        let mut params = Vec::with_capacity(r.items(n_params)?);
        for _ in 0..n_params {
            params.push(r.str()?);
        }
        let n_body = r.u32()?;
        let body = nodes(&mut r, n_body)?;
        components.push(Component { name, params, body });
    }
    let n_pages = r.u32()?;
    let mut pages = Vec::with_capacity(r.items(n_pages)?);
    for _ in 0..n_pages {
        let name = r.str()?;
        let n_root = r.u32()?;
        let root = nodes(&mut r, n_root)?;
        pages.push(Page { name, root });
    }
    Ok(RawUi { components, pages })
}

fn nodes(r: &mut Reader, n: u32) -> Result<Vec<Node>, ZypuiError> {
    let mut out = Vec::with_capacity(r.items(n)?);
    for _ in 0..n {
        out.push(node(r)?);
    }
    Ok(out)
}

fn node(r: &mut Reader) -> Result<Node, ZypuiError> {
    match r.u8()? {
        0 => {
            let tag = match r.u8()? {
                0 => NodeTag::Text,
                1 => NodeTag::Image,
                2 => NodeTag::Box,
                3 => NodeTag::Control,
                k => return crate::err(format!("非法元素标签 {k}")),
            };
            let key = opt_expr(r)?;
            let n_class = r.u32()?;
            let mut classes = Vec::with_capacity(n_class as usize);
            for _ in 0..n_class {
                classes.push(classtok(r)?);
            }
            let n_attr = r.u32()?;
            let mut attrs = Vec::with_capacity(n_attr as usize);
            for _ in 0..n_attr {
                let name = r.str()?;
                attrs.push((name, expr(r)?));
            }
            let n_child = r.u32()?;
            let children = nodes(r, n_child)?;
            Ok(Node::El(ElementNode {
                tag,
                key,
                classes,
                attrs,
                children,
            }))
        }
        1 => {
            let cond = expr(r)?;
            let n_then = r.u32()?;
            let then = nodes(r, n_then)?;
            let n_els = r.u32()?;
            let els = nodes(r, n_els)?;
            Ok(Node::If(IfNode { cond, then, els }))
        }
        2 => {
            let var = r.str()?;
            let index = opt_str(r)?;
            let track_key = opt_expr(r)?;
            let list = expr(r)?;
            let n_body = r.u32()?;
            let body = nodes(r, n_body)?;
            Ok(Node::For(ForNode {
                var,
                index,
                track_key,
                list,
                body,
            }))
        }
        3 => {
            let comp = r.str()?;
            let key = opt_expr(r)?;
            let n_class = r.u32()?;
            let mut class_extra = Vec::with_capacity(n_class as usize);
            for _ in 0..n_class {
                class_extra.push(classtok(r)?);
            }
            let n_arg = r.u32()?;
            let mut args = Vec::with_capacity(n_arg as usize);
            for _ in 0..n_arg {
                let name = r.str()?;
                args.push((name, expr(r)?));
            }
            let n_child = r.u32()?;
            let children = nodes(r, n_child)?;
            Ok(Node::Use(UseNode {
                comp,
                key,
                class_extra,
                args,
                children,
            }))
        }
        4 => Ok(Node::Slot),
        k => crate::err(format!("非法节点类型 {k}")),
    }
}

fn classtok(r: &mut Reader) -> Result<ClassTok, ZypuiError> {
    match r.u8()? {
        0 => Ok(ClassTok::Lit(r.str()?)),
        1 => {
            let cond = expr(r)?;
            let tok = r.str()?;
            Ok(ClassTok::If { cond, tok })
        }
        2 => Ok(ClassTok::Dyn(expr(r)?)),
        k => crate::err(format!("非法分类名词类型 {k}")),
    }
}

fn expr(r: &mut Reader) -> Result<Expr, ZypuiError> {
    match r.u8()? {
        0 => Ok(Expr::LitS(r.str()?)),
        1 => Ok(Expr::LitF(f32::from_le_bytes(
            r.bytes(4)?.try_into().unwrap(),
        ))),
        2 => match r.u8()? {
            0 => Ok(Expr::LitB(false)),
            1 => Ok(Expr::LitB(true)),
            k => crate::err(format!("非法布尔 {k}")),
        },
        3 => Ok(Expr::Ref(r.str()?)),
        4 => Ok(Expr::Binding(r.str()?)),
        5 => {
            let name = r.str()?;
            let n = r.u32()?;
            let mut args = Vec::with_capacity(r.items(n)?);
            for _ in 0..n {
                args.push(expr(r)?);
            }
            Ok(Expr::Call(name, args))
        }
        6 => {
            let op = match r.u8()? {
                0 => BinOp::Add,
                1 => BinOp::Sub,
                2 => BinOp::Mul,
                3 => BinOp::Div,
                4 => BinOp::Eq,
                5 => BinOp::Ne,
                6 => BinOp::Lt,
                7 => BinOp::Le,
                8 => BinOp::Gt,
                9 => BinOp::Ge,
                10 => BinOp::And,
                11 => BinOp::Or,
                12 => BinOp::Mod,
                k => return crate::err(format!("非法运算符 {k}")),
            };
            let l = expr(r)?;
            let rr = expr(r)?;
            Ok(Expr::Bin(op, Box::new(l), Box::new(rr)))
        }
        7 => Ok(Expr::Not(Box::new(expr(r)?))),
        8 => Ok(Expr::Neg(Box::new(expr(r)?))),
        9 => {
            let c = expr(r)?;
            let a = expr(r)?;
            let b = expr(r)?;
            Ok(Expr::Cond(Box::new(c), Box::new(a), Box::new(b)))
        }
        10 => {
            let base = expr(r)?;
            let name = r.str()?;
            Ok(Expr::Field(Box::new(base), name))
        }
        k => crate::err(format!("非法表达式类型 {k}")),
    }
}

fn opt_expr(r: &mut Reader) -> Result<Option<Expr>, ZypuiError> {
    if r.u8()? == 1 {
        Ok(Some(expr(r)?))
    } else {
        Ok(None)
    }
}

fn opt_str(r: &mut Reader) -> Result<Option<&'static str>, ZypuiError> {
    if r.u8()? == 1 {
        Ok(Some(r.str()?))
    } else {
        Ok(None)
    }
}

/// 边界检查的字节游标。所有读入经它，越界一律 `Err`。
struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn expect(&mut self, magic: &[u8; 4]) -> Result<(), ZypuiError> {
        if self.bytes(4)? != &magic[..] {
            return crate::err("不是 .zypui 二进制（魔数不符）");
        }
        Ok(())
    }

    fn u8(&mut self) -> Result<u8, ZypuiError> {
        Ok(self.bytes(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, ZypuiError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    /// 数量字段必须被剩余字节撑得起 —— 否则恶意/损坏的长度会让
    /// `with_capacity(40 亿)` 直接分配中止。「全函数」从字节数做起。
    fn items(&self, n: u32) -> Result<usize, ZypuiError> {
        let n = n as usize;
        if n > self.b.len() - self.p {
            return crate::err(format!("数量字段 {n} 超过剩余字节数"));
        }
        Ok(n)
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], ZypuiError> {
        let end = self.p.checked_add(n).ok_or(ZypuiError {
            msg: "长度溢出".into(),
        })?;
        if end > self.b.len() {
            return crate::err(format!(
                "二进制提前结束（要 {} 字节，剩 {}）",
                n,
                self.b.len() - self.p
            ));
        }
        let s = &self.b[self.p..end];
        self.p = end;
        Ok(s)
    }

    /// 读一个字符串并 `Box::leak` 成 `&'static`。每次加载泄一次（见 lib.rs）。
    fn str(&mut self) -> Result<&'static str, ZypuiError> {
        let n = self.u32()? as usize;
        let raw = self.bytes(n)?;
        let s = std::str::from_utf8(raw).map_err(|_| ZypuiError {
            msg: "字符串不是 UTF-8".into(),
        })?;
        Ok(Box::leak(s.to_string().into_boxed_str()))
    }
}

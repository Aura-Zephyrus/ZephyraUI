//! 表达式求值 —— 每帧实例化期跑的最后一环。
//!
//! # 作用域链
//!
//! 标识符按 [`Scope`] 链解析：最内层先（`for` 变量），组件实参在它的上层，
//! 链尾是内置词（[`BUILTINS`]）。`@name` 不走这条链，直接问宿主。
//!
//! # 为什么求值是全函数
//!
//! 每帧跑的代码不能 panic：跨类型比较判 false、缺绑定判假、数值除零得 0。
//! **静态能查的错（未知标识符、未知调用名）在 zypuic 编译期报**，运行期
//! 只剩动态那部分，全部给温和的降级值 —— 和样式表「加载时报错」的分工一致。

use crate::expr::{BinOp, Expr, CALLS};
use crate::intern::Interner;
use crate::val::Val;

/// 标识符作用域的一层。`parent` 串成链。
#[derive(Debug)]
pub struct Scope<'a> {
    /// 这层的 `(名字, 值)` —— 组件实参或 `for` 当前项/下标。
    pub vars: &'a [(&'static str, Val)],
    /// 外层；`None` 表示链尾。
    pub parent: Option<&'a Scope<'a>>,
}

impl<'a> Scope<'a> {
    /// 空作用域（页根层，无组件实参无 for 变量）。
    pub const ROOT: Scope<'static> = Scope {
        vars: &[],
        parent: None,
    };

    /// 推一层。
    pub fn push(&'a self, vars: &'a [(&'static str, Val)]) -> Scope<'a> {
        Scope {
            vars,
            parent: Some(self),
        }
    }

    /// 沿链找名字。
    pub fn lookup(&self, name: &str) -> Option<&Val> {
        if let Some((_, v)) = self.vars.iter().find(|(n, _)| *n == name) {
            return Some(v);
        }
        self.parent.and_then(|p| p.lookup(name))
    }
}

/// 求值环境：宿主 + 当前实例 key。
///
/// 当前实例 key（`hover`/`press` 的基准）由实例化器在进入带 key 的元素/
/// 组件实例时更新 —— 数据流与 Rust 控件「调用方传 hovered、构造器扇出」一致。
pub struct Ctx<'a> {
    /// 宿主（绑定 + 文案 + 输入状态）。
    pub host: &'a dyn crate::Host,
    /// 当前实例的 key（无 key 的顶层片段用空串 —— `hover` 恒假）。
    /// 恒为 `&'static`（来源只有求值产物），见 `instantiate::Ctx` 的说明。
    pub cur_key: &'static str,
    /// 动态串去重表（`+` 拼接的结果从这里 `&'static` 化）。
    pub it: &'a mut Interner,
}

/// 求值一个表达式。全函数，不 panic。
pub fn eval(expr: &Expr, scope: &Scope, ctx: &mut Ctx) -> Val {
    match expr {
        Expr::LitS(s) => Val::S(s),
        Expr::LitF(f) => Val::F(*f),
        Expr::LitB(b) => Val::B(*b),

        Expr::Ref(name) => {
            if let Some(v) = scope.lookup(name) {
                return v.clone();
            }
            match *name {
                // 内置词：当前实例 key 的输入状态。
                "hover" => Val::B(ctx.host.hovers(ctx.cur_key)),
                "press" => Val::B(ctx.host.holds(ctx.cur_key)),
                // 当前实例 key 本身（expander 家族的「可点区域 = 实例 key」）。
                "key" => Val::S(ctx.cur_key),
                // 声明了但**没传值**的组件实参落在这里：合法，缺省即假
                // （`<Button/>` 不带 icon，模板里的 `if icon` 就走空分支）。
                // 「标识符根本不存在」这类错由 zypuic 编译期拦，运行期不炸帧。
                _ => Val::B(false),
            }
        }

        Expr::Binding(name) => ctx.host.binding(name).unwrap_or(Val::B(false)),

        Expr::Call(name, args) => match *name {
            "tr" => {
                let key = args
                    .first()
                    .map(|a| eval(a, scope, ctx))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                Val::S(ctx.host.tr(key))
            }
            "min" | "max" => {
                let a = args
                    .first()
                    .map(|a| eval(a, scope, ctx))
                    .unwrap_or(Val::B(false));
                let b = args
                    .get(1)
                    .map(|a| eval(a, scope, ctx))
                    .unwrap_or(Val::B(false));
                match (a.as_f32(), b.as_f32()) {
                    (Some(a), Some(b)) => Val::F(if *name == "min" { a.min(b) } else { a.max(b) }),
                    _ => Val::B(false),
                }
            }
            "range" => {
                let n = args
                    .first()
                    .map(|a| eval(a, scope, ctx))
                    .and_then(|v| v.as_f32())
                    .unwrap_or(0.0);
                let n = n.max(0.0) as usize;
                Val::L((0..n).map(|i| Val::F(i as f32)).collect())
            }
            "at" => {
                let list = args.first().map(|a| eval(a, scope, ctx));
                let i = args
                    .get(1)
                    .map(|a| eval(a, scope, ctx))
                    .and_then(|v| v.as_f32());
                match (list, i) {
                    (Some(Val::L(items)), Some(i)) => items
                        .get(i.max(0.0) as usize)
                        .cloned()
                        .unwrap_or(Val::B(false)),
                    _ => Val::B(false),
                }
            }
            "hover" | "press" => {
                let key = args
                    .first()
                    .map(|a| eval(a, scope, ctx))
                    .and_then(|v| v.as_str())
                    .unwrap_or(ctx.cur_key);
                if *name == "hover" {
                    Val::B(ctx.host.hovers(key))
                } else {
                    Val::B(ctx.host.holds(key))
                }
            }
            "str" => {
                let v = args
                    .first()
                    .map(|a| eval(a, scope, ctx))
                    .unwrap_or(Val::B(false));
                match v {
                    Val::F(f) => Val::S(ctx.it.intern(&fmt_f32(f))),
                    Val::S(s) => Val::S(s),
                    _ => Val::B(false),
                }
            }
            other => {
                debug_assert!(
                    CALLS.contains(&other),
                    "未知调用 `{other}` 应被 zypuic 拦下"
                );
                Val::B(false)
            }
        },
        Expr::Bin(op, l, r) => bin(*op, eval(l, scope, ctx), eval(r, scope, ctx), ctx),
        Expr::Not(x) => Val::B(!eval(x, scope, ctx).truthy()),
        Expr::Neg(x) => Val::F(-eval(x, scope, ctx).as_f32().unwrap_or(0.0)),
        Expr::Cond(c, a, b) => {
            if eval(c, scope, ctx).truthy() {
                eval(a, scope, ctx)
            } else {
                eval(b, scope, ctx)
            }
        }
        Expr::Field(base, name) => {
            let v = eval(base, scope, ctx);
            // 记录外/无此字段都降级为假 —— 数据形状错了在画面上可见，不炸帧。
            v.field(name).cloned().unwrap_or(Val::B(false))
        }
    }
}

/// 二元运算。`+` 对两串是拼接（结果经去重表 `&'static` 化）。
fn bin(op: BinOp, l: Val, r: Val, ctx: &mut Ctx) -> Val {
    use BinOp::*;
    match op {
        And => Val::B(l.truthy() && r.truthy()),
        Or => Val::B(l.truthy() || r.truthy()),
        Eq => Val::B(eq(&l, &r)),
        Ne => Val::B(!eq(&l, &r)),
        Add => match (&l, &r) {
            (Val::S(a), Val::S(b)) => {
                if b.is_empty() {
                    Val::S(a)
                } else if a.is_empty() {
                    Val::S(b)
                } else {
                    let mut s = String::with_capacity(a.len() + b.len());
                    s.push_str(a);
                    s.push_str(b);
                    Val::S(ctx.it.intern(&s))
                }
            }
            (Val::F(a), Val::F(b)) => Val::F(a + b),
            // 跨类型加法没有意义；降级成左值，让错误在画面上可见而不是炸帧。
            _ => l,
        },
        Sub | Mul | Div => match (&l, &r) {
            (Val::F(a), Val::F(b)) => {
                let v = match op {
                    Sub => a - b,
                    Mul => a * b,
                    // 除零得 0：全函数优先（见模块文档）。
                    _ => {
                        if *b == 0.0 {
                            0.0
                        } else {
                            a / b
                        }
                    }
                };
                Val::F(v)
            }
            _ => l,
        },
        Mod => match (&l, &r) {
            (Val::F(a), Val::F(b)) => {
                if *b == 0.0 {
                    Val::F(0.0)
                } else {
                    Val::F(a % *b)
                }
            }
            _ => l,
        },
        Lt | Le | Gt | Ge => match (&l, &r) {
            (Val::F(a), Val::F(b)) => Val::B(match op {
                Lt => a < b,
                Le => a <= b,
                Gt => a > b,
                _ => a >= b,
            }),
            _ => Val::B(false),
        },
    }
}

/// 同类型相等；跨类型恒 false（错误在画面上可见，不炸帧）。
fn eq(l: &Val, r: &Val) -> bool {
    match (l, r) {
        (Val::S(a), Val::S(b)) => a == b,
        (Val::F(a), Val::F(b)) => a == b,
        (Val::B(a), Val::B(b)) => a == b,
        _ => false,
    }
}

/// 数值 → 十进制串：整数值不带小数点（`3.0` → `"3"`），其余保留到足够精度。
/// `str()` 专为动态 key / 数字文案服务 —— 那些场景全是整数。
fn fmt_f32(f: f32) -> String {
    if f.fract() == 0.0 && f.abs() < 1e9 {
        format!("{}", f as i64)
    } else {
        format!("{f}")
    }
}

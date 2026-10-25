//! 求值结果的值类型。
//!
//! # 字符串只有 `&'static str` 一个形状
//!
//! 字面量来自解码/解析（本就是 `&'static`）；`tr()` 的返回值由宿主的文案表给
//! （编译期常量）；表达式拼接出的动态串经 [`crate::Interner`] 去重后 `&'static`。
//! 没有 `String` 变体，就没有「每帧一次泄漏」这类错误能藏的地方。

/// 一个求值结果。
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    /// 字符串（见模块文档：恒为 `&'static`）。
    S(&'static str),
    /// 数值（f32，与属性系统的 `Float` 对齐）。
    F(f32),
    /// 布尔。
    B(bool),
    /// 列表（`for` 的数据源；元素通常是 `S`）。
    L(Vec<Val>),
    /// 记录（命名字段集）。数据驱动控件的模型项：`(id, label)` 行、
    /// 日历格……由宿主经 [`crate::Host::binding`] 提供，**只出现在运行期**
    /// （表达式只访问字段，不构造记录 —— 二进制格式里没有它）。
    Rec(Vec<(&'static str, Val)>),
    /// 透传的属性值（如滤镜链这类**结构值**）。表达式语言没有它的字面量，
    /// 唯一来源是宿主绑定；用途只有一个——喂给对应的属性分支（`filter`），
    /// 在其它上下文（条件、class、text）里一律按「假 / 空串」降级。
    Raw(zephyra_core::property::PropertyValue),
}

impl Val {
    /// 按字段名取记录字段。非记录或无此字段返回 `None`。
    pub fn field(&self, name: &str) -> Option<&Val> {
        match self {
            Val::Rec(fields) => fields.iter().find(|(n, _)| *n == name).map(|(_, v)| v),
            _ => None,
        }
    }

    /// 条件真值：`B` 即本身；非空串为真；非零为真；列表非空为真。
    ///
    /// 只给 `if` / `?:` / `&&` 这几个分流点用，不做隐式转换到别处。
    pub fn truthy(&self) -> bool {
        match self {
            Val::S(s) => !s.is_empty(),
            Val::F(f) => *f != 0.0,
            Val::B(b) => *b,
            Val::L(l) => !l.is_empty(),
            Val::Rec(_) => true,
            // 结构值没有真值语义 —— 出现在条件里就是绑错了名字，按假降级。
            Val::Raw(_) => false,
        }
    }

    /// 按字符串取用；非串返回 `None`（调用方决定报错还是降级）。
    pub fn as_str(&self) -> Option<&'static str> {
        match self {
            Val::S(s) => Some(s),
            _ => None,
        }
    }

    /// 按数值取用。
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Val::F(f) => Some(*f),
            _ => None,
        }
    }

    /// 调试/错误信息里的类型名。
    pub fn type_name(&self) -> &'static str {
        match self {
            Val::S(_) => "string",
            Val::F(_) => "number",
            Val::B(_) => "bool",
            Val::L(_) => "list",
            Val::Rec(_) => "record",
            Val::Raw(_) => "raw",
        }
    }
}

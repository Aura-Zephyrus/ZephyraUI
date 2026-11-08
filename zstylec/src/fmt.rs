//! `.zstyle` 排版器：把源码排成标准形式。
//!
//! # 为什么是「只动空白」而不是从 IR 回写
//!
//! [`crate::parse`] 把值**归一化**了：`10px` → `Float(10.0)`、`#rrggbb` → `Color`、
//! `@令牌` → `Resource("令牌")`。从 IR 反写 CSS 不可能还原源码拼写 —— 单位被抹掉，
//! `0.00` 与 `0` 无从区分，`26 18 0 1.5` 这种数值列表的形状也丢了。于是排版器
//! **直接吃原文**：选择器、属性名、值、注释**逐字照搬**，只重排它们之间的空白
//! （缩进 / 空行 / `{` 与 `;` 的换行 / 属性名与值之间的空格）。
//!
//! 「不改语义」因此是**构造上成立**的，不靠测试保证（测试只是复核）。
//!
//! # 形态
//!
//! ```text
//! /* 注释照抄，续行的缩进也照抄 */
//! Control.button.accent {
//!     background: @accent_fill;
//! }
//!
//! Control.button.hovered {
//!     border-color: @control_border_strong;
//! }
//! ```
//!
//! | 规则 | 为什么 |
//! |---|---|
//! | 选择器与 `{` 之间一个空格，`{` 在行尾 | `A    {` 那种对齐空格是编辑器对齐习惯，diff 里全是噪音 |
//! | 一条声明一行，缩进 4 空格 | 与仓内存量一致（实测全部缩进行都是 4） |
//! | `}` 独占一行 | 同上 |
//! | 顶层条目之间：**有空行就留一个，没有就不加** | 注释"贴"着它注释的那条规则（`*/` 与选择器之间无空行）与"分节注释 + 空行"两种都是存量写法 |
//! | 注释逐字照抄（只重排首行缩进） | 续行的 ` *` 是作者自己对齐的，工具不猜 |
//! | 空行折叠成一个 | 编辑器折叠/手抖会留下连续空行 |
//! | 换行统一 LF、文末单个换行 | CRLF 只在两份示例文件里出现（历史遗留），统一掉 |
//!
//! # 不认识的输入返回 `None`，而不是猜
//!
//! 注释没闭合、`}` 多出来、声明没有 `:`、`{` 后面没有闭合 —— 这些都是「源码
//! 已经不是这个排版器认识的形状」。这时**原样返回**（调用方保留原件），绝不拿
//! 一份猜出来的排版覆盖上去：一个只会排版空白的工具，比原样保留更危险。

/// 排版。`None` = 输入不是本排版器认识的结构（见模块文档）。
///
/// # 为什么不叫 `zstylec::format`
///
/// 那会跟 std 的 `format!` 宏在同一个作用域撞名，`cargo doc` 直接报
/// 「`format` is both a function and a macro」并让**整个 crate** 文档不出来。
/// 所以库入口把它导出成 [`crate::format_style`]（见 `lib.rs`）。
pub fn format(src: &str) -> Option<String> {
    // 换行先统一成 LF：CRLF 文件里的注释原文若带着 `\r`，与 `\n` 拼接后会是
    // 混合换行。只换行符，别的一个字节不动。
    let normalized: String = src
        .chars()
        .map(|c| if c == '\r' { '\n' } else { c })
        .collect();
    let toks = lex(&normalized)?;
    let items = parse_items(&toks)?;
    Some(emit(&items))
}

/// 已经排好了吗 —— `--check` 用。
///
/// `None`（结构不认识）算**不干净**：`--check` 的职责是「提醒你去 `fmt` 一次」，
/// 宁可多提醒一次，也不要在修不好的文件上也报干净。
pub fn is_formatted(src: &str) -> bool {
    format(src).as_deref() == Some(src)
}

// ---- 词法 ----

/// 词法事件。
///
/// 只有四个字符是结构位：`{` `}` `;` `:`。`(` `)` `#` `@` 之类一律算文本 ——
/// 本语言的值里不出现 `;`（实测 19 份文件 0 处），所以不需要括号深度。
enum Tok {
    /// `/* … */` 整段原文。首行缩进已剥掉，续行一字未动（含续行自己的缩进）。
    Comment(String),
    /// 一段非结构化原文：选择器 / 属性名 / 值。内部空白已折叠成单空格。
    Text(String),
    Open,
    Close,
    Semi,
    Colon,
    /// 一个或多个空行（折叠成一个）。
    Blank,
}

fn lex(src: &str) -> Option<Vec<Tok>> {
    let b = src.as_bytes();
    let mut toks: Vec<Tok> = Vec::new();
    // 正在累积的文本段起点。只在遇到结构字符 / 空白 / 注释时才切 —— 那些全是
    // ASCII，而 UTF-8 续字节都 ≥ 0x80，所以 `src[seg..i]` 的切点永不落在字符中间。
    let mut seg: Option<usize> = None;
    let mut i = 0usize;

    while i < b.len() {
        match b[i] {
            b'{' | b'}' | b';' | b':' => {
                flush(&mut toks, &mut seg, src, i);
                toks.push(match b[i] {
                    b'{' => Tok::Open,
                    b'}' => Tok::Close,
                    b';' => Tok::Semi,
                    _ => Tok::Colon,
                });
                i += 1;
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                flush(&mut toks, &mut seg, src, i);
                // `/*` 之后找不到 `*/` ⇒ 注释没闭合 ⇒ 结构不认识。
                let rest = &src[i + 2..];
                let end = rest.find("*/")?;
                let stop = i + 2 + end + 2;
                toks.push(Tok::Comment(src[i..stop].trim_start().to_string()));
                i = stop;
            }
            _ if (b[i] as char).is_ascii_whitespace() => {
                flush(&mut toks, &mut seg, src, i);
                let mut newlines = 0usize;
                while i < b.len() {
                    let w = b[i];
                    if w == b'\n' {
                        newlines += 1;
                        i += 1;
                    } else if (w as char).is_ascii_whitespace() {
                        i += 1;
                    } else {
                        break;
                    }
                }
                // 空行只保留「有没有」，不保留「有几个」。开头的那几个不算。
                if newlines >= 2 && !matches!(toks.last(), Some(Tok::Blank)) {
                    toks.push(Tok::Blank);
                }
            }
            _ => {
                if seg.is_none() {
                    seg = Some(i);
                }
                i += 1;
            }
        }
    }
    flush(&mut toks, &mut seg, src, b.len());
    Some(toks)
}

fn flush(toks: &mut Vec<Tok>, seg: &mut Option<usize>, src: &str, end: usize) {
    if let Some(s) = seg.take() {
        let t = collapse(&src[s..end]);
        // 理论上不会为空（空白已被上一轮吃掉），但空段会推出空 `Text`，
        // 让上层多一个分支 —— 直接不推。
        if !t.is_empty() {
            toks.push(Tok::Text(t));
        }
    }
}

/// 折叠内部空白成单空格，并去掉首尾空白。
///
/// 选择器/属性名/值里的对齐空格（`box.page-guide    {` 那类）在这里收敛。
fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        }
    }
    out
}

// ---- 结构 ----

/// 顶层条目。`gap` = 它前面**原本**有没有空行。
struct Item {
    gap: bool,
    kind: ItemKind,
}

enum ItemKind {
    Comment(String),
    Rule {
        selector: String,
        body: Vec<BodyItem>,
    },
}

struct BodyItem {
    gap: bool,
    kind: BodyKind,
}

enum BodyKind {
    Comment(String),
    Decl { name: String, value: String },
}

fn parse_items(toks: &[Tok]) -> Option<Vec<Item>> {
    let mut items: Vec<Item> = Vec::new();
    let mut i = 0usize;
    let mut gap = false;

    loop {
        match toks.get(i) {
            None => break,
            Some(Tok::Blank) => {
                gap = true;
                i += 1;
            }
            Some(Tok::Comment(c)) => {
                items.push(Item {
                    gap,
                    kind: ItemKind::Comment(c.clone()),
                });
                gap = false;
                i += 1;
            }
            Some(Tok::Text(_)) => {
                // 选择器：可以是由多个 `Text` 拼成的（跨行选择器），但中间不允许
                // 夹注释 —— 那不在存量形状里，宁可不认。
                let mut sel = String::new();
                loop {
                    match toks.get(i) {
                        Some(Tok::Text(t)) => {
                            if !sel.is_empty() {
                                sel.push(' ');
                            }
                            sel.push_str(t);
                            i += 1;
                        }
                        Some(Tok::Open) => {
                            i += 1;
                            break;
                        }
                        _ => return None,
                    }
                }
                if sel.is_empty() {
                    return None;
                }
                let body = parse_body(toks, &mut i)?;
                items.push(Item {
                    gap,
                    kind: ItemKind::Rule {
                        selector: sel,
                        body,
                    },
                });
                gap = false;
            }
            // 顶层出现 `}` / `;` / `{` 而没有选择器 = 源码已坏。
            Some(_) => return None,
        }
    }
    Some(items)
}

/// 解析一个规则体，`*i` 指向 `{` 之后、返回时指向 `}` 之后。
fn parse_body(toks: &[Tok], i: &mut usize) -> Option<Vec<BodyItem>> {
    let mut body: Vec<BodyItem> = Vec::new();
    let mut gap = false;

    loop {
        match toks.get(*i) {
            Some(Tok::Close) => {
                *i += 1;
                return Some(body);
            }
            Some(Tok::Blank) => {
                gap = true;
                *i += 1;
            }
            Some(Tok::Comment(c)) => {
                body.push(BodyItem {
                    gap,
                    kind: BodyKind::Comment(c.clone()),
                });
                gap = false;
                *i += 1;
            }
            Some(Tok::Text(_)) => {
                let name = match toks.get(*i) {
                    Some(Tok::Text(t)) => t.clone(),
                    _ => return None,
                };
                *i += 1;
                match toks.get(*i) {
                    Some(Tok::Colon) => *i += 1,
                    _ => return None,
                }
                let mut value = String::new();
                loop {
                    match toks.get(*i) {
                        Some(Tok::Text(t)) => {
                            if !value.is_empty() {
                                value.push(' ');
                            }
                            value.push_str(t);
                            *i += 1;
                        }
                        Some(Tok::Blank) => *i += 1,
                        Some(Tok::Semi) => {
                            *i += 1;
                            break;
                        }
                        _ => return None,
                    }
                }
                if value.is_empty() {
                    return None;
                }
                body.push(BodyItem {
                    gap,
                    kind: BodyKind::Decl { name, value },
                });
                gap = false;
            }
            _ => return None,
        }
    }
}

// ---- 输出 ----

fn emit(items: &[Item]) -> String {
    let mut out = String::new();
    let mut first = true;

    for it in items {
        // 顶层条目之间：原本有空行就留一个空行，没有就不加（注释"贴"着
        // 它注释的规则）。首条之前不加（文件开头的空行是噪音）。
        if !first && it.gap {
            out.push('\n');
        }
        first = false;
        match &it.kind {
            ItemKind::Comment(c) => {
                out.push_str(c);
                out.push('\n');
            }
            ItemKind::Rule { selector, body } => {
                out.push_str(selector);
                out.push_str(" {\n");
                let mut body_first = true;
                for b in body {
                    if !body_first && b.gap {
                        out.push('\n');
                    }
                    body_first = false;
                    match &b.kind {
                        BodyKind::Comment(c) => {
                            out.push_str("    ");
                            out.push_str(c);
                            out.push('\n');
                        }
                        BodyKind::Decl { name, value } => {
                            out.push_str("    ");
                            out.push_str(name);
                            out.push_str(": ");
                            out.push_str(value);
                            out.push_str(";\n");
                        }
                    }
                }
                out.push_str("}\n");
            }
        }
    }
    out
}

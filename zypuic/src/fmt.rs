//! `.zypui` 排版器：把源码排成规范里那五条规则。
//!
//! # 为什么只动位置，不动内容
//!
//! 属性顺序**不是语义** —— 编译器按名字取属性（`XEl::attr`），把 `class` 挪到
//! 行尾与把它写在中间编出的字节码逐字节相同。所以排版器只做三件事，全部**不改
//! 引号内的任何字节**：
//!
//! 1. 跨行的开始标签合成一行（规则 2/3）；
//! 2. 一行挤了多个元素时摊开成「一个元素一行」（规则 3）；
//! 3. 属性重排成 `key` 在前、`class` 收尾（规则 4/5）。
//!
//! # 五条规则（`violations` 的判据）
//!
//! | # | 规则 | 为什么 |
//! |---|---|---|
//! | 1 | 缩进是 2 的倍数；无 tab、无行尾空白 | 编辑器各有一套默认，不钉住就漂 |
//! | 2 | 注释外的每个非空行以 `<` 开头 | 标签不跨行、也没有裸文本节点 |
//! | 3 | 一行至多一个**开始**标签，且行尾是 `>` 或 `/>` | 一个元素一行，diff 逐行可读 |
//! | 4 | `key` 若在，必须是第一个属性 | 身份先于一切 |
//! | 5 | `class` 若在，必须是最后一个属性 | class 表达式最长，放行尾才对得齐 |
//!
//! # 规则与判据住在同一个模块里
//!
//! 五条排版规则的**判据**就在下面（`violations`），排版门禁只是把它逐文件跑一遍。
//! 上一轮（`docs/dev-log/2026-10-31-zypui-style-normalization.md`）判据与排版
//! 是两份实现，于是有了「门禁少报一半 class 违规」那种漂移。两份实现就要漂移，
//! 所以这次只留一份：**formatter 能修什么，`violations` 就报什么**。
//!
//! # 看不懂的源码不动
//!
//! 与 `zstylec` 的 fmt 同一约定：XML 语法不干净时返回 `None`，调用方原样保留。
//! 排版器只该在看得懂的文件上动手。

/// 排版。`None` = 输入不是合法 XML 子集语法（见模块文档）。
pub fn format(src: &str) -> Option<String> {
    // 换行先统一成 LF。⚠ `\r` 与 `\n` 必须**成对**处理：逐字符把 `\r` 换成 `\n`
    // 会让 CRLF 变成两个 `\n`，即每个行尾多出一个空行。所以先合并 `\r\n`，
    // 剩下的孤立 `\r`（老 Mac 换行）再单独换。
    let normalized = normalize_eol(src);

    // 看不懂的源码不动。parse_xml 只查 XML 层语法（标签闭合、属性引号），
    // 组件名/属性名是否登记归 build 层（见 lib.rs 的说明）。
    let (_, diags) = crate::xml::parse_xml(&normalized, "fmt.zypui");
    if !diags.is_empty() {
        return None;
    }

    Some(relayout(&normalized))
}

// ---- 一行标签的形状 ----

/// 换行统一成 LF：`\r\n` → `\n`（成对），剩下来的孤立 `\r` → `\n`。
fn normalize_eol(src: &str) -> String {
    if !src.contains('\r') {
        return src.to_string();
    }
    let mut out = String::with_capacity(src.len());
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\r' {
            out.push('\n');
            if it.peek() == Some(&'\n') {
                it.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 标签的三种形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagKind {
    /// `<Name …>`（开标签，深度 +1）
    Open,
    /// `<Name …/>`（自闭，深度不变）
    SelfClose,
    /// `</Name>`（闭标签，深度 −1）
    Close,
}

/// 扫一个以 `<` 开头的标签，返回 (形态, 标签结束后的下标)。
///
/// ⚠ 属性值里允许裸 `>`（`cond="a > b"` 不用写 `&gt;`），所以必须跟引号状态，
/// 不能 `find('>')` 了事 —— 那会把 `cond="a > b"` 的 `>` 当成标签结尾。
fn scan_tag(s: &str) -> Option<(TagKind, usize)> {
    let b = s.as_bytes();
    if b.first() != Some(&b'<') {
        return None;
    }
    let mut i = 1usize;
    let closing = b.get(1) == Some(&b'/');
    if closing {
        i = 2;
    }
    // 引号内的字节一律不判结构。属性值不可能含自己的引号（解析器遇到就闭合），
    // 所以「唯一引号定值」这条是语法保证的，不需要转义规则。
    let mut quote: Option<u8> = None;
    let mut prev_slash = false;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' | b'\'' => quote = Some(c),
            b'>' => {
                let kind = if closing {
                    TagKind::Close
                } else if prev_slash {
                    TagKind::SelfClose
                } else {
                    TagKind::Open
                };
                return Some((kind, i + 1));
            }
            _ => prev_slash = c == b'/',
        }
        i += 1;
    }
    // 没找到闭合 ⇒ 这是跨行标签的一段。
    None
}

/// 标签名（`<Control …>` → `Control`）。取不到就返回空串，调用方按原样处理。
fn tag_name(tag: &str) -> String {
    let b = tag.as_bytes();
    let mut i = 1usize;
    if b.get(1) == Some(&b'/') {
        i = 2;
    }
    let start = i;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'-' | b':' | b'.'))
    {
        i += 1;
    }
    tag[start..i].to_string()
}

/// 行首空白（整行照抄缩进之外的属性重排只动属性段，所以缩进必须单独拿出来）。
fn leading_ws(line: &str) -> String {
    let end = line.len() - line.trim_start().len();
    line[..end].to_string()
}

// ---- 属性段 ----

/// 属性名 + 它的原文片段（`name="value"`，含首尾空白已剥掉）。
struct Attr {
    name: String,
    raw: String,
}

/// 把属性段切成一个个属性。**引号内**的字节一律不当分隔符。
///
/// 只处理 XML 语法干净的情形（`format` 已先 `parse_xml` 过一遍）：属性一定形如
/// `name="value"` 或 `name='value'`。解析不出形状时把剩余整段当一个属性交出去 ——
/// 调用方会原样保留，不会编造。
///
/// ⚠ 这个函数曾经死循环过：解析完一个属性推入 `v` 之后忘了把游标推到属性末尾，
/// `while i < b.len()` 于是原地重启，同一个属性被推一次一次又一次，直到
/// 申请 1.6 GB 失败。所以下面**每一次** `push` 之前都显式把 `i` 推到该属性的
/// 结尾 —— 「循环变量必须前进」在这里不是风格问题，是「测试跑不跑得完」的问题。
fn split_attrs(s: &str) -> Vec<Attr> {
    let b = s.as_bytes();
    let mut v: Vec<Attr> = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let start = i;
        while i < b.len()
            && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'-' | b':' | b'.'))
        {
            i += 1;
        }
        let name = s[start..i].to_string();
        let mut j = i;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if b.get(j) != Some(&b'=') {
            if !name.is_empty() {
                v.push(Attr {
                    name,
                    raw: s[start..].trim_end().to_string(),
                });
            }
            // 剩余整段交出去，不进下一轮。
            return v;
        }
        j += 1;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let q = match b.get(j) {
            Some(&c) if c == b'"' || c == b'\'' => c,
            _ => {
                v.push(Attr {
                    name,
                    raw: s[start..].trim_end().to_string(),
                });
                return v;
            }
        };
        j += 1;
        while j < b.len() && b[j] != q {
            j += 1;
        }
        if j < b.len() {
            j += 1; // 吃掉闭合引号
        }
        v.push(Attr {
            name,
            raw: s[start..j].trim_end().to_string(),
        });
        // ⚠ 没有这一行，上面的循环就是死的。游标必须越过刚解析完的属性。
        i = j;
    }
    v
}

/// 属性重排：`key` 提前到第一位，`class` 挪到最后一位，其余保持原相对顺序。
///
/// 只动属性**位置**，引号内内容一个字节不改 —— 这正是「属性顺序不是语义」的
/// 凭据（`XEl::attr` 按名取）。class 的词序（类序即匹配序）是语义，而在引号
/// 内部，一个字节都不碰。
fn reorder_attrs(tag: &str) -> String {
    if !tag.starts_with('<') || tag.starts_with("</") {
        return tag.to_string();
    }
    let b = tag.as_bytes();
    // 标签名结尾 = 属性段起点。
    let mut i = 1usize;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'-' | b':' | b'.'))
    {
        i += 1;
    }
    let term = if tag.ends_with("/>") { "/>" } else { ">" };
    let body_end = b.len() - term.len();
    if body_end < i {
        return tag.to_string();
    }
    let attrs = split_attrs(&tag[i..body_end]);
    if attrs.is_empty() {
        return tag.to_string();
    }
    let mut order: Vec<usize> = (0..attrs.len()).collect();
    if let Some(p) = attrs.iter().position(|a| a.name == "key") {
        if p != 0 {
            let k = order.remove(p);
            order.insert(0, k);
        }
    }
    if let Some(p) = order.iter().position(|&j| attrs[j].name == "class") {
        if p + 1 != order.len() {
            let c = order.remove(p);
            order.push(c);
        }
    }
    let joined = order
        .iter()
        .map(|&j| attrs[j].raw.clone())
        .collect::<Vec<_>>()
        .join(" ");
    format!("{} {}{}", &tag[..i], joined, term)
}

// ---- 摊开 & 重排 ----

/// 把一个「逻辑行」（可能挤了多个元素）摊成一串裸行：一个元素一行。
fn unpack(buf: &str) -> Vec<String> {
    let Some((kind, end)) = scan_tag(buf) else {
        // 扫不到闭合 —— 交给 `relayout` 的续接逻辑，不该走到这里。
        return vec![buf.to_string()];
    };
    let head = buf[..end].to_string();
    let rest = buf[end..].trim();
    if rest.is_empty() {
        return vec![head];
    }
    if kind == TagKind::Open {
        let name = tag_name(&head);
        let close = format!("</{name}>");
        if rest.ends_with(&close) {
            let inner = rest[..rest.len() - close.len()].trim();
            let mut v = vec![head];
            v.extend(unpack_sequence(inner));
            v.push(close);
            return v;
        }
    }
    // 闭标签对不上（或头是自闭标签后面还挂着东西）⇒ 保守整行保留。
    vec![buf.to_string()]
}

/// 摊开一串连续的元素，每个一行。
fn unpack_sequence(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let Some((_, end)) = scan_tag(rest) else {
            out.push(rest.to_string());
            return out;
        };
        out.extend(unpack(&rest[..end]));
        rest = rest[end..].trim();
    }
    out
}

// ---- 主流程 ----

fn relayout(src: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut depth: usize = 0;
    // 跨行标签续接中的 (缩进, 已累积文本)。
    let mut pending: Option<(String, String)> = None;
    let mut in_comment = false;

    let lines: Vec<&str> = if let Some(body) = src.strip_suffix('\n') {
        body.split('\n').collect()
    } else {
        src.split('\n').collect()
    };

    for raw in &lines {
        if in_comment {
            out.push((*raw).to_string());
            if raw.contains("-->") {
                in_comment = false;
            }
            continue;
        }
        // 含注释的行**照抄**：注释里出现 `<` / `>` 都可能，跟标签续接混在一起
        // 只会猜错。仓内存量注释都独占一行，照抄即零改动。
        if raw.contains("<!--") {
            out.push(raw.trim_end().to_string());
            if !raw.contains("-->") {
                in_comment = true;
            }
            continue;
        }
        if raw.trim().is_empty() {
            // 空行照抄（续接中的跨行标签也允许中间有空行）。
            out.push(String::new());
            continue;
        }

        let (indent, buf) = match pending.take() {
            Some((ind, b)) => (ind, format!("{b} {}", raw.trim_start())),
            None => (leading_ws(raw), raw.trim_start().to_string()),
        };

        if scan_tag(&buf).is_none() {
            pending = Some((indent, buf));
            continue;
        }

        for line in unpack(&buf) {
            emit_one(&mut out, &mut depth, &line, &indent);
        }
    }

    if let Some((indent, buf)) = pending {
        // 续接没等到闭合：源码不干净（已被 parse_xml 挡掉），原样输出。
        out.push(format!("{indent}{buf}"));
    }

    let mut s = out.join("\n");
    // 空文件保持空；否则文末一个换行。行尾空白已在上面逐行 trim_end。
    if !s.is_empty() {
        s.push('\n');
    }
    s
}

/// 输出一行，并按形态维护缩进深度。深度 = 未闭合开标签数。
fn emit_one(out: &mut Vec<String>, depth: &mut usize, line: &str, indent: &str) {
    let Some((kind, _)) = scan_tag(line) else {
        out.push(format!("{indent}{line}"));
        return;
    };
    let fixed = reorder_attrs(line);
    match kind {
        TagKind::Open => {
            out.push(format!("{}{}", " ".repeat(*depth * 2), fixed));
            *depth += 1;
        }
        TagKind::SelfClose => {
            out.push(format!("{}{}", " ".repeat(*depth * 2), fixed));
        }
        TagKind::Close => {
            *depth = depth.saturating_sub(1);
            out.push(format!("{}{}", " ".repeat(*depth * 2), fixed));
        }
    }
}

// ---- 五条规则的判据 ----

/// 注释**打码**而不是删掉：字节数与换行数逐字节不变，于是「第 N 行」永远指源文件
/// 第 N 行，注释里的 `<` 也不会被当成标签。
///
/// 只替换 ASCII 注释定界符内部的字节（多字节字符的续字节都 ≥ 0x80，不可能撞上
/// `<!--` / `-->`），新行保留、其余换成空格。
fn mask_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"<!--") {
            let end = match b[i..].windows(3).position(|w| w == b"-->") {
                Some(p) => i + p + 3,
                None => b.len(),
            };
            for &c in &b[i..end] {
                out.push(if c == b'\n' { b'\n' } else { b' ' });
            }
            i = end;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("打码只替换 ASCII 注释区，UTF-8 结构不变")
}

/// 一行里开始标签 `<Name` 的出现次数（`</Name>` 是闭标签，不算）。
fn start_tags(line: &str) -> usize {
    let b = line.as_bytes();
    let mut n = 0;
    for i in 0..b.len() {
        if b[i] == b'<' && i + 1 < b.len() && b[i + 1].is_ascii_alphabetic() {
            n += 1;
        }
    }
    n
}

/// 取一行的属性名列表（按出现顺序）。
///
/// 走 `scan_tag` + `split_attrs` —— 与排版器**同一份**扫描代码，所以「门禁看得见
/// 的」必然覆盖「排版器改了位置的那些属性」。上一轮这里手写了一份只认 `"`
/// 的版本，结果单引号属性的 `class` 审查直接失明。
fn attr_names(line: &str) -> Vec<String> {
    let t = line.trim_start();
    let Some((kind, end)) = scan_tag(t) else {
        return Vec::new();
    };
    if kind == TagKind::Close {
        return Vec::new();
    }
    let body_end = if kind == TagKind::SelfClose {
        end - 2
    } else {
        end - 1
    };
    let name_end = tag_name_end(t);
    if body_end < name_end {
        return Vec::new();
    }
    split_attrs(&t[name_end..body_end])
        .into_iter()
        .map(|a| a.name)
        .collect()
}

/// 标签名结束下标（`<Control …` 里 `<` 之后那段名字的尾巴）。
fn tag_name_end(t: &str) -> usize {
    let b = t.as_bytes();
    let mut i = 1usize;
    if b.get(1) == Some(&b'/') {
        i = 2;
    }
    while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'-' | b':' | b'.'))
    {
        i += 1;
    }
    i
}

/// 按五条规则逐行检查，返回全部违规（不短路 —— 一次报完省几轮）。
///
/// 消息形如 `行号: 规则N 说明`，不带文件名：文件名是调用方（门禁）的事，
/// 这样这条判据也能被单元测试直接拿去用。
pub fn violations(src: &str) -> Vec<String> {
    let mut v = Vec::new();
    let masked = mask_comments(src);
    // 原文查排版（规则1），打码副本查结构（规则2-5）——两者行号严格对齐。
    for (i, (raw, masked_line)) in src.lines().zip(masked.lines()).enumerate() {
        let no = i + 1;
        if raw.contains('\t') {
            v.push(format!("{no}: 规则1 含 tab"));
        }
        if raw != raw.trim_end() {
            v.push(format!("{no}: 规则1 行尾有空白：{raw:?}"));
        }
        let line = masked_line.trim_end();
        // 整行都是注释或空白：结构规则无从谈起（规则1 已在原文上查过）。
        if line.trim_start().is_empty() {
            continue;
        }
        // 缩进只对真正的元素行有意义 —— 多行注释的续行是按 `<!--` 对齐的（5 空格）。
        let indent = raw.len() - raw.trim_start().len();
        if indent % 2 != 0 {
            v.push(format!("{no}: 规则1 缩进不是 2 的倍数（{indent}）"));
        }
        if !line.trim_start().starts_with('<') {
            v.push(format!("{no}: 规则2 非空行必须以 `<` 开头：{line:?}"));
            continue;
        }
        if start_tags(line) > 1 {
            v.push(format!("{no}: 规则3 一行多于一个开始标签：{line:?}"));
        }
        if start_tags(line) > 0 {
            if !line.ends_with('>') {
                v.push(format!(
                    "{no}: 规则3 标签没有在行尾闭合（跨行？）：{line:?}"
                ));
            }
            let names = attr_names(line);
            if let Some(p) = names.iter().position(|n| n == "key") {
                if p != 0 {
                    v.push(format!("{no}: 规则4 key 不是第一个属性：{line}"));
                }
            }
            if let Some(p) = names.iter().position(|n| n == "class") {
                if p + 1 != names.len() {
                    v.push(format!("{no}: 规则5 class 不是最后一个属性：{line}"));
                }
            }
        } else if !line.ends_with('>') {
            v.push(format!("{no}: 规则3 闭标签行应以 `>` 结尾：{line:?}"));
        }
    }
    v
}

/// 已按五条规则排好，且**排版器也认得**吗。
///
/// 两个条件都要：只判后者的话，门禁绿与排版器认识是两件事，`--check` 会在
/// 排版器修不了的文件上说干净。
pub fn is_formatted(src: &str) -> bool {
    violations(src).is_empty() && format(src).as_deref() == Some(src)
}

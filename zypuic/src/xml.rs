//! 手写 XML 子集解析器。
//!
//! # 支持的语法（全部，没有别的）
//!
//! ```text
//! doc      := (ws | comment | element)*
//! element  := "<" name attr* "/>"  |  "<" name attr* ">" body "</" name ">"
//! attr     := name "=" ('"' v '"' | "'" v "'")
//! comment  := "<!--" … "-->"
//! ```
//!
//! # 对 XML 的刻意偏离
//!
//! - **属性值里允许裸 `<` `>`**：条件表达式 `cond="a < b"` 不用写成
//!   `&lt;`，写 UI 的人不该跟转义符搏斗。字符串里的引号用另一种引号
//!   包属性（`cond='x == "a"'`），或 `&quot;`。
//! - 文本内容只在 `<text>` 元素里有意义，别处出现非空白文本直接报错 ——
//!   静默吞掉一段字比报错糟得多。
//!
//! # 报错质量是本模块的存在理由
//!
//! 每条错误带行号列号。解析器唯一的工作就是把「第几行哪个字符写错了」
//! 说清楚，剩下的判断交给 build 阶段。

/// 一条解析/校验诊断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    /// 来源文件名（只用于显示）。
    pub file: &'static str,
    /// 1 起行号。
    pub line: u32,
    /// 1 起列号。
    pub col: u32,
    /// 级别（本编译器只产 error；预留 warning 给未来的拼写建议）。
    pub level: Level,
    /// 说明（中文，说清「错在哪、为什么」）。
    pub msg: String,
}

/// 诊断级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// 错误：阻断编译。
    Error,
    /// 警告：不阻断。
    Warning,
}

impl Diag {
    pub(crate) fn error(file: &'static str, line: u32, col: u32, msg: impl Into<String>) -> Self {
        Self {
            file,
            line,
            col,
            level: Level::Error,
            msg: msg.into(),
        }
    }

    /// `file:line:col: level: msg`（rustc 风格，编辑器可点击）。
    pub fn render(&self) -> String {
        let lv = match self.level {
            Level::Error => "error",
            Level::Warning => "warning",
        };
        format!(
            "{}:{}:{}: {}: {}",
            self.file, self.line, self.col, lv, self.msg
        )
    }
}

/// 解析出的 XML 元素。
#[derive(Debug, Clone)]
pub struct XEl {
    /// 标签名。
    pub name: String,
    /// `(名, 值(实体已解码), 行, 列)`。
    pub attrs: Vec<(String, String, u32, u32)>,
    /// 子节点。
    pub children: Vec<XNode>,
    /// 直接文本内容（实体已解码；只拼相邻文本段）。
    pub text: String,
    /// 元素名的行号（报错用）。
    pub line: u32,
    /// 列号。
    pub col: u32,
}

impl XEl {
    /// 取属性值（找不到返回 `None`）。
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, ..)| n == name)
            .map(|(_, v, ..)| v.as_str())
    }
}

/// XML 子节点。
#[derive(Debug, Clone)]
pub enum XNode {
    /// 元素。
    El(XEl),
    /// 文本（行号供「别处不该有文本」的报错用）。
    Text(String, u32, u32),
}

/// 解析一份 XML 子集源码。语法错误进 `Vec<Diag>`，**尽量继续解析**
/// （一次看到全部错，而不是改一个跑一趟）。
pub fn parse_xml(src: &str, file: &'static str) -> (Vec<XEl>, Vec<Diag>) {
    let mut p = Parser {
        s: src.as_bytes(),
        src,
        i: 0,
        file,
        diags: Vec::new(),
    };
    let roots = p.doc();
    (roots, p.diags)
}

struct Parser<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
    file: &'static str,
    diags: Vec<Diag>,
}

impl<'a> Parser<'a> {
    fn error(&mut self, line: u32, col: u32, msg: impl Into<String>) {
        self.diags.push(Diag::error(self.file, line, col, msg));
    }

    fn pos(&self) -> (u32, u32) {
        let (mut line, mut col) = (1u32, 1u32);
        for b in &self.s[..self.i.min(self.s.len())] {
            if *b == b'\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    /// 报错但以「当前位置」算行列（先记位置再前进的场景用）。
    fn error_at(&mut self, msg: impl Into<String>) {
        let (l, c) = self.pos();
        self.error(l, c, msg);
    }

    fn skip_ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] as char).is_ascii_whitespace() {
            self.i += 1;
        }
    }

    /// 跳过空白与注释。`<?…?>` / `<!DOCTYPE` 处理指令：跳过并警告
    /// （XML 工具链会往里塞东西，静默跳过会让「别的工具写的头」隐形）。
    fn skip_extra(&mut self) {
        loop {
            self.skip_ws();
            if self.s[self.i..].starts_with(b"<!--") {
                let (l, c) = self.pos();
                match find(self.s, self.i + 4, b"-->") {
                    Some(end) => self.i = end + 3,
                    None => {
                        self.error(l, c, "注释没有闭合（缺 `-->`）");
                        self.i = self.s.len();
                    }
                }
            } else if self.s[self.i..].starts_with(b"<?") || self.s[self.i..].starts_with(b"<!D") {
                let (l, c) = self.pos();
                let end = find(self.s, self.i, b">")
                    .map(|e| e + 1)
                    .unwrap_or(self.s.len());
                self.diags.push(Diag {
                    file: self.file,
                    line: l,
                    col: c,
                    level: Level::Warning,
                    msg: "处理指令/DOCTYPE 不属于 .zypui，已跳过".into(),
                });
                self.i = end;
            } else {
                return;
            }
        }
    }

    fn doc(&mut self) -> Vec<XEl> {
        let mut roots = Vec::new();
        loop {
            self.skip_extra();
            if self.i >= self.s.len() {
                return roots;
            }
            if self.s[self.i] != b'<' {
                let (l, c) = self.pos();
                // 收掉这段流浪文本，报一条错，继续。
                let start = self.i;
                while self.i < self.s.len() && self.s[self.i] != b'<' {
                    self.i += 1;
                }
                let text = &self.src[start..self.i];
                if !text.trim().is_empty() {
                    self.error(
                        l,
                        c,
                        format!("顶层出现流浪文本 `{}`（顶层只能是元素）", ellipsis(text)),
                    );
                }
                continue;
            }
            match self.element() {
                Some(el) => roots.push(el),
                // element() 内部已报错；保证前进，避免死循环。
                None => {
                    if self.i < self.s.len() && self.s[self.i] == b'<' {
                        self.i += 1;
                    }
                }
            }
        }
    }

    /// 解析一个元素。返回 `None` = 当前 `<` 不是元素开头（或语法坏死）。
    fn element(&mut self) -> Option<XEl> {
        let (line, col) = self.pos();
        debug_assert_eq!(self.s[self.i], b'<');
        self.i += 1;
        let name = self.ident();
        if name.is_empty() {
            self.error_at("`<` 后面不是标签名");
            return None;
        }
        let mut attrs = Vec::new();
        loop {
            self.skip_ws();
            if self.i >= self.s.len() {
                self.error(line, col, format!("<{name}> 没有闭合（文件提前结束）"));
                return None;
            }
            match self.s[self.i] {
                b'/' => {
                    // 自闭合。
                    self.i += 1;
                    if self.i < self.s.len() && self.s[self.i] == b'>' {
                        self.i += 1;
                        return Some(XEl {
                            name,
                            attrs,
                            children: Vec::new(),
                            text: String::new(),
                            line,
                            col,
                        });
                    }
                    self.error_at(format!("<{name}/> 自闭合斜杠后不是 `>`"));
                    return None;
                }
                b'>' => {
                    self.i += 1;
                    break;
                }
                _ => {
                    if let Some(a) = self.attr() {
                        attrs.push(a);
                    } else {
                        self.error_at(format!("<{name}> 的属性区出现意外字符"));
                        return None;
                    }
                }
            }
        }

        // 有开始标签，收 body。
        let mut children = Vec::new();
        let mut text = String::new();
        loop {
            self.skip_extra();
            if self.i >= self.s.len() {
                self.error(line, col, format!("<{name}> 没有闭合（文件提前结束）"));
                return None;
            }
            if self.s[self.i] == b'<' {
                if self.s[self.i..].starts_with(b"</") {
                    // 闭合标签。
                    self.i += 2;
                    let close = self.ident();
                    self.skip_ws();
                    let ok_close = close == name && self.i < self.s.len() && self.s[self.i] == b'>';
                    if ok_close {
                        self.i += 1;
                    } else {
                        self.error_at(format!(
                            "闭合标签 </{}> 与开始标签 <{name}> 不配",
                            if close.is_empty() { "?" } else { &close }
                        ));
                    }
                    return Some(XEl {
                        name,
                        attrs,
                        children,
                        text,
                        line,
                        col,
                    });
                }
                if let Some(child) = self.element() {
                    children.push(XNode::El(child));
                } else {
                    // 子元素语法坏死：返回 None 让上层报错后前进，避免雪崩。
                    return None;
                }
            } else {
                // 文本段（到下一个 `<`）。
                let (l, c) = self.pos();
                let start = self.i;
                while self.i < self.s.len() && self.s[self.i] != b'<' {
                    self.i += 1;
                }
                let raw = &self.src[start..self.i];
                let decoded = decode_entities(raw);
                if !decoded.trim().is_empty() {
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(decoded.trim());
                    children.push(XNode::Text(decoded.trim().to_string(), l, c));
                }
            }
        }
    }

    /// 属性：`name = "v"` / `name='v'`。失败返回 None 并报错。
    fn attr(&mut self) -> Option<(String, String, u32, u32)> {
        let (line, col) = self.pos();
        let name = self.ident();
        if name.is_empty() {
            return None;
        }
        self.skip_ws();
        if self.i >= self.s.len() || self.s[self.i] != b'=' {
            self.error(line, col, format!("属性 `{name}` 缺 `=`"));
            return None;
        }
        self.i += 1;
        self.skip_ws();
        if self.i >= self.s.len() || (self.s[self.i] != b'"' && self.s[self.i] != b'\'') {
            self.error(line, col, format!("属性 `{name}` 的值要用引号包起来"));
            return None;
        }
        let quote = self.s[self.i];
        self.i += 1;
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i] != quote {
            self.i += 1;
        }
        if self.i >= self.s.len() {
            self.error(line, col, format!("属性 `{name}` 的值没有闭合引号"));
            return None;
        }
        let raw = &self.src[start..self.i];
        self.i += 1; // 吃掉闭合引号
        Some((name, decode_entities(raw), line, col))
    }

    /// 标识符：字母/`_` 开头，字母/数字/`_`/`-`/`:` 续。
    fn ident(&mut self) -> String {
        let start = self.i;
        if self.i < self.s.len() && (self.s[self.i].is_ascii_alphabetic() || self.s[self.i] == b'_')
        {
            self.i += 1;
            while self.i < self.s.len()
                && (self.s[self.i].is_ascii_alphanumeric()
                    || self.s[self.i] == b'_'
                    || self.s[self.i] == b'-'
                    || self.s[self.i] == b':')
            {
                self.i += 1;
            }
        }
        self.src[start..self.i].to_string()
    }
}

/// 子串查找。
fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// 五个预定义实体。数字实体不给（`.zypui` 里用不上，给了就是第二种写法）。
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// 报错文本里太长的截断。
fn ellipsis(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() > 24 {
        let head: String = t.chars().take(24).collect();
        format!("{head}…")
    } else {
        t.to_string()
    }
}

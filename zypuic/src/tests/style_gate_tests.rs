//! 仓内**全部** `*.zypui` 的排版门禁：写法统一靠测试钉住，不靠评审记性。
//!
//! # 为什么值得一条测试
//!
//! 属性顺序**不是语义** —— zypuic 按名字取属性（`XEl::attr`），把 `class` 挪到
//! 行尾与把它放前面编出的字节码逐字节相同。既然编译器不管，「写法统一」就没有
//! 任何机器兜底：三份文件曾各写各的（class 有的在中、有的在尾；标签有的跨三行；
//! `<if>` 有的挤一行有的三行）。人看着难受，diff 也难读。
//!
//! 这几条规则**只影响人读**，所以门禁判的是排版而不是语义 —— 真正的语义正确性
//! 由 zypuic 编译期报错 + gallery 整树门禁承担（见 `docs/developer/07-declarative-ui.md`）。
//!
//! # 规则
//!
//! | # | 规则 | 为什么 |
//! |---|---|---|
//! | 1 | 缩进是 2 的倍数；无 tab、无行尾空白 | 编辑器各有一套默认，不钉住就漂 |
//! | 2 | 注释外的每个非空行以 `<` 开头 | 标签不跨行、也没有裸文本节点 |
//! | 3 | 一行至多一个**开始**标签，且行尾是 `>` 或 `/>` | 一个元素一行，diff 逐行可读 |
//! | 4 | `key` 若在，必须是第一个属性 | 身份先于一切 |
//! | 5 | `class` 若在，必须是最后一个属性 | class 表达式最长，放行尾才对得齐 |
//!
//! 属性之间空格数不查：字符串**值内部**的多空格是内容
//! （`text="  --route reference"`），按行做正则会误伤。

use std::path::{Path, PathBuf};

/// 工作区根 = `zypuic/` 的父目录（`CARGO_MANIFEST_DIR` 指向本 crate）。
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zypuic 在工作区根下")
        .to_path_buf()
}

/// 递归收集仓内全部 `.zypui`，跳过 `target` / `.git`（构建产物与仓库元数据）。
fn collect_zypui(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name == "target" || name == ".git" || name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            collect_zypui(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("zypui") {
            out.push(p);
        }
    }
}

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

/// 属性名列表（`key` / `class` 位置判据用）。值里的 `"` 不存在：表达式用单引号。
fn attr_names(tag: &str) -> Vec<&str> {
    let mut v = Vec::new();
    // ⚠ 必须先 trim_start：带缩进时下面那次 `find(whitespace)` 只吃掉**第一个**空白字符，
    // 余下的空白会让标签名与第一个属性粘在一起（`<control class` 被当成一个名字），
    // 于是 `class` / `key` 从列表里凭空消失 —— 门禁当场失明（少报一半违规）。
    let mut rest = tag.trim_start();
    // 跳过标签名本身
    if let Some(sp) = rest.find(|c: char| c.is_whitespace()) {
        rest = &rest[sp..];
    } else {
        return v;
    }
    while let Some(i) = rest.find(|c: char| c.is_whitespace()) {
        rest = &rest[i + 1..];
        if rest.starts_with("/>") || rest.starts_with('>') {
            break;
        }
        let end = rest.find('=').unwrap_or(rest.len());
        let name: &str = &rest[..end];
        if name.is_empty() {
            break;
        }
        v.push(name);
        // 整个 `name="value"` 推进
        match rest.find('"') {
            Some(q) => match rest[q + 1..].find('"') {
                Some(q2) => rest = &rest[q + 1 + q2 + 1..],
                None => break,
            },
            None => break,
        }
    }
    v
}

/// 逐文件跑五条规则，返回全部违规（不短路 —— 一次报完省几轮）。
fn violations(path: &Path) -> Vec<String> {
    let mut v = Vec::new();
    let rel = path
        .strip_prefix(workspace_root())
        .unwrap_or(path)
        .display()
        .to_string();
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            v.push(format!("{rel}: 读不了：{e}"));
            return v;
        }
    };
    // 原文查排版（规则1），打码副本查结构（规则2-5）——两者行号严格对齐。
    let masked = mask_comments(&src);
    for (i, (raw, masked_line)) in src.lines().zip(masked.lines()).enumerate() {
        let no = i + 1;
        if raw.contains('\t') {
            v.push(format!("{rel}:{no}: 规则1 含 tab"));
        }
        if raw != raw.trim_end() {
            v.push(format!("{rel}:{no}: 规则1 行尾有空白：{raw:?}"));
        }
        // 整行都是注释或空白：结构规则无从谈起（规则1 已在原文上查过，包括行尾空白）。
        let line = masked_line.trim_end();
        if line.trim_start().is_empty() {
            continue;
        }
        // 缩进只对真正的元素行有意义 —— 多行注释的续行是按 `<!--` 对齐的（5 空格）。
        let indent = raw.len() - raw.trim_start().len();
        if indent % 2 != 0 {
            v.push(format!("{rel}:{no}: 规则1 缩进不是 2 的倍数（{indent}）"));
        }
        if !line.trim_start().starts_with('<') {
            v.push(format!("{rel}:{no}: 规则2 非空行必须以 `<` 开头：{line:?}"));
            continue;
        }
        if start_tags(line) > 1 {
            v.push(format!("{rel}:{no}: 规则3 一行多于一个开始标签：{line:?}"));
        }
        if start_tags(line) > 0 {
            if !line.ends_with('>') {
                v.push(format!(
                    "{rel}:{no}: 规则3 标签没有在行尾闭合（跨行？）：{line:?}"
                ));
            }
            let names = attr_names(line);
            if let Some(p) = names.iter().position(|n| *n == "key") {
                if p != 0 {
                    v.push(format!("{rel}:{no}: 规则4 key 不是第一个属性：{line:?}"));
                }
            }
            if let Some(p) = names.iter().position(|n| *n == "class") {
                if p + 1 != names.len() {
                    v.push(format!(
                        "{rel}:{no}: 规则5 class 不是最后一个属性：{line:?}"
                    ));
                }
            }
        } else if !line.ends_with('>') {
            v.push(format!("{rel}:{no}: 规则3 闭标签行应以 `>` 结尾：{line:?}"));
        }
    }
    v
}

#[test]
fn repo_zypui_files_obey_the_layout_rules() {
    let mut files = Vec::new();
    collect_zypui(&workspace_root(), &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "一个 .zypui 都没找到 —— 工作区根算错了？（{}）",
        workspace_root().display()
    );

    let all: Vec<String> = files.iter().flat_map(|f| violations(f)).collect();
    assert!(
        all.is_empty(),
        "共 {} 个 .zypui，{} 条排版违规（写法见 docs/developer/zypui-style.md）：\n{}",
        files.len(),
        all.len(),
        all.join("\n")
    );
}

/// 规则本身要能抓到违规 —— 拿几段故意写坏的源码验判据，防止门禁形同虚设。
#[test]
fn layout_violations_are_actually_detected() {
    // 规则5：class 不在末尾。
    let bad = r#"<container key="a" class="x" icon="gear"/>"#;
    let dir = std::env::temp_dir().join("zypuic-style-gate-selftest");
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let f = dir.join("bad.zypui");
    std::fs::write(&f, format!("{bad}\n")).expect("写临时文件");
    let v = violations(&f);
    assert!(
        v.iter().any(|x| x.contains("规则5")),
        "class 在中间应当被抓到，得到 {v:?}"
    );

    // 规则4：key 不是第一个。
    let f2 = dir.join("bad2.zypui");
    std::fs::write(&f2, "<container class=\"x\" key=\"a\"/>\n").expect("写临时文件");
    assert!(
        violations(&f2).iter().any(|x| x.contains("规则4")),
        "key 在 class 之后应当被抓到"
    );

    // 规则3：标签跨行。
    let f3 = dir.join("bad3.zypui");
    std::fs::write(&f3, "<container key=\"a\"\n  class=\"x\"/>\n").expect("写临时文件");
    let v3 = violations(&f3);
    assert!(
        v3.iter().any(|x| x.contains("规则3")),
        "跨行标签应当被抓到，得到 {v3:?}"
    );

    // 规则2：裸文本。
    let f4 = dir.join("bad4.zypui");
    std::fs::write(&f4, "hello\n").expect("写临时文件");
    assert!(
        violations(&f4).iter().any(|x| x.contains("规则2")),
        "裸文本应当被抓到"
    );

    // 规则5 也管**没有 key** 的元素（`<control class="…" interactive="true">`）。
    let f6 = dir.join("nokey.zypui");
    std::fs::write(&f6, "<control class=\"button x\" interactive=\"true\"/>\n")
        .expect("写临时文件");
    assert!(
        violations(&f6).iter().any(|x| x.contains("规则5")),
        "无 key 元素的 class 不在末尾也应当被抓到，得到 {:?}",
        violations(&f6)
    );

    // 缩进行必须同样被看见：`attr_names` 曾把 `<control class` 当成一个名字，
    // 于是带缩进的违规一条都报不出来（下面两条就是那次的回归）。
    let f7 = dir.join("indented5.zypui");
    std::fs::write(&f7, "  <control class=\"button\" interactive=\"true\"/>\n")
        .expect("写临时文件");
    let v7 = violations(&f7);
    assert!(
        v7.iter().any(|x| x.contains("规则5")),
        "缩进行的 class 不在末尾应当被抓到，得到 {v7:?}"
    );

    let f8 = dir.join("indented4.zypui");
    std::fs::write(
        &f8,
        "  <control interactive=\"true\" key=\"a\" class=\"x\"/>\n",
    )
    .expect("写临时文件");
    let v8 = violations(&f8);
    assert!(
        v8.iter().any(|x| x.contains("规则4")),
        "缩进行的 key 不在首位应当被抓到，得到 {v8:?}"
    );

    // 干净写法不报（注释里的 < 也不算标签）。
    let f5 = dir.join("ok.zypui");
    std::fs::write(
        &f5,
        "<!-- <not-a-tag class=\"in-comment\" key=\"x\"> -->\n<container key=\"a\" text=\"hi\" class=\"c\"/>\n",
    )
    .expect("写临时文件");
    assert!(
        violations(&f5).is_empty(),
        "干净文件不该报，得到 {:?}",
        violations(&f5)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

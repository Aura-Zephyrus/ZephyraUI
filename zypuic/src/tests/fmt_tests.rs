//! `fmt` 的单元测试。
//!
//! # 这些用例守什么
//!
//! 排版器承诺「只动位置、不动内容」，且与排版门禁（`style_gate_tests.rs`）判的
//! 五条规则**完全对齐**。测试的责任是把这两件事钉住：
//!
//! - 排版前后语义等价：属性多重集 / 标签多重集 / 注释条数逐一相等（`same_tree`）；
//! - 在**真文件**上更强一档：把同一份文件胡乱排版一遍再排回来，编译产物与
//!   原件**逐字节相同**（`repo_file_formatting_produces_identical_bytecode`）；
//! - 已排好的文件逐字节不变（否则每次 `fmt` 都是新 diff）；
//! - 门禁判的每一条，`fmt` 都真的能修（「跑完 fmt 门禁还红」比没有 fmt 更糟）；
//! - 引号内一个字节不改（class 的**词序**是语义：类序即匹配序）。

use crate::fmt::{format, is_formatted};

/// 源码里**全部** `name="value"`（单引号同样收）的原文 —— 排版器只动属性之间
/// 的空白，引号内一个字节都不该改，所以多重集必须逐一相等。
fn attr_runs(src: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let b = src.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let start = i;
            while i < b.len()
                && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'-' | b':' | b'.'))
            {
                i += 1;
            }
            let mut j = i;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if b.get(j) == Some(&b'=') {
                j += 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if let Some(&q) = b.get(j) {
                    if q == b'"' || q == b'\'' {
                        j += 1;
                        while j < b.len() && b[j] != q {
                            j += 1;
                        }
                        if j < b.len() {
                            j += 1;
                        }
                        v.push(src[start..j].to_string());
                        i = j;
                        continue;
                    }
                }
            }
            continue;
        }
        i += 1;
    }
    v.sort();
    v
}

/// 全部标签名的多重集（开/闭都算：丢一行元素会让它当场不等）。
fn tag_runs(src: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let b = src.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'<' && i + 1 < b.len() && b[i + 1].is_ascii_alphabetic() {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
                j += 1;
            }
            v.push(src[i + 1..j].to_string());
        }
    }
    v.sort();
    v
}

/// 排版前后**语义等价**：属性多重集 / 标签多重集 / 注释条数逐一相等。
///
/// 这是与上一轮（2026-10-31 `.zypui` 归一化）同一把尺子。单个测试片段不一定是
/// 能独立编译的文件（顶层只认 `<component>` / `<page>`，`cond` 只在 `<if>` 上），
/// 所以这里不拿「编译产物逐字节相同」当通用判据 —— 那个留给
/// `repo_file_formatting_produces_identical_bytecode`（真文件，可编译）。
fn same_tree(before: &str, after: &str) {
    assert_eq!(
        attr_runs(before),
        attr_runs(after),
        "排版改变了属性原文：\n--- before ---\n{before}\n--- after (formatted) ---\n{after}",
    );
    assert_eq!(
        tag_runs(before),
        tag_runs(after),
        "排版改变了标签：\n--- before ---\n{before}\n--- after (formatted) ---\n{after}",
    );
    let cb = before.matches("<!--").count();
    let ca = after.matches("<!--").count();
    assert_eq!(cb, ca, "排版改变了注释条数");
}

#[test]
fn already_formatted_stays_byte_identical() {
    let src = "<!-- 注释 -->\n<component name=\"X\">\n  <container key=\"a\" text=\"hi\" class=\"c\"/>\n</component>\n";
    assert_eq!(format(src).as_deref(), Some(src));
    assert!(is_formatted(src));
}

#[test]
fn class_moves_to_the_last_position_without_touching_its_words() {
    // 位置动，词序一个字节不动（词序即匹配序，是语义）。
    let src = "<container key=\"a\" class=\"x y {press ? 'p' : ''}\" icon=\"gear\"/>\n";
    let out = format(src).expect("recognized");
    assert_eq!(
        out,
        "<container key=\"a\" icon=\"gear\" class=\"x y {press ? 'p' : ''}\"/>\n"
    );
    assert!(out.contains("class=\"x y {press ? 'p' : ''}\""), "{out:?}");
    same_tree(src, &out);
}

#[test]
fn key_moves_to_the_first_position() {
    let src = "<control interactive=\"true\" key=\"a\" class=\"c\"/>\n";
    let out = format(src).expect("recognized");
    assert_eq!(
        out,
        "<control key=\"a\" interactive=\"true\" class=\"c\"/>\n"
    );
    same_tree(src, &out);
}

#[test]
fn both_moves_compose_key_first_class_last() {
    let src = "<control interactive=\"true\" class=\"btn\" key=\"go\"/>\n";
    let out = format(src).expect("recognized");
    assert_eq!(
        out,
        "<control key=\"go\" interactive=\"true\" class=\"btn\"/>\n"
    );
    same_tree(src, &out);
}

#[test]
fn spanning_start_tag_joins_into_one_line() {
    let src = "<TextLabel key=\"blurb\"\n  text=\"{tr('x')}\"\n  level=\"text-body\"/>\n";
    let out = format(src).expect("recognized");
    assert_eq!(
        out,
        "<TextLabel key=\"blurb\" text=\"{tr('x')}\" level=\"text-body\"/>\n"
    );
    same_tree(src, &out);
    // 再跑一遍必须不变（幂等）。
    assert_eq!(format(&out).as_deref(), Some(out.as_str()));
}

#[test]
fn packed_if_element_unpacks_to_three_lines() {
    let src = "<component name=\"P\">\n  <if cond=\"@ref_family == 0\"><PageBasics/></if>\n</component>\n";
    let out = format(src).expect("recognized");
    assert_eq!(
        out,
        "<component name=\"P\">\n  <if cond=\"@ref_family == 0\">\n    <PageBasics/>\n  </if>\n</component>\n",
    );
    same_tree(src, &out);
}

#[test]
fn a_gt_inside_a_value_is_not_the_tag_end() {
    // 属性值里允许裸 `>`（见 xml.rs 的「对 XML 的刻意偏离」）。排版器若拿
    // `find('>')` 定位标签尾，会把 cond 中间那个 `>` 当成结尾。
    let src = "<container key=\"a\" cond=\"@n > 3\" class=\"c\"/>\n";
    let out = format(src).expect("recognized");
    assert!(out.contains("cond=\"@n > 3\""), "{out:?}");
    assert!(
        out.trim_end().ends_with("/>"),
        "tag must stay one line: {out:?}"
    );
    same_tree(src, &out);
}

#[test]
fn single_quoted_values_are_reordered_too() {
    // 值允许用单引号包（属性里要放双引号时），重排不能把引号弄丢。
    let src = "<container class='c' key='a'/>\n";
    let out = format(src).expect("recognized");
    assert_eq!(out, "<container key='a' class='c'/>\n");
    same_tree(src, &out);
}

#[test]
fn indentation_is_derived_from_nesting_depth() {
    // 一眼能看出来的坏缩进要修；深度 = 未闭合开标签数。
    let src = "<component name=\"X\">\n<container key=\"a\">\n<container key=\"b\"/>\n</container>\n</component>\n";
    let out = format(src).expect("recognized");
    assert_eq!(
        out,
        "<component name=\"X\">\n  <container key=\"a\">\n    <container key=\"b\"/>\n  </container>\n</component>\n",
    );
    same_tree(src, &out);
}

#[test]
fn trailing_whitespace_and_tabs_are_cleaned() {
    let src = "<component name=\"X\">   \n\t<container key=\"a\"/>  \n</component>\n";
    let out = format(src).expect("recognized");
    assert_eq!(
        out,
        "<component name=\"X\">\n  <container key=\"a\"/>\n</component>\n"
    );
    for l in out.lines() {
        assert_eq!(l, l.trim_end(), "trailing whitespace leaked: {l:?}");
        assert!(!l.contains('\t'), "tab leaked: {l:?}");
    }
    same_tree(src, &out);
}

#[test]
fn multiline_comments_are_copied_verbatim() {
    // 注释逐字照抄（它可能出现 `<` / `>`），行号也因此稳定。
    let src = "<!--\n  <not-a-tag key=\"x\">\n  a > b\n-->\n<component name=\"X\">\n  <container key=\"a\"/>\n</component>\n";
    let out = format(src).expect("recognized");
    assert!(
        out.contains("<!--\n  <not-a-tag key=\"x\">\n  a > b\n-->"),
        "{out:?}"
    );
    assert!(is_formatted(src));
}

#[test]
fn crlf_is_normalized() {
    let src = "<component name=\"X\">\r\n  <container key=\"a\"/>\r\n</component>\r\n";
    let out = format(src).expect("recognized");
    assert_eq!(
        out,
        "<component name=\"X\">\n  <container key=\"a\"/>\n</component>\n"
    );
    assert!(!out.contains('\r'));
}

#[test]
fn broken_xml_returns_none() {
    // 语法不干净 ⇒ 不猜、不覆盖。
    assert!(format("<container key=\"a\"\n").is_none());
    assert!(format("</component>\n").is_none());
    assert!(format("hello\n").is_none());
    // 属性缺引号。
    assert!(format("<container key=a/>\n").is_none());
}

#[test]
fn is_formatted_flags_unformatted_input() {
    assert!(!is_formatted(
        "<container key=\"a\" class=\"c\" icon=\"gear\"/>\n"
    ));
    assert!(!is_formatted("<container\n  key=\"a\"/>\n"));
    assert!(!is_formatted("<if cond=\"a\"><B/></if>\n"));
    assert!(!is_formatted("<container key=\"a\"/>"));
    assert!(is_formatted("<container key=\"a\"/>\n"));
}

#[test]
fn format_is_idempotent() {
    let src = "<component name=\"X\">\n<container key=\"a\" class=\"c\" icon=\"gear\"/>\n  <if cond=\"@n > 1\"><PageA/></if>\n</component>\n";
    let once = format(src).expect("recognized");
    let twice = format(&once).expect("formatted output must still parse");
    assert_eq!(once, twice);
    same_tree(src, &once);
}

/// 门禁与排版器对齐的**硬**检查：每条规则各给一个坏样例，`fmt` 必须修到
/// 与门禁同一判据下的干净形态。否则「跑完 fmt 门禁还红」。
///
/// 判据与 `style_gate_tests::fmt_fixes_exactly_what_the_gate_checks` 共享
/// 同一份代码（`crate::fmt`），这里只补一条**编译产物等价**的钳制。
#[test]
fn fmt_fixes_exactly_what_the_gate_checks() {
    let bad = "<container key=\"a\" class=\"c\" icon=\"gear\"/>\n";
    let want = "<container key=\"a\" icon=\"gear\" class=\"c\"/>\n";
    let got = format(bad).expect("recognized");
    assert_eq!(got, want);
    assert!(
        crate::fmt::violations(&got).is_empty(),
        "fixed output still violates the gate: {got:?}"
    );
    same_tree(bad, &got);
}

/// 金标准：拿**真文件**验证「只动位置」—— 把同一份文件胡乱排版一遍再排回来，
/// 编译产物必须与原件逐字节相同。
///
/// 想去掉缩进 + 把 `class` 挪到最前面，两者都只动空白与顺序；编译产物里
/// 没有这两样的痕迹，所以任何一处不同都意味着排版器动到了内容。
#[test]
fn repo_files_survive_a_mangle_and_reformat_roundtrip() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zypuic 在工作区根下")
        .to_path_buf();
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    collect(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "一个 .zypui 都没找到");

    for f in &files {
        let src =
            std::fs::read_to_string(f).unwrap_or_else(|e| panic!("读不了 {}: {e}", f.display()));
        // 组件表是**跨文件**的（`shell.zypui` 用 `TextLabel`，定义在
        // `controls.zypui`），所以每个文件都要连同其余文件一起编 —— 与
        // `gallery/build.rs` 的做法一致。基准 = 全部原件的编译产物。
        // （`compile` 要 `&'static str` 文件名；测试里泄露一份就好，与
        // `main.rs::read_files` 同一做法。）
        let all: Vec<(&'static str, &str)> = files
            .iter()
            .map(|p| {
                let n: &'static str = Box::leak(p.display().to_string().into_boxed_str());
                let s: &'static str = Box::leak(
                    std::fs::read_to_string(p)
                        .unwrap_or_else(|e| panic!("读不了 {}: {e}", p.display()))
                        .into_boxed_str(),
                );
                (n, s)
            })
            .collect();
        let base = crate::compile(&all).unwrap_or_else(|d| {
            panic!(
                "{} 编译不过：{:?}",
                f.display(),
                d.first().map(|x| x.msg.clone())
            )
        });

        // 胡乱排版一遍：只去缩进 + 把 class 挪到最前。
        let mangled = mangle(&src);
        let fixed = crate::fmt::format(&mangled)
            .unwrap_or_else(|| panic!("{}: 排版器不认得自己弄乱的源码", f.display()));
        // 同一套文件，只把当前那份换成整理后的版本。
        let mixed: Vec<(&'static str, &str)> = files
            .iter()
            .map(|p| {
                let n: &'static str = Box::leak(p.display().to_string().into_boxed_str());
                let s: &'static str = if p == f {
                    Box::leak(fixed.as_str().to_string().into_boxed_str())
                } else {
                    Box::leak(
                        std::fs::read_to_string(p)
                            .unwrap_or_else(|e| panic!("读不了 {}: {e}", p.display()))
                            .into_boxed_str(),
                    )
                };
                (n, s)
            })
            .collect();
        let round = crate::compile(&mixed).unwrap_or_else(|d| {
            panic!(
                "{}: 排回来之后编译不过：{:?}",
                f.display(),
                d.first().map(|x| x.msg.clone())
            )
        });
        assert_eq!(
            base,
            round,
            "{}: 胡乱排版一遍再排回来，编译产物变了 —— 排版器动到了内容",
            f.display()
        );
        // 且排回来的形态必须与原件逐字节相同（不然每次 fmt 都是新 diff）。
        assert_eq!(fixed, src, "{}: 排回来的形态与原件不同", f.display());
    }
}

fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
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
            collect(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("zypui") {
            out.push(p);
        }
    }
}

/// 只动**位置**地弄乱一份 `.zypui`：去掉每行缩进，并把 `class="…"` 挪到
/// 属性最前面（若该行有 ` key="…"` 则留在 key 之后）。
///
/// 故意不做任何内容改动 —— 这条用例的判据是「只动位置的输入 -> fmt -> 与原件等字节」。
/// 只动**位置**地弄乱一份 `.zypui`：去掉每行缩进，并把 `class="…"` 挪到
/// 标签名之后第一个位置。
///
/// 故意不做任何内容改动 —— 这条用例的判据是「只动位置的输入 → fmt → 与原件等字节」。
///
/// ⚠ 注释行**原样保留**：五条规则本来就豁免注释，而排版器也正是逐字照抄它们，
/// 动注释只会让判据误判。
fn mangle(src: &str) -> String {
    let mut out = String::new();
    let mut in_comment = false;
    for line in src.lines() {
        // 注释（可跨行）原样搬过去。
        if in_comment {
            out.push_str(line);
            out.push('\n');
            if line.contains("-->") {
                in_comment = false;
            }
            continue;
        }
        if line.contains("<!--") {
            out.push_str(line);
            out.push('\n');
            // 行内 `<!--` 多于 `-->` ⇒ 这是跨行注释的开头。
            if line.matches("<!--").count() > line.matches("-->").count() {
                in_comment = true;
            }
            continue;
        }

        let t = line.trim();
        if let Some(a) = t.find(" class=\"") {
            // 值里不可能含自己的引号（解析器遇到即闭合），下一个 `"` 就是结尾。
            if let Some(end) = t[a + 8..].find('"').map(|b| a + 8 + b + 1) {
                let seg = &t[a..end];
                let rest = format!("{}{}", &t[..a], &t[end..]);
                // 插入点 = 标签名的结尾。**不是**「第一个空白」：class 是唯一
                // 属性时 `rest` 直接是 `<container>`，按空白找会落到 `>` 之后，
                // 于是标签被撕成两半（实测：`<container>class="…"`）。
                let name_end = tag_name_end(&rest);
                let rebuilt = format!("{}{}{}", &rest[..name_end], seg, &rest[name_end..]);
                out.push_str(rebuilt.trim());
                out.push('\n');
                continue;
            }
        }
        out.push_str(t);
        out.push('\n');
    }
    out
}

/// `<Name` 那段标签名的结尾下标。
fn tag_name_end(s: &str) -> usize {
    let b = s.as_bytes();
    if b.first() != Some(&b'<') {
        return 0;
    }
    let mut i = 1usize;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'-' | b':' | b'.'))
    {
        i += 1;
    }
    i
}

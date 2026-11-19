//! `fmt` 的单元测试。
//!
//! # 这些用例守什么
//!
//! 排版器承诺的是**「不改语义」**。它靠构造保证（只动空白、值逐字照搬），
//! 测试的责任是把这条承诺**钉住**，并且在有人改坏排版器时当场红：
//!
//! - 已排好的文件必须逐字节不变（否则每次 `fmt` 都是新 diff，CI 永远不干净）；
//! - 值必须逐字保留（单位、`@令牌`、十六进制颜色、`0.00` 与 `0` 的区别）；
//! - 排版前后 `zstylec::parse` 的 IR 必须一致（这不是构造保证能覆盖的那部分：
//!   IR 一样才说明「排版器确实只动了空白」）；
//! - 不认识的输入返回 `None`，而不是猜一份排版。

use crate::fmt::{format, is_formatted};

/// 排版前后 IR 一致 —— 「不改语义」的硬判据。
fn same_ir(before: &str, after: &str) {
    let a = crate::parse("a.zstyle", before);
    let b = crate::parse("b.zstyle", after);
    assert_eq!(
        format!("{:?}", a.sheet),
        format!("{:?}", b.sheet),
        "排版改变了 IR：\n--- before ---\n{before}\n--- after (formatted) ---\n{after}",
    );
    assert_eq!(
        a.errors.len(),
        b.errors.len(),
        "排版改变了诊断数：{:?} vs {:?}",
        a.errors.iter().map(|e| &e.headline).collect::<Vec<_>>(),
        b.errors.iter().map(|e| &e.headline).collect::<Vec<_>>()
    );
}

#[test]
fn already_formatted_stays_byte_identical() {
    // 这条是**全部**其余用例的地基：`fmt` 跑在干净文件上必须是 no-op，
    // 否则每次提交都带一堆 diff，门禁永远不干净。
    let src = "/* 注释 */\ncontrol.button {\n    background: @accent_fill;\n}\n\ncontrol.button.accent {\n    border-color: @accent_fill;\n}\n";
    let out = format(src).expect("canonical source must be recognized");
    assert_eq!(out, src);
    assert!(is_formatted(src));
}

#[test]
fn single_line_rule_expands() {
    let src = "box.page-guide    { direction: vertical; spacing: 24px; cross-align: stretch; }\n";
    let out = format(src).expect("one-line rule must be recognized");
    assert_eq!(
        out,
        "box.page-guide {\n    direction: vertical;\n    spacing: 24px;\n    cross-align: stretch;\n}\n"
    );
    // 选择器里那种对齐空格必须收敛成一个。
    assert!(!out.contains("  {"), "selector padding leaked: {out:?}");
}

#[test]
fn value_text_is_preserved_verbatim() {
    // 单位、`@令牌`、十六进制、`0.00` 与 `0` 的区别、数值列表里的多个空格
    // 一律逐字保留 —— 排版器只动属性名与值**之间**的空格。
    let src = "control.material     {  material-params:  26  18  0  1.5 ; padding: 0.00 ; corner-radius: 8px ; background: @tok ; border-color: #e81123aa ; filter: frosted(2) ; }\n";
    let out = format(src).expect("values must survive formatting");
    assert!(out.contains("material-params: 26 18 0 1.5;"), "{out:?}");
    assert!(out.contains("padding: 0.00;"), "{out:?}");
    assert!(out.contains("corner-radius: 8px;"), "{out:?}");
    assert!(out.contains("background: @tok;"), "{out:?}");
    assert!(out.contains("border-color: #e81123aa;"), "{out:?}");
    assert!(out.contains("filter: frosted(2);"), "{out:?}");
    same_ir(src, &out);
}

#[test]
fn comments_are_copied_verbatim_and_reindented() {
    // 续行的 ` *` 是作者对齐的，排版器只动首行缩进。
    let src = "control {\n    /* 颜色全部走 @令牌 */\n    background: @control_fill;\n}\n\n/* 多行注释\n *\n * 第二行\n */\ncontrol.pressed {\n    border-width: 2px;\n}\n";
    let out = format(src).expect("comments must survive formatting");
    assert_eq!(out, src, "注释必须逐字保留");
    assert!(is_formatted(src));

    // 注释所在层级变了，首行缩进跟着走，续行一字不动。
    let flat = "control{/* c */background: red;}\n";
    let nested = format(flat).expect("nested comment must be recognized");
    assert!(
        nested.starts_with("control {\n    /* c */\n    background: red;\n}\n"),
        "{nested:?}"
    );
}

#[test]
fn blank_lines_are_collapsed_but_not_inserted() {
    // 原本没有空行 ⇒ 不加（注释"贴"着它注释的规则）；原有的空行只留一个。
    let glued = "/* 状态规则 */\ncontrol.button.hovered {\n    border-color: @tok;\n}\n\n\n\ncontrol.button.pressed {\n    border-color: @tok;\n}\n";
    let out = format(glued).expect("gap handling");
    assert_eq!(
        out,
        "/* 状态规则 */\ncontrol.button.hovered {\n    border-color: @tok;\n}\n\ncontrol.button.pressed {\n    border-color: @tok;\n}\n",
    );
}

#[test]
fn crlf_and_trailing_newlines_normalize() {
    let src = "control {\r\n    width: 10px;\r\n}\r\n\r\n\r\n";
    let out = format(src).expect("CRLF must be recognized");
    assert_eq!(out, "control {\n    width: 10px;\n}\n");
    assert!(!out.contains('\r'), "CR leaked out");
    assert!(!is_formatted(src));
}

#[test]
fn formatting_does_not_change_the_ir() {
    let src = "/* demo */\ncontrol {\n  width:40px;   height:40px;\n}\n\n\n\nbox.page-guide { direction:vertical; spacing:24px; cross-align:stretch; }\n";
    let out = format(src).expect("mangled source must still be recognizable");
    same_ir(src, &out);
    // 且第二次必须是 no-op（幂等）—— 不是幂等的话 `--check` 会在格式化后的
    // 文件上继续报警。
    assert_eq!(format(&out).as_deref(), Some(out.as_str()));
}

#[test]
fn unrecognized_shapes_return_none() {
    // 注释没闭合。
    assert!(format("/* never closed\ncontrol {\n  width: 10px;\n}\n").is_none());
    // `}` 比 `{` 多。
    assert!(format("}\ncontrol {\n  width: 10px;\n}\n").is_none());
    // 体没闭合（EOF 时还在 `}` 等）。
    assert!(format("control {\n  width: 10px;\n").is_none());
    // 声明没有 `:`。
    assert!(format("control {\n  width 10px;\n}\n").is_none());
    // 顶层裸声明。
    assert!(format("width: 10px;\n").is_none());
    // 值里出现 `{`（嵌套规则语法不支持，也不该被排版器吞下去）。
    assert!(format("control {\n  background: red {\n}\n").is_none());
}

#[test]
fn is_formatted_flags_unformatted_input() {
    assert!(!is_formatted("control{width:10px;}\n"));
    assert!(!is_formatted("control {\n    width: 10px;\n}"));
    assert!(!is_formatted("control { width: 10px; }\n"));
    assert!(is_formatted("control {\n    width: 10px;\n}\n"));
}

#[test]
fn line_comments_are_not_comments() {
    // 本语言只认 `/* */`（`a_line_comment_is_not_a_comment` 那条测试钉着）。
    // `//` 因此**不是**注释，它后面的 `;` 是结构位。
    let with_slashes = "control {\n    text: //not-a-comment;\n}\n";
    let out = format(with_slashes).expect("slashes in a value are plain text");
    assert!(out.contains("text: //not-a-comment;"), "{out:?}");
    same_ir(with_slashes, &out);
}

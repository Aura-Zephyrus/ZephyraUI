//! 仓内**全部** `*.zstyle` 的排版门禁：「写法统一」靠测试钉住，不靠评审记性。
//!
//! # 为什么值得一条测试
//!
//! 排版规范里唯一「机器能判、人一定会忘」的部分是：已经统一过的写法不许再漂回去。
//! 实测 2026-10-31 存量漂移：19 份文件里 2 份是 CRLF、44 处单行规则
//! （`box.page-guide { direction: vertical; … }`）、选择器与 `{` 之间
//! 有对齐空格（`box.page-guide    {`）。这些都是纯排版差异，语义零变化，
//! 但 diff 里全是噪音。
//!
//! # 门的形状
//!
//! 与 `.zypui` 那条门禁同构：递归扫工作区全部 `*.zstyle`（跳过 `target` / `.git`），
//! 逐份判 `is_formatted`。新文件自动纳入，不需要登记。
//!
//! 另外钉两条**不损耗式**的量（它们不随存量变化而失效，即使存量已经全部干净）：
//!
//! - `fmt` 幂等：`format(format(x)) == format(x)`。不幂等的话 `--check` 会在
//!   格式化后的文件上继续报警，CI 永远不干净。
//! - `fmt` 不改 IR：排版前后 `parse` 的 IR 逐字段相等。括号里的注释解释了
//!   为什么这条重要 —— 它是「只动空白」这条承诺的硬判据。

use std::path::{Path, PathBuf};

/// 工作区根 = `zstylec/` 的父目录。
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zstylec 在工作区根下")
        .to_path_buf()
}

/// 递归收集仓内全部 `.zstyle`，跳过 `target` / `.git` 与隐藏目录。
fn collect_zstyle(dir: &Path, out: &mut Vec<PathBuf>) {
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
            collect_zstyle(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("zstyle") {
            out.push(p);
        }
    }
}

fn all_repo_zstyle() -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_zstyle(&workspace_root(), &mut files);
    files.sort();
    files
}

#[test]
fn repo_zstyle_files_are_formatted() {
    let files = all_repo_zstyle();
    assert!(
        !files.is_empty(),
        "一个 .zstyle 都没找到 —— 工作区根算错了？（{}）",
        workspace_root().display()
    );

    let mut dirty: Vec<String> = Vec::new();
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else {
            dirty.push(format!("{}: 读不了", f.display()));
            continue;
        };
        if crate::fmt::is_formatted(&src) {
            continue;
        }
        let rel = f
            .strip_prefix(workspace_root())
            .unwrap_or(f)
            .display()
            .to_string();
        // `format` 返回 None = 不是本排版器认识的结构（源码坏了）。那更要报 ——
        // 它不会被顺手套上一层排版，所以「以为 fmt 过了」是个假象。
        match crate::fmt::format(&src) {
            Some(out) => dirty.push(format!(
                "{rel}: 要排版（{} 字节 → {} 字节）",
                src.len(),
                out.len()
            )),
            None => dirty.push(format!(
                "{rel}: 读不了结构（源码可能坏了，或用了不支持的语法）"
            )),
        }
    }

    assert!(
        dirty.is_empty(),
        "共 {} 个 .zstyle，{} 份要排版 —— 跑 `zephyra fmt` / `zstylec fmt`：\n{}",
        files.len(),
        dirty.len(),
        dirty.join("\n")
    );
}

/// 全仓 `.zstyle` 必须**零诊断**：类型选择器或定位值一旦用了旧词汇
/// （`Container` / `Text` / `position: absolute` …），这里立刻红。
///
/// 与排版门禁互补：排版器只动空白，**不**跑语义校验；这条才是词汇/枚举值的门禁。
#[test]
fn every_repo_zstyle_parses_without_diagnostics() {
    let files = all_repo_zstyle();
    assert!(
        !files.is_empty(),
        "一个 .zstyle 都没找到 —— 工作区根算错了？（{}）",
        workspace_root().display()
    );

    let mut bad: Vec<String> = Vec::new();
    for f in &files {
        let src =
            std::fs::read_to_string(f).unwrap_or_else(|e| panic!("读不了 {}: {e}", f.display()));
        let report = crate::parse(&f.display().to_string(), &src);
        for e in &report.errors {
            bad.push(format!(
                "{}:{}: {}",
                f.strip_prefix(workspace_root()).unwrap_or(f).display(),
                e.at.line,
                e.headline
            ));
        }
    }

    assert!(
        bad.is_empty(),
        "{} 份 .zstyle 里有 {} 条诊断 —— 旧词汇/旧枚举值必须清零：\n{}",
        files.len(),
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn formatting_is_idempotent_on_every_repo_file() {
    for f in all_repo_zstyle() {
        let src =
            std::fs::read_to_string(&f).unwrap_or_else(|e| panic!("读不了 {}: {e}", f.display()));
        let once = crate::fmt::format(&src)
            .unwrap_or_else(|| panic!("{} 不是排版器认识的结构", f.display()));
        let twice = crate::fmt::format(&once).expect("格式化后的源码必须仍然可认");
        assert_eq!(
            once,
            twice,
            "{}: fmt 不幂等 —— `--check` 会在格式化后的文件上继续报警",
            f.display()
        );
    }
}

#[test]
fn formatting_does_not_change_the_ir_on_any_repo_file() {
    // 这不是构造保证能覆盖的那部分：IR 相等才证明「排版器确实只动了空白」。
    // 存量已经全部干净 ⇒ format(x) == x，这条当时是空转；但任何一次改写
    // 排版器、或新增一份未排版的 .zstyle，它立刻变成真判据。
    for f in all_repo_zstyle() {
        let src =
            std::fs::read_to_string(&f).unwrap_or_else(|e| panic!("读不了 {}: {e}", f.display()));
        let out = crate::fmt::format(&src)
            .unwrap_or_else(|| panic!("{} 不是排版器认识的结构", f.display()));
        let a = crate::parse("a.zstyle", &src);
        let b = crate::parse("b.zstyle", &out);
        assert_eq!(
            format!("{:?}", a.sheet),
            format!("{:?}", b.sheet),
            "{}: 排版改变了 IR",
            f.display()
        );
        assert_eq!(
            a.errors.len(),
            b.errors.len(),
            "{}: 排版改变了诊断数",
            f.display()
        );
    }
}

/// 门禁本身要能抓到违规 —— 拿一份故意写烂的源码验判据，防止门禁形同虚设。
#[test]
fn the_gate_actually_catches_drift() {
    let bad = "control { width:1px; height:2px }\n";
    assert!(
        !crate::fmt::is_formatted(bad),
        "未排版的源码必须被判脏 —— 门禁形同虚设"
    );
    // 单行规则也是违规。
    let one_line = "box.x { width: 1px; }\n";
    assert!(
        !crate::fmt::is_formatted(one_line),
        "单行规则必须被判脏 —— 门禁漏掉了一整类漂移"
    );
    // 干净的必须判干净（否则 `zephyra fmt` 之后 CI 仍然红）。
    let clean = "control {\n    width: 1px;\n    height: 2px;\n}\n";
    assert!(
        crate::fmt::is_formatted(clean),
        "clean source must pass the gate"
    );
}

//! `fmt` —— 聚合三个排版器：Rust 走 `cargo fmt`，`.zstyle` 走 `zstylec::format_style`，
//! `.zypui` 走 `zypuic::format_ui`。
//!
//! # 为什么 CLI 只做「聚合」，不在自己身上再写一份排版逻辑
//!
//! 排版**判据**必须住在编译器里（`zstylec::fmt::violations` /
//! `zypuic::fmt::violations`），CLI 只负责「找到文件、调谁、写不写回、报什么」。
//! 一旦 CLI 自己也判一遍排版，「CLI 说排好了、编译器门禁还红」就有了一席之地 ——
//! 那种分裂极难归因（用户会怀疑其中一边装错了版本）。
//!
//! Rust 那一档是**唯一的例外**：它不归我们管，`cargo fmt` 就是那条路，所以直接
//! spawn 它（与 `run` / `build` spawn cargo 同一形状）。
//!
//! # 用法
//!
//! ```text
//! zephyra fmt            # 扫描整个工程，三者全跑（cargo fmt 也要）
//! zephyra fmt --check    # 只报不写，给 CI 用（有要排版的就退 1）
//! zephyra fmt app.zstyle # 只排指定文件（Rust 那一档跳过）
//! ```
//!
//! # 退出码
//!
//! | 码 | 含义 |
//! |---|---|
//! | 0 | 全部干净 / 已改好 |
//! | 1 | `--check` 下有文件要排版，或某个排版器报了错 |
//! | 2 | 用法错（路径不存在、扩展名不认） |
//! | 3 | 环境缺失（找不到 cargo） |

use super::{cargo_path, spawn_inherit, CliError, Write};
use crate::args::Args;
use crate::ExitCode;
use std::path::{Path, PathBuf};

/// 我们负责排版的扩展名 → 处理方式。
///
/// 未知扩展名**报错**而不是静默跳过：把 `foo.txt` 传给 `fmt` 却什么都没发生，
/// 是「看起来生效了其实没有」那一类失败（见 `Command::flags` 的说明）。
fn ext_kind(p: &Path) -> Option<Kind> {
    match p.extension().and_then(|e| e.to_str()) {
        Some("zstyle") => Some(Kind::Zstyle),
        Some("zypui") => Some(Kind::Zypui),
        Some("rs") => Some(Kind::Rust),
        _ => None,
    }
}

enum Kind {
    Zstyle,
    Zypui,
    Rust,
}

pub fn run(a: &Args, w: &mut dyn Write) -> Result<ExitCode, CliError> {
    let check = a.has("check");
    // 工程根：`--manifest-path` 的父目录，否则当前目录。
    let root = match a.option("manifest-path") {
        Some(m) => {
            let p = PathBuf::from(m);
            let p = if p.is_absolute() { p } else { cwd()?.join(p) };
            p.parent().map(Path::to_path_buf).unwrap_or(p)
        }
        None => cwd()?,
    };

    // Rust 那一档只在「这是个 cargo 工程」时才跑：`cargo fmt` 需要 manifest，
    // 而「这里只有一堆 .zstyle」的目录（比如本仓的 docs/、或临时目录）不该被
    // 一条 `error: could not find Cargo.toml` 打红。
    let rust_project = root.join("Cargo.toml").is_file();

    // 指定了文件就只排它们；否则扫整个工程，并且把 Rust 那一档也带上
    // （`zephyra fmt` 的形状应当与 `cargo fmt` 一致：对整个工程生效）。
    let (targets, with_rust) = if a.positional.is_empty() {
        let mut found = Vec::new();
        for ext in ["zstyle", "zypui"] {
            collect_ext(&root, ext, &mut found);
        }
        (found, rust_project)
    } else {
        let mut t = Vec::new();
        let mut any_rust = false;
        for raw in &a.positional {
            let p = if Path::new(raw).is_absolute() {
                PathBuf::from(raw)
            } else {
                cwd()?.join(raw)
            };
            if p.is_dir() {
                // 目录 ⇒ 扫里面的样式表/结构文件（Rust 由 `cargo fmt` 自己扫）。
                for ext in ["zstyle", "zypui"] {
                    collect_ext(&p, ext, &mut t);
                }
                continue;
            }
            match ext_kind(&p) {
                Some(Kind::Zstyle) | Some(Kind::Zypui) => t.push(p),
                Some(Kind::Rust) => {
                    // 点名了 Rust 文件：交给 `cargo fmt`，跟工程里其余 Rust 文件
                    // 一起排 —— 逐个调 rustfmt 会漏掉「没点名的那些」。
                    any_rust = true;
                }
                None => {
                    return Err(CliError::usage(format!(
                        "cannot format {} (expected .zstyle, .zypui or .rs)",
                        p.display()
                    )));
                }
            }
        }
        (t, any_rust)
    };

    let mut dirty = 0usize;

    for p in &targets {
        let src = std::fs::read_to_string(p)
            .map_err(|e| CliError::usage(format!("cannot read {}: {e}", p.display())))?;
        let out = match ext_kind(p).expect("targets 全部来自 ext_kind 过滤") {
            // 名字是 `format_style` 而不是 `format` —— 后者会跟 std 的 `format!`
            // 宏撞名，cargo doc 会让整个 zstylec 文档不出来。
            Kind::Zstyle => zstylec::format_style(&src),
            Kind::Zypui => zypuic::format_ui(&src),
            // 单文件 Rust 交给 `cargo fmt`（见下）：这里不逐个 rustfmt。
            Kind::Rust => continue,
        };
        match out {
            None => {
                // 编译器不认识的结构 ⇒ 不猜、不覆盖，报错让人先修源码。
                w.err_line(&format!(
                    "error  {}: not recognized by its compiler (broken source?)",
                    p.display()
                ));
                return Ok(ExitCode::Failure);
            }
            Some(o) if o == src => w.line(&format!("ok  {}", p.display())),
            Some(o) => {
                dirty += 1;
                if check {
                    w.line(&format!("would format  {}", p.display()));
                } else {
                    std::fs::write(p, &o).map_err(|e| {
                        CliError::failure(format!("cannot write {}: {e}", p.display()))
                    })?;
                    w.line(&format!("formatted  {}", p.display()));
                }
            }
        }
    }

    // Rust 那一档：跑 `cargo fmt --all`（`--check` 透传）。只在「扫整个工程」
    // 或用户点名了 .rs 文件时跑 —— 只排样式表时不该顺带动 Rust。
    if with_rust {
        let cargo = cargo_path()?;
        let mut args = vec!["fmt", "--all"];
        if check {
            args.push("--check");
        }
        // cargo fmt 自身的问题（装没装 nightly 等）：`?` 传播成退 3，
        // 而不是伪装成 0/1。`--check` 下要排 Rust 时它退 1 —— 与 DSL 那一档
        // 同一形状，所以并进同一个计数，最后统一退 1。
        let code = spawn_inherit(&cargo, &args, None, None)?;
        if code == ExitCode::Failure {
            dirty += 1;
        }
    }

    if check && dirty > 0 {
        w.err_line(&format!("\n{dirty} file(s) would be formatted"));
        return Ok(ExitCode::Failure);
    }
    Ok(ExitCode::Success)
}

fn cwd() -> Result<PathBuf, CliError> {
    std::env::current_dir().map_err(|e| CliError::failure(format!("cannot get cwd: {e}")))
}

/// 递归收集某个扩展名的文件，跳过 `target` / `.git` / 隐藏目录 —— 与两个编译器
/// 的门禁扫描同一形状（扫到的集合一致，`zephyra fmt` 之后门禁才真会绿）。
fn collect_ext(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
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
            collect_ext(&p, ext, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some(ext) {
            out.push(p);
        }
    }
    out.sort();
}

//! `zypuic` 命令行入口。
//!
//! # 用法
//!
//! ```text
//! zypuic check <file.zypui>...       # 检查（默认子命令）
//! zypuic compile <out.bin> <file...> # 编译成二进制
//! zypuic --help / --version
//! ```
//!
//! # 退出码（CI 依赖，与 zstylec 同一约定，别改）
//!
//! | 码 | 含义 |
//! |---|---|
//! | 0 | 全部文件干净（**警告不算脏**） |
//! | 1 | 至少一个文件有**错误** |
//! | 2 | 用法错、或文件读不了 |

use std::process::ExitCode;

const USAGE: &str = "\
zypuic -- ZephyraUI UI-file compiler

USAGE
    zypuic check <file.zypui>...        check one or more UI files
    zypuic compile <out.bin> <file...>  compile into a binary artifact
    zypuic --help                       this text
    zypuic --version

EXIT CODES
    0  all files clean (warnings do not count as dirty)
    1  at least one file has an error
    2  bad usage, or a file could not be read
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    }
    match args[0].as_str() {
        "--version" => {
            println!("zypuic {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "--help" | "-h" => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        "check" => cmd_check(&args[1..]),
        "compile" => cmd_compile(&args[1..]),
        other => {
            eprintln!("未知子命令 `{other}`\n");
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// 读文件列表，失败（用法错）返回 None 并打印退出码 2 的原因。
fn read_files(args: &[String]) -> Option<Vec<(&'static str, String)>> {
    if args.is_empty() {
        eprint!("{USAGE}");
        return None;
    }
    let mut files = Vec::with_capacity(args.len());
    for a in args {
        match std::fs::read_to_string(a) {
            Ok(src) => {
                let name: &'static str = Box::leak(a.clone().into_boxed_str());
                files.push((name, src));
            }
            Err(e) => {
                eprintln!("读不了 {a}：{e}");
                return None;
            }
        }
    }
    Some(files)
}

fn cmd_check(args: &[String]) -> ExitCode {
    let Some(files) = read_files(args) else {
        return ExitCode::from(2);
    };
    let refs: Vec<(&'static str, &str)> = files.iter().map(|(n, s)| (*n, s.as_str())).collect();
    match zypuic::parse_all(&refs) {
        Ok(_) => {
            println!("{} 个文件全部干净", files.len());
            ExitCode::SUCCESS
        }
        Err(diags) => {
            for d in &diags {
                eprintln!("{}", d.render());
            }
            eprintln!("共 {} 条诊断", diags.len());
            ExitCode::from(1)
        }
    }
}

fn cmd_compile(args: &[String]) -> ExitCode {
    let Some(out) = args.first() else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    let Some(files) = read_files(&args[1..]) else {
        return ExitCode::from(2);
    };
    let refs: Vec<(&'static str, &str)> = files.iter().map(|(n, s)| (*n, s.as_str())).collect();
    match zypuic::compile(&refs) {
        Ok(bytes) => match std::fs::write(out, &bytes) {
            Ok(()) => {
                println!("{} ← {} 字节", out, bytes.len());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("写不了 {out}：{e}");
                ExitCode::from(2)
            }
        },
        Err(diags) => {
            for d in &diags {
                eprintln!("{}", d.render());
            }
            eprintln!("共 {} 条诊断", diags.len());
            ExitCode::from(1)
        }
    }
}

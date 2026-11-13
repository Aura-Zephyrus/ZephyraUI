//! 子命令表。`--help` 与执行都查这一张表，所以两者不可能不同步。
//!
//! # 命令的取舍
//!
//! 参考 Tauri CLI 的形状（`init/dev/build/add/info` 那套），但只保留对 ZephyraUI
//! 真的有意义的：
//!
//! | 命令 | 干什么 | 为什么要 |
//! |---|---|---|
//! | `new` | 生成一个可跑的空工程 | 「上手成本」是一切的前置 |
//! | `check` | 查样式表 | 唯一能在不启动 GUI 的情况下抓错的一步，必须能进 CI |
//! | `fmt` | 排版（`.zstyle` / `.zypui` / Rust） | 写法统一靠命令不靠评审；判据在编译器里，CLI 只聚合 |
//! | `run` | 跑起来 | |
//! | `build` | 构建 | |
//! | `info` | 报版本与工具链状态 | 出问题时第一句要问的就是「你装的什么版本」 |
//! | `props` | 列受支持的属性 | 与 `zstylec props` 同源，不制造第二条路径 |
//! | `mcp` | MCP server：AI 直连运行中的界面 | 「AI 会用工具」是当代的必备形态；桥协议在 `zephyra-fx::bridge` |
//!
//! 没有 `migrate` / `sign` / `updater` —— 那些对应 Tauri 的分发体系，ZephyraUI
//! 现在没有，加进来就是空壳。

use crate::args::Args;
use crate::ExitCode;

pub struct Command {
    pub name: &'static str,
    pub about: &'static str,
    /// 位置参数说明。
    pub args: &'static str,
    /// 这个命令**认**的布尔标志。
    ///
    /// ⚠ 这张表存在的唯一理由是「未知标志要报错」。
    /// 实测踩过：`zephyra check --strictness` 曾经安静通过、exit 0，看起来
    /// 像是「开了严格模式跑过了」，其实那个标志根本没生效 —— 而用户以为自己
    /// 已经收紧了检查。这类静默忽略比报错糟得多。
    pub flags: &'static [&'static str],
    pub run: fn(&Args, &mut dyn Write) -> Result<ExitCode, CliError>,
}

/// 统一的输出端。抽出来是为了**能测**：命令往 stdout/stderr 写东西时，
/// 把它接到内存里就不必起子进程去验。
pub trait Write {
    fn line(&mut self, s: &str);
    fn err_line(&mut self, s: &str);
}

pub struct Io<'a> {
    pub out: &'a mut dyn std::io::Write,
    pub err: &'a mut dyn std::io::Write,
}

impl Write for Io<'_> {
    fn line(&mut self, s: &str) {
        let _ = writeln!(self.out, "{s}");
    }
    fn err_line(&mut self, s: &str) {
        let _ = writeln!(self.err, "{s}");
    }
}

/// 命令级错误。带退出码，所以「用法错」和「环境缺」不会混成一种。
#[derive(Debug)]
pub struct CliError {
    pub message: String,
    pub code: ExitCode,
}

impl CliError {
    pub fn usage(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            code: ExitCode::Usage,
        }
    }
    pub fn missing(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            code: ExitCode::MissingEnv,
        }
    }
    pub fn failure(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            code: ExitCode::Failure,
        }
    }
}

pub const COMMANDS: &[Command] = &[
    Command {
        name: "new",
        about: "scaffold a new ZephyraUI project",
        args: "<name> [--path <dir>]",
        flags: &["force"],
        run: new::run,
    },
    Command {
        name: "check",
        about: "check one or more stylesheets",
        args: "<file.zstyle>... [--format human|json]",
        flags: &[],
        run: check::run,
    },
    Command {
        name: "fmt",
        about: "format project sources: .zstyle / .zypui / Rust (via cargo fmt)",
        args: "[<path>...] [--check] [--manifest-path <p>]",
        flags: &["check"],
        run: fmt::run,
    },
    Command {
        name: "dev",
        about: "run in dev mode: tracing on, restarts when a stylesheet changes",
        args: "[--release] [--styles <dir>] [--no-watch] [--no-trace] [--manifest-path <p>] [--log <file>]",
        flags: &["release", "no-watch", "no-trace"],
        run: dev::run,
    },
    Command {
        name: "run",
        about: "run a ZephyraUI app (debug)",
        args: "[--release] [--manifest-path <p>] [--log <file>]",
        flags: &["release"],
        run: run::run,
    },
    Command {
        name: "build",
        about: "build a ZephyraUI app",
        args: "[--release] [--manifest-path <p>] [--log <file>]",
        flags: &["release"],
        run: build::build,
    },
    Command {
        name: "info",
        about: "print versions and toolchain status",
        args: "",
        flags: &[],
        run: info::run,
    },
    Command {
        name: "props",
        about: "list every supported stylesheet property",
        args: "",
        flags: &[],
        run: props::run,
    },
    Command {
        name: "mcp",
        about: "MCP server: let AI agents inspect and drive a running ZephyraUI app",
        args: "[--port <n>] [--host <addr>]",
        flags: &[],
        run: mcp::run,
    },
    Command {
        name: "bridge",
        about: "talk to a running app's debug bridge directly (one process, many commands)",
        args: "[--port <n>] [--host <addr>] [--stdin] [--list] <command>... | log <file> [--tail <n>]",
        flags: &["stdin", "list"],
        run: bridge::run,
    },
    Command {
        name: "perf",
        about: "performance monitor for a running app (profile / watch)",
        args: "profile|watch [--port <n>] [--host <addr>] [--seconds <n>] [--interval-ms <n>] [--duration-ms <n>]",
        flags: &[],
        run: perf::run,
    },
    Command {
        name: "debug",
        about: "debugger for a running app (watch / tree / snapshot / diff)",
        args: "watch|tree|snapshot|diff [--port <n>] [--host <addr>] [--interval-ms <n>] [--duration-ms <n>] [--depth <n>] [--all] <args>...",
        flags: &["all"],
        run: debug::run,
    },
];

pub fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// 渲染帮助。**从表里生成**，所以「帮助里有的」一定「真能跑」。
pub fn render_help(bin: &str, version: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("{bin} {version}\nZephyraUI command line interface\n\nUSAGE\n    {bin} <command> [options]\n\nCOMMANDS\n"));
    let w = COMMANDS.iter().map(|c| c.name.len()).max().unwrap_or(0);
    for c in COMMANDS {
        s.push_str(&format!("    {:w$}    {}\n", c.name, c.about, w = w));
    }
    s.push_str("\nOPTIONS\n");
    s.push_str("    --help, -h        this text\n");
    s.push_str("    --version, -V     print version\n");
    s.push('\n');
    for c in COMMANDS {
        if !c.args.is_empty() {
            s.push_str(&format!("USAGE\n    {bin} {} {}\n\n", c.name, c.args));
        }
    }
    s.push_str("EXIT CODES\n");
    s.push_str("    0  success\n");
    s.push_str("    1  the action failed (check found errors, build failed, ...)\n");
    s.push_str("    2  bad usage (unknown command, missing argument)\n");
    s.push_str("    3  environment is missing (no cargo, directory already exists)\n");
    s
}

mod bridge;
mod build;
mod check;
mod debug;
mod dev;
mod fmt;
mod info;
mod mcp;
mod new;
mod perf;
mod props;
mod run;

/// 找 cargo。没有它时给**明确**的缺环境错误，而不是让调用方看到一条奇怪的 spawn 失败。
pub fn cargo_path() -> Result<std::path::PathBuf, CliError> {
    // 先看 CARGO（cargo 跑我们时设的），再看 PATH 里的 cargo。
    if let Ok(p) = std::env::var("CARGO") {
        let pb = std::path::PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
    }
    which_cargo().ok_or_else(|| {
        CliError::missing(
            "cargo not found. Install Rust from https://rustup.rs and make sure `cargo` is on PATH.",
        )
    })
}

pub fn app_config_path(a: &Args) -> Result<Option<std::path::PathBuf>, CliError> {
    let root = match a.option("manifest-path") {
        Some(manifest) => {
            let manifest = std::path::PathBuf::from(manifest);
            let manifest = if manifest.is_absolute() {
                manifest
            } else {
                std::env::current_dir()
                    .map_err(|e| CliError::failure(format!("cannot get current directory: {e}")))?
                    .join(manifest)
            };
            manifest
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or(manifest)
        }
        None => std::env::current_dir()
            .map_err(|e| CliError::failure(format!("cannot get current directory: {e}")))?,
    };
    let config = root.join("ZephyraUI.json");
    if !config.is_file() {
        return Ok(None);
    }
    std::fs::canonicalize(&config)
        .map(Some)
        .map_err(|e| CliError::failure(format!("cannot resolve {}: {e}", config.display())))
}

fn which_cargo() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let c = dir.join("cargo.exe");
        if c.exists() {
            return Some(c);
        }
        let c = dir.join("cargo");
        if c.exists() {
            return Some(c);
        }
    }
    None
}

/// 起一个子进程并把它的输出**继承**出去（直接透传，不缓冲）。
///
/// `log` 非空时额外把子进程的 stdout / stderr **落一份**到那个文件。
///
/// # 为什么落盘这一档要走管道
///
/// `status()` 让子进程直接继承本进程的 stdout/stderr —— 那是零拷贝、零延迟的，
/// 但**没法**再抄一份到文件。所以落盘这一档必须改成 `piped()` + 转发。
///
/// ⚠ 转发按**字节块**做（不是按行）：cargo 的进度输出用 `\r` 结尾而不带 `\n`，
/// 按行读会把「编译中 12%」这类进度憋到整行结束才吐出来 —— 那正是本函数
/// 的注释里说的「看起来半天才出来」。
///
/// ⚠ stdout 与 stderr 写进**同一个**文件，两股流可能在块边界处交错。
/// 这是刻意的：应用与 cargo 的诊断都在 stderr 里，分成两个文件反而要人自己对。
pub fn spawn_inherit<S: AsRef<std::ffi::OsStr>>(
    program: &std::path::Path,
    args: &[S],
    config_path: Option<&std::path::Path>,
    log: Option<&std::path::Path>,
) -> Result<ExitCode, CliError> {
    let Some(log_path) = log else {
        let mut command = std::process::Command::new(program);
        command.args(args);
        if let Some(path) = config_path {
            command.env("ZEPHYRA_APP_CONFIG_PATH", path);
        }
        let status = command
            .status()
            .map_err(|e| CliError::missing(format!("cannot run {}: {e}", program.display())))?;
        return Ok(if status.success() {
            ExitCode::Success
        } else {
            ExitCode::Failure
        });
    };

    let mut child = spawn_pumped(program, args, config_path, Some(log_path))?;
    let status = child
        .wait()
        .map_err(|e| CliError::failure(format!("cannot wait for {}: {e}", program.display())))?;
    Ok(if status.success() {
        ExitCode::Success
    } else {
        ExitCode::Failure
    })
}

/// 一个**输出被转发**的子进程。
///
/// 「转发」= 子进程的 stdout / stderr 走管道，另开线程写到终端（以及可选的
/// 日志文件）。为什么要有这一档：`status()` 那种「直接继承」没法抄一份到
/// 文件，而 `--log` 的全部价值就在于那份副本。
pub struct PumpedChild {
    child: std::process::Child,
    pumps: Vec<std::thread::JoinHandle<()>>,
}

impl PumpedChild {
    /// 子进程是否已经结束（不阻塞）。
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    /// 结束子进程并**等转发线程收尾**。
    ///
    /// ⚠ 必须等转发线程：不等的话，子进程写完的最后一段（常常正是崩溃那一行）
    /// 会在父进程退出时被丢掉 —— 而那正是要留证据的地方。
    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let status = self.child.wait();
        for pump in self.pumps.drain(..) {
            let _ = pump.join();
        }
        status
    }

    /// 杀掉子进程（不join 转发线程 —— 调用方接着会 `wait`）。
    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

/// 起一个子进程，stdout/stderr 走管道并被转发。
///
/// `log` 非空时转发目标多一个文件（追加写）。
pub fn spawn_pumped<S: AsRef<std::ffi::OsStr>>(
    program: &std::path::Path,
    args: &[S],
    config_path: Option<&std::path::Path>,
    log: Option<&std::path::Path>,
) -> Result<PumpedChild, CliError> {
    let file = match log {
        Some(path) => {
            let f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| {
                    CliError::failure(format!("cannot open log {}: {e}", path.display()))
                })?;
            Some(std::sync::Arc::new(std::sync::Mutex::new(f)))
        }
        None => None,
    };

    let mut command = std::process::Command::new(program);
    command.args(args);
    if let Some(path) = config_path {
        command.env("ZEPHYRA_APP_CONFIG_PATH", path);
    }
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| CliError::missing(format!("cannot run {}: {e}", program.display())))?;

    let mut pumps = Vec::new();
    if let Some(out) = child.stdout.take() {
        pumps.push(pump(out, std::io::stdout(), file.clone()));
    }
    if let Some(err) = child.stderr.take() {
        pumps.push(pump(err, std::io::stderr(), file));
    }
    Ok(PumpedChild { child, pumps })
}

/// 把一条流**同时**写到终端与日志文件。
fn pump<R: std::io::Read + Send + 'static>(
    reader: R,
    mut terminal: impl std::io::Write + Send + 'static,
    log: Option<std::sync::Arc<std::sync::Mutex<std::fs::File>>>,
) -> std::thread::JoinHandle<()> {
    use std::io::Read;
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::with_capacity(8 * 1024, reader);
        let mut buf = [0u8; 8 * 1024];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let _ = terminal.write_all(&buf[..n]);
            let _ = terminal.flush();
            if let Some(log) = &log {
                if let Ok(mut file) = log.lock() {
                    use std::io::Write;
                    let _ = file.write_all(&buf[..n]);
                    let _ = file.flush();
                }
            }
        }
    })
}

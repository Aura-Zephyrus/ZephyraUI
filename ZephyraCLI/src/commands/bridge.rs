//! `zephyra bridge`：直接对运行中的应用调试桥下命令。
//!
//! # 为什么要有这个入口（`mcp` 已经有了）
//!
//! `mcp` 是给 AI 用的：JSON-RPC over stdio，一条工具一个进程。**人**（和脚本）
//! 要的是「一条命令跑一件事，别为了看一眼 stats 起一个 MCP server」。
//!
//! 实测踩过的坑：本轮排查里调了上百次桥，每次都是「起一个 `zephyra.exe mcp`
//! 进程 → JSON-RPC 握手 → TCP 连接 → 发一条 → 断开」。开销全在进程与握手，
//! 不在桥本身。而「一次调用里连着做几件事」（点击 → 等两帧 → 截图）是排查
//! 动画类问题的标准动作，逐条起进程做既慢又容易在中间插进别的输入。
//!
//! 所以这里给桥一个**一等公民入口**：
//!
//! ```text
//! zephyra bridge "click_key switch:solid" "wait_frames 2" "stats"
//! zephyra bridge --stdin            # 每行一条命令（脚本 / REPL）
//! zephyra bridge --list             # 列出可达的实例
//! ```
//!
//! 协议**只有一份**：`zephyra-fx::bridge` 的行文本协议，`mcp` 子命令也用它
//! （只是外面套了 JSON-RPC）。

use crate::args::Args;
use crate::bridge_client::{BridgeClient, BridgeResponse};
use crate::commands::{CliError, Write};
use crate::ExitCode;

use std::io::BufRead;

/// 应用侧桥的默认端口。与 `zephyra-fx::bridge::DEFAULT_PORT` 一致
/// （CLI 刻意不依赖 zephyra-fx；两边各写一份，由测试钉住）。
fn default_port() -> u16 {
    7878
}

/// 端口扫描范围（`--list` 用）。
const SCAN_PORTS: std::ops::RangeInclusive<u16> = 7878..=7890;

pub fn run(args: &Args, io: &mut dyn Write) -> Result<ExitCode, CliError> {
    let host = args.option("host").unwrap_or("127.0.0.1").to_string();
    let port: u16 = args
        .option("port")
        .map(|p| p.parse())
        .transpose()
        .map_err(|_| CliError::usage("--port needs a number"))?
        .unwrap_or_else(default_port);

    if args.has("list") {
        return list_instances(&host, io);
    }

    // `zephyra bridge log <file>`：**本地**读日志，不需要连应用。
    //
    // 放在 `bridge` 下面是因为它服务的是同一个排查场景（应用跑着，你看它的
    // 输出）—— 而 `run --log` / `dev --log` 落下来的那份正是给这里看的。
    if args.positional.first().map(String::as_str) == Some("log") {
        return log_tail(args, io);
    }

    let mut client = BridgeClient::connect(&host, port, 3).map_err(|e| {
        CliError::failure(format!(
            "连不上应用调试桥 {host}:{port}（{e}）——确认应用以 ZEPHYRA_DEBUG_PORT={port} 启动"
        ))
    })?;

    // `--stdin`：每行一条命令。空行忽略；`#` 开头是注释（脚本里好写）。
    if args.has("stdin") {
        let stdin = std::io::stdin();
        let mut failed = false;
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let cmd = line.trim();
            if cmd.is_empty() || cmd.starts_with('#') {
                continue;
            }
            if !dispatch(&mut client, cmd, io)? {
                failed = true;
            }
        }
        return Ok(if failed {
            ExitCode::Failure
        } else {
            ExitCode::Success
        });
    }

    // 位置参数：每一条都是一条桥命令。
    let commands: Vec<&str> = args.positional.iter().map(|s| s.as_str()).collect();
    if commands.is_empty() {
        return Err(CliError::usage(
            "bridge 需要至少一条命令，如 `zephyra bridge \"stats\"`；\
             或用 --stdin 每行一条",
        ));
    }
    let mut failed = false;
    for cmd in commands {
        if !dispatch(&mut client, cmd, io)? {
            failed = true;
        }
    }
    Ok(if failed {
        ExitCode::Failure
    } else {
        ExitCode::Success
    })
}

/// 发一条命令并打印响应。返回 `false` = 这条命令失败（应用侧回 `ERR`）。
fn dispatch(client: &mut BridgeClient, cmd: &str, io: &mut dyn Write) -> Result<bool, CliError> {
    // 客户端侧的选项（`--out` / `--rect` / `--scale`）要**摘掉**再发出去：
    // 应用只回字节，落盘与裁剪都是客户端的事。
    //
    // ⚠ 摘的时候必须**逐字节保留其余部分**：按空白切分再拼回去会把引号里的
    // **连续空格**压成一个 —— `type_text "a  b"` 变成 `type_text "a b"`，
    // 输入框里就少了一个空格。
    let (wire_cmd, out, rect, scale) = strip_client_options(cmd);
    let response = client
        .call(&wire_cmd)
        .map_err(|e| CliError::failure(format!("{cmd}: {e}")))?;
    match response {
        BridgeResponse::Text(lines) => {
            for line in lines {
                io.line(&line);
            }
            Ok(true)
        }
        BridgeResponse::Err(e) => {
            io.err_line(&format!("{cmd}: {e}"));
            Ok(false)
        }
        BridgeResponse::Binary { data, .. } => {
            // `shot` 的二进制载荷：落盘。路径由命令自己带（`shot --out x.bmp`）；
            // 没带就写临时文件并把路径打出来 —— **不丢数据**（丢掉的截图会让人
            // 以为「截图没拍到」，其实是没写出来）。
            //
            // 扩展名 `.png` ⇒ 转 PNG（见 `crate::shot`）；其余原样 BMP。
            let data = match rect {
                Some((x, y, w, h)) => crate::shot::crop_bmp(&data, x, y, w, h, scale)
                    .map_err(|e| CliError::failure(format!("{cmd}: {e}")))?,
                None => data,
            };
            let path = out.unwrap_or_else(default_shot_path);
            crate::shot::write_shot(&path, &data)
                .map_err(|e| CliError::failure(format!("写截图 {} 失败：{e}", path.display())))?;
            io.line(&format!("wrote {}", path.display()));
            Ok(true)
        }
    }
}

/// 一块矩形（`x, y, w, h`，**自顶向下**坐标）。
type Rect = (u32, u32, u32, u32);

/// 摘掉截图的**客户端侧**选项：`--out <p>` / `--path <p>`（落盘路径）、
/// `--rect x,y,w,h`（裁一块）、`--scale N`（放大几倍）。
///
/// 其余部分**逐字节保留** —— 见 [`dispatch`] 里关于连续空格的那条。
///
/// # 为什么裁剪在客户端
///
/// 宿主手里只有「整张图」；要它只读回一块，得改 GPU 侧的拷贝区与渲染目标
/// 尺寸（两个后端各一份）。而客户端**已经有整张图**了，裁一块只是搬字节。
///
/// # 为什么放大用整数倍
///
/// 最近邻（复制像素）不改像素值。插值会造出原图没有的颜色 —— 而「这块有没有
/// 脏东西」正是要看的。
fn strip_client_options(cmd: &str) -> (String, Option<std::path::PathBuf>, Option<Rect>, u32) {
    let words = words_with_spans(cmd);
    let is_shot = words
        .first()
        .map(|(s, e)| &cmd[*s..*e] == "shot")
        .unwrap_or(false);

    let mut drop: Vec<(usize, usize)> = Vec::new();
    let mut out_path: Option<std::path::PathBuf> = None;
    let mut rect: Option<Rect> = None;
    let mut scale: u32 = 1;

    let mut i = 0;
    while i < words.len() {
        if is_shot && i + 1 < words.len() {
            let w = &cmd[words[i].0..words[i].1];
            let (vs, ve) = words[i + 1];
            let value = &cmd[vs..ve];
            let matched = if w == "--out" || w == "--path" {
                out_path = Some(std::path::PathBuf::from(value));
                true
            } else if w == "--rect" {
                match parse_rect(value) {
                    Some(r) => {
                        rect = Some(r);
                        true
                    }
                    None => false,
                }
            } else if w == "--scale" {
                match value.parse::<u32>() {
                    Ok(n) => {
                        scale = n.clamp(1, 16);
                        true
                    }
                    Err(_) => false,
                }
            } else {
                false
            };
            if matched {
                drop.push(words[i]);
                drop.push(words[i + 1]);
                i += 2;
                continue;
            }
        }
        i += 1;
    }

    // 剩下的词按**原样**（含引号内的连续空格）用单空格拼回去。
    //
    // ⚠ 不能直接删区间再 compress：那样引号外的空白会留下一串空格（应用侧
    // 无所谓，但回执里会很难看），而引号内的空白又必须原样保留 ——
    // 所以按「词」重建：一个词是一个整体，词**之间**用单空格，词**内部**不动。
    let kept: Vec<&str> = words
        .iter()
        .filter(|(s, e)| !drop.contains(&(*s, *e)))
        .map(|(s, e)| &cmd[*s..*e])
        .collect();
    (kept.join(" "), out_path, rect, scale)
}

/// 把一行拆成「词」及其**字节区间**。引号内的空白**不算**分隔符。
fn words_with_spans(s: &str) -> Vec<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let mut in_quotes = false;
        while i < bytes.len() {
            let c = bytes[i];
            if in_quotes {
                if c == b'\\' {
                    // 转义：跳过被转义的那一个字节。
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if c == b'"' {
                    in_quotes = false;
                }
                i += 1;
                continue;
            }
            if c == b'"' {
                in_quotes = true;
                i += 1;
                continue;
            }
            if c.is_ascii_whitespace() {
                break;
            }
            i += 1;
        }
        out.push((start, i));
    }
    out
}

/// `x,y,w,h` → 四个无符号整数。
fn parse_rect(value: &str) -> Option<Rect> {
    let mut it = value.split(',');
    let x = it.next()?.parse().ok()?;
    let y = it.next()?.parse().ok()?;
    let w = it.next()?.parse().ok()?;
    let h = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((x, y, w, h))
}

/// 把 `--out <path>` / `--path <path>` 从**截图**命令里摘出来，
/// 返回 `(发给应用的命令, 落盘路径)`。
///
/// ⚠ 只对 `shot` 生效：别的命令里出现 `--out` 可能是**内容**
/// （`type_text "--out"` 要把那三个字打进输入框）。无差别地摘会静默改写
/// 用户要输入的内容 —— 那是最难发现的一类 bug。
#[cfg(test)]
fn split_out_path(cmd: &str) -> (String, Option<std::path::PathBuf>) {
    let (text, out, _, _) = strip_client_options(cmd);
    (text, out)
}

/// 截图没指定路径时的默认落点（临时目录）。
fn default_shot_path() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("zephyra-shot-{}.bmp", std::process::id()));
    path
}

/// `zephyra bridge log <file> [--tail N]`：打印日志文件的**末尾**若干行。
///
/// # 为什么读「末尾」而不是全文
///
/// 日志是**长**的（一次 dev 会话几 MB 很常见），而排查时要看的是「刚才
/// 发生了什么」—— 末尾那几十行。所以只读文件**尾部**一段（上限 1 MiB），
/// 不把整个文件读进内存。
///
/// # 为什么不需要连上应用
///
/// 日志是本地文件。把它挂在 `bridge` 下面只是因为它属于同一个排查场景，
/// 不是因为它需要桥。
fn log_tail(args: &Args, io: &mut dyn Write) -> Result<ExitCode, CliError> {
    let Some(path) = args.positional.get(1) else {
        return Err(CliError::usage(
            "bridge log 需要一个日志文件路径，如 `zephyra bridge log app.log`",
        ));
    };
    let tail: usize = match args.option("tail") {
        Some(v) => v
            .parse()
            .map_err(|_| CliError::usage("--tail needs a number"))?,
        None => 40,
    };
    let lines = tail_lines(std::path::Path::new(path), tail).map_err(CliError::failure)?;
    for line in lines {
        io.line(&line);
    }
    Ok(ExitCode::Success)
}

/// 取文件末尾 N 行。
///
/// 只读尾部 [`TAIL_WINDOW`] 字节：日志可能很大，而「末尾 N 行」几乎总在
/// 那个窗口里。窗口起点落在**行中间**时丢掉那一行 —— 半个行号比没有行号更糟。
const TAIL_WINDOW: u64 = 1024 * 1024;

fn tail_lines(path: &std::path::Path, count: usize) -> Result<Vec<String>, String> {
    use std::io::{Read, Seek, SeekFrom};

    let file = std::fs::File::open(path).map_err(|e| format!("读 {} 失败：{e}", path.display()))?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let start = len.saturating_sub(TAIL_WINDOW);
    let mut reader = std::io::BufReader::new(file);
    reader
        .seek(SeekFrom::Start(start))
        .map_err(|e| format!("定位 {} 失败：{e}", path.display()))?;
    let mut buf = Vec::new();
    reader
        .read_to_end(&mut buf)
        .map_err(|e| format!("读 {} 失败：{e}", path.display()))?;

    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if start > 0 {
        // 窗口起点在行中间 → 丢掉第一行（它只剩后半截）。
        if let Some(idx) = text.find('\n') {
            text = text[idx + 1..].to_string();
        } else {
            text.clear();
        }
    }
    let all: Vec<&str> = text.lines().collect();
    let skip = all.len().saturating_sub(count);
    Ok(all[skip..].iter().map(|s| s.to_string()).collect())
}

/// `--list`：扫一段端口，把可达的实例列出来。
///
/// 多实例时（开发时常见：旧构建还开着、新构建又起了一个）**认人**比试端口
/// 重要 —— 所以每条都带 `ping` 的回答（应用标题 + 进程号）。
fn list_instances(host: &str, io: &mut dyn Write) -> Result<ExitCode, CliError> {
    let mut found = 0;
    for port in SCAN_PORTS {
        let Ok(mut client) = BridgeClient::connect(host, port, 1) else {
            continue;
        };
        let Ok(resp) = client.call("ping") else {
            continue;
        };
        let text = match resp {
            BridgeResponse::Text(lines) => lines.join(" "),
            BridgeResponse::Err(e) => format!("ERR {e}"),
            BridgeResponse::Binary { .. } => "(binary)".into(),
        };
        io.line(&format!("{host}:{port}  {text}"));
        found += 1;
    }
    if found == 0 {
        io.line(&format!(
            "没有可达的实例（扫过 {}..={}）——确认应用以 ZEPHYRA_DEBUG_PORT=<port> 启动",
            SCAN_PORTS.start(),
            SCAN_PORTS.end()
        ));
    }
    Ok(ExitCode::Success)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--out <path>` 是**客户端侧**选项：必须从发给应用的命令里摘掉。
    ///
    /// ⚠ 漏摘的后果是应用侧报「未知参数」——而用户传的是完全合理的参数，
    /// 错误信息指向的方向（协议）离真正的原因（客户端多嘴了）十万八千里。
    #[test]
    fn out_path_is_stripped_from_the_wire_command() {
        let (cmd, out) = split_out_path("shot --out C:/tmp/a.bmp");
        assert_eq!(cmd, "shot");
        assert_eq!(out.unwrap().to_string_lossy(), "C:/tmp/a.bmp");

        // `--path` 是同义词。
        let (cmd, out) = split_out_path("shot --path C:/tmp/a.bmp");
        assert_eq!(cmd, "shot");
        assert!(out.is_some());

        // 其它命令不受影响。
        let (cmd, out) = split_out_path("stats");
        assert_eq!(cmd, "stats");
        assert!(out.is_none());

        // ⚠ 只有 `shot` 会被摘：别的命令里 `--out` 可能是**要输入的内容**
        // （`type_text "--out"` 要把那三个字打进输入框）。
        let (cmd, out) = split_out_path("type_text \"--out\"");
        assert_eq!(cmd, "type_text \"--out\"");
        assert!(out.is_none());

        // ⚠ 引号里的**连续空格**必须保住：按空白切分再拼回去会把它压成一个。
        let (cmd, _) = split_out_path("type_text \"a  b\"");
        assert_eq!(cmd, "type_text \"a  b\"");
    }

    /// `--rect` / `--scale` 也是客户端侧的，且**逐字节保留**其余部分。
    #[test]
    fn crop_options_are_stripped_and_the_rest_is_preserved() {
        let (cmd, out, rect, scale) =
            strip_client_options("shot --out C:/tmp/a.png --rect 10,20,30,40 --scale 3 --repaint");
        assert_eq!(cmd, "shot --repaint");
        assert_eq!(out.unwrap().to_string_lossy(), "C:/tmp/a.png");
        assert_eq!(rect, Some((10, 20, 30, 40)));
        assert_eq!(scale, 3);

        // 不是 `shot` 的命令一个都不摘。
        let (cmd, out, rect, scale) = strip_client_options("type_text \"--out\"");
        assert_eq!(cmd, "type_text \"--out\"");
        assert!(out.is_none());
        assert!(rect.is_none());
        assert_eq!(scale, 1);
    }

    /// 末尾 N 行：只给最后几行，且**不**带更早的行。
    #[test]
    fn tail_lines_returns_the_last_n_lines() {
        let dir = std::env::temp_dir().join(format!("zephyra-bridge-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.log");
        std::fs::write(&path, "one\ntwo\nthree\nfour\nfive\n").unwrap();

        let lines = tail_lines(&path, 2).unwrap();
        assert_eq!(lines, vec!["four".to_string(), "five".to_string()]);

        // 要的行数比文件还多 ⇒ 全给。
        let lines = tail_lines(&path, 99).unwrap();
        assert_eq!(lines.len(), 5);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 文件不存在要**明确报错**（而不是安静地给空）。
    #[test]
    fn tail_lines_reports_a_missing_file() {
        let err = tail_lines(std::path::Path::new("no/such/file.log"), 10).unwrap_err();
        assert!(err.contains("失败"), "{err}");
    }
}

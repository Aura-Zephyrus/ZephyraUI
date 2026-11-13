//! `zephyra debug`：调试器。
//!
//! # 为什么要有它
//!
//! `bridge` 是「发一条命令、看一条回答」—— 一次一问。而调试真正难的是
//! **盯着一个量随时间怎么变**：
//!
//! - 「这个按钮的 `Pressed` 属性到底有没有被写上？」
//! - 「我点了它，为什么画面没变？」
//! - 「改样式之后，**哪些节点**变了？」
//!
//! 这三个问题的共同形状是「**反复问同一个问题，只在答案变了的时候才说话**」。
//! 人做这件事靠盯着窗口；AI 做这件事需要一个工具。
//!
//! | 子命令 | 干什么 |
//! |---|---|
//! | `watch` | 反复问一条桥命令，**只在答案变了时**打印 |
//! | `tree` | 打印一棵子树（按路径 + 深度裁剪） |
//! | `snapshot` | 把当前布局树存成一份 |
//! | `diff` | 与存的那份比 —— 哪些节点变了 |
//!
//! # 为什么不新增桥协议
//!
//! `watch` 问的是**任意**桥命令（`props 0.1.0.1` / `stats` / `tree`…），
//! 所以它不需要新协议 —— 它只是**反复发已有的命令**。
//!
//! 而 `tree` / `snapshot` / `diff` 都在**客户端**做裁剪与比较：宿主只给整棵树，
//! 客户端手里已经有那份文本了，裁一块不需要改 GPU 侧的拷贝区。

use crate::args::Args;
use crate::bridge_client::{BridgeClient, BridgeResponse};
use crate::commands::{CliError, Write};
use crate::ExitCode;

/// 应用侧桥的默认端口。与 `zephyra-fx::bridge::DEFAULT_PORT` 一致
/// （CLI 刻意不依赖 zephyra-fx；两边各写一份，由测试钉住）。
fn default_port() -> u16 {
    7878
}

/// `watch` 的默认采样间隔（毫秒）。
const DEFAULT_INTERVAL_MS: u64 = 200;

/// `watch` 的默认观察时长（毫秒）。
const DEFAULT_DURATION_MS: u64 = 5_000;

pub fn run(args: &Args, io: &mut dyn Write) -> Result<ExitCode, CliError> {
    let sub = args
        .positional
        .first()
        .map(String::as_str)
        .unwrap_or("")
        .to_string();

    // `snapshot` 的参数是**文件路径**，不是桥命令 —— 它是纯本地动作。
    if sub == "snapshot" {
        return snapshot(args, io);
    }

    let host = args.option("host").unwrap_or("127.0.0.1").to_string();
    let port: u16 = args
        .option("port")
        .map(|p| p.parse())
        .transpose()
        .map_err(|_| CliError::usage("--port needs a number"))?
        .unwrap_or_else(default_port);

    let mut client = BridgeClient::connect(&host, port, 3).map_err(|e| {
        CliError::failure(format!(
            "连不上应用调试桥 {host}:{port}（{e}）——确认应用以 ZEPHYRA_DEBUG_PORT={port} 启动"
        ))
    })?;

    match sub.as_str() {
        "watch" => watch(&mut client, args, io),
        "tree" => tree(&mut client, args, io),
        "diff" => diff(&mut client, args, io),
        "" => Err(CliError::usage(
            "debug 需要一个子命令：watch / tree / snapshot / diff",
        )),
        other => Err(CliError::usage(format!(
            "debug 只接受 `watch` / `tree` / `snapshot` / `diff`，实得 `{other}`"
        ))),
    }
}

/// `debug watch <命令...>`：反复问，只在答案变了时打印。
///
/// # 为什么默认只打印「变化」
///
/// 一条 `props` 的输出有十几行；每 200 ms 打一次，一天下来是几十万行噪音。
/// 而调试要看的恰恰是**变化的那一刻** —— 「属性在第 3.2 秒被写上了」。
///
/// `--all` 可以把每次采样都打出来（要看「它一直不变」时有用）。
///
/// # 为什么答案变了才算「有变化」
///
/// 比较的是**整段文本**。桥的回答是确定性的：同一状态 → 同一段文本。
/// 所以文本变了 ⇒ 状态变了。
fn watch(client: &mut BridgeClient, args: &Args, io: &mut dyn Write) -> Result<ExitCode, CliError> {
    // 命令 = 除子命令外的**全部**位置参数（拼回一行）。
    //
    // ⚠ 不能只取第一个位置参数：`props 0.1.0.1` 是三个词。
    let cmd = args
        .positional
        .iter()
        .skip(1)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if cmd.trim().is_empty() {
        return Err(CliError::usage(
            "debug watch 需要一条桥命令，如 `debug watch \"props 0.1.0.1\"`",
        ));
    }

    let interval_ms = match args.option("interval-ms") {
        Some(v) => v
            .parse()
            .map_err(|_| CliError::usage("--interval-ms needs a number"))?,
        None => DEFAULT_INTERVAL_MS,
    };
    let duration_ms = match args.option("duration-ms") {
        Some(v) => v
            .parse()
            .map_err(|_| CliError::usage("--duration-ms needs a number"))?,
        None => DEFAULT_DURATION_MS,
    };
    if interval_ms == 0 || duration_ms == 0 {
        return Err(CliError::usage("--interval-ms 与 --duration-ms 不能为 0"));
    }
    let print_all = args.has("all");

    io.line(&format!(
        "盯着 `{cmd}`，每 {interval_ms} ms 问一次，共 {duration_ms} ms（Ctrl-C 提前结束）"
    ));

    let started = std::time::Instant::now();
    let mut previous: Option<Vec<String>> = None;
    let mut samples = 0usize;
    let mut changes = 0usize;

    while (started.elapsed().as_millis() as u64) < duration_ms {
        let now = started.elapsed().as_millis() as u64;
        let lines = match client.call(&cmd) {
            Ok(BridgeResponse::Text(lines)) => lines,
            Ok(BridgeResponse::Err(e)) => {
                // 桥回错**不是**终止条件：应用可能正在切页，那一瞬间路径
                // 是不存在的。把它当成「这一次的答案是错误」记下来。
                vec![format!("ERR {e}")]
            }
            Ok(BridgeResponse::Binary { .. }) => vec!["<binary response>".into()],
            Err(e) => {
                io.err_line(&format!("问应用失败：{e}"));
                return Err(CliError::failure("调试器与应用的连接断了"));
            }
        };
        samples += 1;
        let changed = previous.as_ref().map(|p| *p != lines).unwrap_or(true);
        if changed {
            changes += 1;
            io.line(&format!("--- +{now}ms ---"));
            for line in &lines {
                io.line(line);
            }
        } else if print_all {
            io.line(&format!("--- +{now}ms（未变）---"));
            for line in &lines {
                io.line(line);
            }
        }
        previous = Some(lines);
        std::thread::sleep(std::time::Duration::from_millis(interval_ms));
    }

    io.line(&format!(
        "采样 {samples} 次，答案变了 {changes} 次（只看变化；--all 可看每次）"
    ));
    Ok(ExitCode::Success)
}

/// `debug tree [路径] [--depth N]`：打印一棵子树。
///
/// 裁剪在**客户端**做：宿主只给整棵树，而客户端手里已经有那份文本了。
fn tree(client: &mut BridgeClient, args: &Args, io: &mut dyn Write) -> Result<ExitCode, CliError> {
    let root = args.positional.get(1).map(String::as_str).unwrap_or("0");
    let depth: Option<usize> = match args.option("depth") {
        Some(v) => Some(
            v.parse()
                .map_err(|_| CliError::usage("--depth needs a number"))?,
        ),
        None => None,
    };

    let lines = call_text(client, "tree")?;
    let selected = select_subtree(&lines, root, depth);
    if selected.is_empty() {
        return Err(CliError::failure(format!(
            "路径 `{root}` 下没有节点（树的根是 `0`）"
        )));
    }
    for line in selected {
        io.line(&line);
    }
    Ok(ExitCode::Success)
}

/// `debug snapshot <文件>`：把当前布局树存成一份。
///
/// ⚠ 存的是**布局树**（不是截图）：截图说「画面长这样」，树说「有哪些节点、
/// 在哪、什么文字」。排查「点了没反应」时，树比图有用。
fn snapshot(args: &Args, io: &mut dyn Write) -> Result<ExitCode, CliError> {
    let path = args
        .positional
        .get(1)
        .ok_or_else(|| CliError::usage("debug snapshot 需要一个文件路径"))?;
    let host = args.option("host").unwrap_or("127.0.0.1").to_string();
    let port: u16 = args
        .option("port")
        .map(|p| p.parse())
        .transpose()
        .map_err(|_| CliError::usage("--port needs a number"))?
        .unwrap_or_else(default_port);
    let mut client = BridgeClient::connect(&host, port, 3).map_err(|e| {
        CliError::failure(format!(
            "连不上应用调试桥 {host}:{port}（{e}）——确认应用以 ZEPHYRA_DEBUG_PORT={port} 启动"
        ))
    })?;
    let lines = call_text(&mut client, "tree")?;
    let text = lines.join("\n");
    std::fs::write(path, format!("{text}\n"))
        .map_err(|e| CliError::failure(format!("写 {path} 失败：{e}")))?;
    io.line(&format!("已存 {path}（{} 行）", lines.len()));
    Ok(ExitCode::Success)
}

/// `debug diff <文件>`：与存的那份比。
///
/// 判据是**按路径**：树里每行的第一个字段就是路径，于是「同一个路径的两行
/// 不同」= 「那个节点变了」。
///
/// 报三件事：**新增**（路径原来没有）、**消失**（原来有现在没有）、
/// **变了**（路径在，但那一行不同）。
fn diff(client: &mut BridgeClient, args: &Args, io: &mut dyn Write) -> Result<ExitCode, CliError> {
    let path = args
        .positional
        .get(1)
        .ok_or_else(|| CliError::usage("debug diff 需要一个先前 snapshot 的文件路径"))?;
    let saved = std::fs::read_to_string(path)
        .map_err(|e| CliError::failure(format!("读 {path} 失败：{e}")))?;
    let current = call_text(client, "tree")?;

    let old_map = index_by_path(&saved);
    let new_map = index_by_path(&current.join("\n"));

    let mut added = 0usize;
    let mut removed = 0usize;
    let mut changed = 0usize;
    for (path_key, line) in &new_map {
        match old_map.get(path_key) {
            None => {
                added += 1;
                io.line(&format!("+ {line}"));
            }
            Some(old) if old != line => {
                changed += 1;
                io.line(&format!("~ {line}"));
                io.line(&format!("  原：{old}"));
            }
            Some(_) => {}
        }
    }
    for (path_key, line) in &old_map {
        if !new_map.contains_key(path_key) {
            removed += 1;
            io.line(&format!("- {line}"));
        }
    }
    io.line(&format!(
        "新增 {added}，消失 {removed}，变了 {changed}（共 {} 个节点 → {} 个节点）",
        old_map.len(),
        new_map.len()
    ));
    Ok(ExitCode::Success)
}

/// 按路径索引一棵树（路径 → 那一行）。
///
/// ⚠ 键是**行首的路径字段**，不是整行 —— 缩进会变（树的深度变了），
/// 而路径不会。
fn index_by_path(text: &str) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some((path_key, _)) = trimmed.split_once(' ') else {
            continue;
        };
        map.insert(path_key.to_string(), trimmed.to_string());
    }
    map
}

/// 裁出一棵子树。
///
/// 判据：路径**等于**根，或以 `根.` 开头（后者是子节点）。
///
/// ⚠ 不能用 `starts_with(root)` —— 那样 `0.1` 会把 `0.10` 也算进来。
/// 所以分隔符是**必须**的。
fn select_subtree(lines: &[String], root: &str, depth: Option<usize>) -> Vec<String> {
    let prefix = format!("{root}.");
    lines
        .iter()
        .filter(|line| {
            let trimmed = line.trim_start();
            let Some((path_key, _)) = trimmed.split_once(' ') else {
                return false;
            };
            if path_key != root && !path_key.starts_with(&prefix) {
                return false;
            }
            match depth {
                None => true,
                Some(max) => path_key.matches('.').count() - root.matches('.').count() <= max,
            }
        })
        .cloned()
        .collect()
}

/// 发一条命令，把响应原样收回（文本）。
fn call_text(client: &mut BridgeClient, cmd: &str) -> Result<Vec<String>, CliError> {
    match client.call(cmd) {
        Ok(BridgeResponse::Text(lines)) => Ok(lines),
        Ok(BridgeResponse::Err(e)) => Err(CliError::failure(format!("{cmd}: {e}"))),
        Ok(BridgeResponse::Binary { .. }) => {
            Err(CliError::failure(format!("{cmd}: 回的是二进制，不是文本")))
        }
        Err(e) => Err(CliError::failure(format!("{cmd}: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_lines() -> Vec<String> {
        [
            "0 Box window 0,0 1400x800 window",
            "  0.0 Box titlebar 0,0 1400x44 titlebar",
            "    0.0.0 Text app-title 10,13 120x18 text-body-strong \"ZephyraUI\"",
            "  0.1 Box body 0,44 1400x756 body",
            "    0.1.0 Box nav 0,44 260x756 nav-pane",
            "    0.1.1 Box content 260,44 1140x756 content-card",
            "      0.1.1.0 Control button 300,60 100x40 button",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    /// 子树裁剪：只留根与它的后代。
    #[test]
    fn subtree_keeps_the_root_and_its_descendants() {
        let out = select_subtree(&tree_lines(), "0.1", None);
        let paths: Vec<&str> = out
            .iter()
            .map(|l| l.trim_start().split(' ').next().unwrap())
            .collect();
        assert_eq!(paths, vec!["0.1", "0.1.0", "0.1.1", "0.1.1.0"], "{paths:?}");
    }

    /// ⚠ 边界：`0.1` **不能**匹配到 `0.10`。
    ///
    /// 这是用 `starts_with` 会踩的坑 —— 而这类 bug 的症状是「多出来一整棵
    /// 别的子树」，读的人第一反应是「树坏了」。
    #[test]
    fn subtree_does_not_match_a_sibling_with_a_longer_number() {
        let lines = vec![
            "0 Box window 0,0 100x100 window".to_string(),
            "  0.1 Box a 0,0 10x10 a".to_string(),
            "  0.10 Box b 0,0 10x10 b".to_string(),
        ];
        let out = select_subtree(&lines, "0.1", None);
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].contains("0.1 Box a"), "{out:?}");
    }

    /// 深度裁剪：根算第 0 层。
    #[test]
    fn depth_limits_the_subtree() {
        let out = select_subtree(&tree_lines(), "0.1", Some(1));
        let paths: Vec<&str> = out
            .iter()
            .map(|l| l.trim_start().split(' ').next().unwrap())
            .collect();
        assert_eq!(paths, vec!["0.1", "0.1.0", "0.1.1"], "{paths:?}");
    }

    /// 按路径索引：缩进变了不算「变了」。
    #[test]
    fn index_is_keyed_by_path_not_by_the_whole_line() {
        let a = index_by_path("0 Box window 0,0 100x100 window\n  0.1 Box a 0,0 1x1 a");
        let b = index_by_path("0 Box window 0,0 100x100 window\n    0.1 Box a 0,0 1x1 a");
        assert_eq!(a.get("0.1"), b.get("0.1"));
    }
}

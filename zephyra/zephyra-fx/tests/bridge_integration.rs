//! 框架级集成测试：真实 `BridgeHost` 契约 + 假 MCP，按**整条协议链**断言。
//!
//! # 与 unit 测试的分工
//!
//! `tests/unit/bridge.rs` 逐函数判（解析、单命令、单状态转换）。这里按链路判：
//! 配置 → 身份帧 → 命令往返 → MCP 重启断连收尾 → 自动重连 → 新身份帧。
//! 任一层漏了，链的某个环节就会缺一条消息或晚一个钩子，直接红在这里。
//!
//! 宿主侧用真实的 [`BridgeHost`] 契约实现（不是 stub）：`on_bridge_disconnect`
//! 的调用次数是断言的一部分 —— MCP 重启时宿主必须且只补一次收尾。

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use zephyra_fx::bridge::{BridgeConfig, BridgeHost, DebugBridge, FrameStats, ShotOptions};

/// 记录断连收尾次数的宿主契约实现。
struct RecordingHost {
    disconnects: Arc<AtomicUsize>,
}

impl BridgeHost for RecordingHost {
    fn viewport(&self) -> (f32, f32) {
        (800.0, 600.0)
    }
    fn tree_dump(&self) -> String {
        "0 Container root 0,0 800x600".into()
    }
    fn props_dump(&self, _path: &[usize]) -> Option<String> {
        None
    }
    fn stats(&self) -> FrameStats {
        FrameStats::default()
    }
    fn root(&self) -> Option<&zephyra_core::node::Node> {
        None
    }
    fn inject_hover(&mut self, _x: f32, _y: f32) {}
    fn inject_click(&mut self, _x: f32, _y: f32) {}
    fn inject_pointer_down(&mut self) {}
    fn inject_pointer_up(&mut self) {}
    fn inject_key(&mut self, _key: &str) {}
    fn inject_scroll(&mut self, _x: f32, _y: f32, _dx: f32, _dy: f32) {}
    fn inject_text(&mut self, _text: &str) {}
    fn screenshot(&mut self, _opts: ShotOptions) -> Option<Vec<u8>> {
        None
    }
    fn on_bridge_disconnect(&mut self) {
        self.disconnects.fetch_add(1, Ordering::AcqRel);
    }
}

/// 假 MCP：接两轮连入。每轮读身份帧、发一条 `ping`、读回响应；
/// 第一轮结束后主动断开，模拟 MCP 重启（应用应退避重连）。
fn fake_mcp(listener: TcpListener, tx: Sender<String>) {
    for round in 0..2 {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).expect("read hello");
        tx.send(format!("r{round}:hello:{}", line.trim())).unwrap();

        stream.write_all(b"ping\n").expect("write ping");
        let mut pong = String::new();
        reader.read_line(&mut pong).expect("read pong");
        let mut ok = String::new();
        reader.read_line(&mut ok).expect("read OK");
        tx.send(format!("r{round}:resp:{}:{}", pong.trim(), ok.trim()))
            .unwrap();
        // 第一轮结束 ⇒ drop stream ⇒ 应用察觉断连并退避重连。
    }
}

/// 显式配置连入 → hello → 命令往返 → MCP 重启 → 收尾一次 → 重连 → 新 hello。
#[test]
fn explicit_config_survives_mcp_restart_with_one_disconnect_hook() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    let mcp = std::thread::spawn(move || fake_mcp(listener, tx));

    let mut bridge = DebugBridge::with_config(BridgeConfig {
        connect_to: Some(addr),
        app_name: Some("integration-app".into()),
        app_version: Some("1.2.3".into()),
        ..Default::default()
    })
    .expect("显式配置应当开启桥");

    let disconnects = Arc::new(AtomicUsize::new(0));
    let mut host = RecordingHost {
        disconnects: disconnects.clone(),
    };

    // 像真宿主 tick 那样每帧 poll；重连退避 CONNECT_RETRY=500ms，给足时间。
    let mut messages = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while messages.len() < 4 && Instant::now() < deadline {
        bridge.poll(&mut host); // ← 宿主帧首的同一调用，触发断连钩子
        while let Ok(m) = rx.try_recv() {
            messages.push(m);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    mcp.join().unwrap();

    assert_eq!(messages.len(), 4, "整条链应留下 4 条消息: {messages:?}");
    assert_eq!(messages[0], "r0:hello:hello integration-app 1.2.3");
    assert!(
        messages[1].contains("pong") && messages[1].ends_with(":OK"),
        "{}",
        messages[1]
    );
    assert_eq!(messages[2], "r1:hello:hello integration-app 1.2.3");
    assert!(
        messages[3].contains("pong") && messages[3].ends_with(":OK"),
        "{}",
        messages[3]
    );

    assert_eq!(
        disconnects.load(Ordering::Acquire),
        1,
        "MCP 重启只有一次断连收尾，重连成功不该再触发"
    );
    // 不检查末态 state()：假 MCP 第二轮答完就 drop 了流，检查那一刻可能
    // 已经是 Reconnecting。两条 hello + 两次 pong 已证明重连确实发生过。
}

/// 显式配置不声明任何拓扑 ⇒ 桥保持关闭（IDE 直接启动的默认形态）。
#[test]
fn empty_explicit_config_leaves_bridge_off() {
    assert!(DebugBridge::with_config(BridgeConfig::default()).is_none());
}

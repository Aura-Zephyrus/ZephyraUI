//! 真 winit 宿主 + 假 MCP 的端到端验证程序。
//!
//! # 为什么是 example 而不是 `#[test]`
//!
//! winit 0.30 要求 `EventLoop` 在主线程创建（`any_thread` 只是兼容性补丁，实测在
//! cargo test 的子线程里建窗 + 跑 D3D11 会访问违规）。cargo test 的每个用例都在
//! 独立线程，装不下真宿主。所以真机验证走 example，在主线程跑：
//!
//! ```powershell
//! cargo run -p zephyra-winit --example mcp_e2e
//! ```
//!
//! # 与自动门禁的分工
//!
//! 协议链路的自动门禁在 `zephyra-fx/tests/bridge_integration.rs`（真实 BridgeHost
//! 契约 + 假 MCP）。本程序补的是「真 EventLoop + HostWindow + set_bridge 注入」
//! 这条路 —— 它不能被自动跑，但能被手动验证。
#![cfg(windows)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use winit::application::ApplicationHandler;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};

use zephyra_core::context::Context;
use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_fx::bridge::{BridgeConfig, DebugBridge};
use zephyra_fx::{InputState, View};
use zephyra_winit::HostWindow;

/// MCP 回传的一对文本：身份行与命令响应。
type Reply = (String, String);

struct TestView;

impl View for TestView {
    fn on_input(&mut self, _state: &InputState) {}
    fn view(&self, _ctx: &Context<'_>) -> Element {
        Element::new(NodeTag::Box)
    }
}

/// 包装 handler：`resumed` 里建真宿主并注入桥，`about_to_wait` 驱动
/// `HostWindow` 自己的 tick（帧首的 `bridge.poll` 就在那里）。
struct App {
    bridge: Option<DebugBridge>,
    host: Option<HostWindow<TestView>>,
    done: Arc<AtomicBool>,
    ticks: usize,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let mut host = HostWindow::new_lenient(event_loop, TestView);
        host.resumed(event_loop); // 走 HostWindow 自己的初始化（dirty + Poll）
        if let Some(bridge) = self.bridge.take() {
            host.set_bridge(Some(bridge));
        }
        self.host = Some(host);
        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        _event: winit::event::WindowEvent,
    ) {
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(host) = &mut self.host {
            // ApplicationHandler::about_to_wait 内部调用 tick() ⇒ bridge.poll()。
            host.about_to_wait(event_loop);
        }
        self.ticks += 1;
        if self.done.load(Ordering::Acquire) || self.ticks > 1000 {
            event_loop.exit();
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// 假 MCP：读身份帧、发一条 `ping`、读回响应，然后通知主程序退出。
fn fake_mcp(listener: TcpListener, done: Arc<AtomicBool>, tx: Sender<Reply>) {
    let (mut stream, _) = listener.accept().expect("accept 应用连入");
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut hello = String::new();
    reader.read_line(&mut hello).expect("读 hello");
    stream.write_all(b"ping\n").expect("发 ping");
    let mut pong = String::new();
    reader.read_line(&mut pong).expect("读 pong");
    tx.send((hello.trim().to_string(), pong.trim().to_string()))
        .unwrap();
    done.store(true, Ordering::Release);
}

fn main() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx): (Sender<Reply>, Receiver<Reply>) = mpsc::channel();
    let done = Arc::new(AtomicBool::new(false));
    let mcp_done = done.clone();
    let mcp = std::thread::spawn(move || fake_mcp(listener, mcp_done, tx));

    let bridge = DebugBridge::with_config(BridgeConfig {
        connect_to: Some(addr),
        app_name: Some("winit-e2e".into()),
        app_version: Some("1.0".into()),
        ..Default::default()
    })
    .expect("显式配置应当开启桥");

    let event_loop = EventLoop::new().expect("建事件循环（主线程）");
    let mut app = App {
        bridge: Some(bridge),
        host: None,
        done: done.clone(),
        ticks: 0,
    };
    event_loop.run_app(&mut app).expect("跑事件循环");
    mcp.join().unwrap();

    let (hello, pong) = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("MCP 应收到身份与响应");
    assert_eq!(hello, "hello winit-e2e 1.0");
    assert!(pong.starts_with("pong"), "pong 应带宿主身份: {pong}");
    assert!(app.ticks < 1000, "事件循环不应跑到上限");

    println!(
        "[mcp-e2e] PASS: hello={hello:?} pong={pong:?} ticks={}",
        app.ticks
    );
}

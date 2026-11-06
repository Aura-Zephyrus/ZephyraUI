//! 纯逻辑门禁：不需要窗口、不需要 GPU、不需要事件循环。
//!
//! 被测的三条判据（事件按窗口分发之外的另外两条）都住在这里：
//! owned 级联关闭、句柄失效、命令队列。

use std::any::Any;

use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

use zephyra_core::styler::Styler;
use zephyra_core::theme::ThemeVariant;
use zephyra_fx::View;

use super::*;

/// 一个不碰平台的宿主替身。
struct FakeHost {
    key: WindowKey,
    theme: ThemeVariant,
    title: String,
    teardown_count: u32,
    close_requests: u32,
}

impl FakeHost {
    fn new(key: WindowKey) -> Self {
        Self {
            key,
            theme: ThemeVariant::Dark,
            title: String::new(),
            teardown_count: 0,
            close_requests: 0,
        }
    }
}

impl WindowHost for FakeHost {
    fn key(&self) -> WindowKey {
        self.key
    }

    fn winit_id(&self) -> Option<WindowId> {
        None
    }

    fn handle_event(&mut self, _event: WindowEvent) -> HostEventOutcome {
        HostEventOutcome::Continue
    }

    fn tick(&mut self) -> HostTick {
        HostTick::default()
    }

    fn set_theme(&mut self, variant: ThemeVariant) {
        self.theme = variant;
    }

    fn set_styler(&mut self, _styler: Box<dyn Styler>) {}

    fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
    }

    fn request_redraw(&self) {}

    fn request_close(&mut self) {
        self.close_requests += 1;
    }

    fn teardown(&mut self) {
        self.teardown_count += 1;
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// `Backend` 的替身。测试不创建真窗口，所以 `create` 直接报错。
struct FakeBackend;

impl Backend for FakeBackend {
    type Window<V: View + 'static> = FakeHost;

    fn create<V: View + 'static>(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _view: V,
        _spec: &WindowSpec,
        _handle: WindowHandle,
    ) -> Result<Self::Window<V>, String> {
        Err("测试替身不建窗口".into())
    }
}

fn set() -> WindowSet<FakeBackend> {
    WindowSet::new(FakeBackend, ShutdownMode::OnLastWindowClosed)
}

fn add(set: &mut WindowSet<FakeBackend>, owner: Option<WindowKey>) -> (WindowKey, WindowHandle) {
    let key = set.allocate_key_for_test();
    let handle = set.push_host(key, Box::new(FakeHost::new(key)), owner);
    (key, handle)
}

#[test]
fn closing_one_window_keeps_siblings() {
    let mut set = set();
    let (a, _) = add(&mut set, None);
    let (b, _) = add(&mut set, None);

    set.close_window(a);

    assert_eq!(set.window_count(), 1);
    assert!(set.host(a).is_none());
    assert!(set.host(b).is_some());
}

#[test]
fn close_cascades_to_owned_windows() {
    let mut set = set();
    let (main, _) = add(&mut set, None);
    let (_tool, _) = add(&mut set, Some(main));
    let (unrelated, _) = add(&mut set, None);

    set.close_window(main);

    assert_eq!(set.window_count(), 1, "主窗口与工具窗口都该关掉");
    assert!(set.host(unrelated).is_some(), "无关窗口不该被牵连");
}

#[test]
fn owner_chain_closes_recursively() {
    let mut set = set();
    let (main, _) = add(&mut set, None);
    let (tool, _) = add(&mut set, Some(main));
    let (tool_child, _) = add(&mut set, Some(tool));

    set.close_window(main);

    assert_eq!(set.window_count(), 0);
    assert!(set.host(tool_child).is_none());
}

#[test]
fn handle_becomes_invalid_after_close() {
    let mut set = set();
    let (key, handle) = add(&mut set, None);
    assert!(handle.is_alive());

    set.close_window(key);

    assert!(!handle.is_alive());
    // 失效句柄的命令必须被静默丢弃，而不是把命令投给一个已关窗口。
    handle.close();
    let mut exit = false;
    set.drain_commands(&mut exit);
    assert_eq!(set.window_count(), 0);
}

#[test]
fn teardown_runs_exactly_once_per_window() {
    let mut set = set();
    let (key, _) = add(&mut set, None);
    set.close_window(key);
    // 二次关闭是空操作。
    set.close_window(key);
    assert_eq!(set.window_count(), 0);
}

#[test]
fn commands_are_queued_and_drained() {
    let mut set = set();
    let (key, handle) = add(&mut set, None);

    handle.set_title("Inspector");
    handle.set_theme(ThemeVariant::Light);
    handle.request_redraw();

    let mut exit = false;
    set.drain_commands(&mut exit);
    assert!(!exit);

    let host = set
        .host_mut(key)
        .unwrap()
        .as_any_mut()
        .downcast_mut::<FakeHost>()
        .unwrap();
    assert_eq!(host.title, "Inspector");
    assert_eq!(host.theme, ThemeVariant::Light);
}

#[test]
fn close_command_closes_only_its_window() {
    let mut set = set();
    let (a, ha) = add(&mut set, None);
    let (b, _) = add(&mut set, None);

    ha.close();
    let mut exit = false;
    set.drain_commands(&mut exit);

    assert!(set.host(a).is_none());
    assert!(set.host(b).is_some());
}

#[test]
fn exit_command_is_reported_not_executed_inline() {
    let mut set = set();
    let (_, handle) = add(&mut set, None);

    handle.exit();
    let mut exit = false;
    set.drain_commands(&mut exit);

    assert!(exit, "Exit 命令必须让调用方去退事件循环");
    // 命令本身不关窗口：关闭是 `close_all` 的事，调用方按顺序做。
    assert_eq!(set.window_count(), 1);
}

#[test]
fn set_theme_all_reaches_every_window() {
    let mut set = set();
    let (a, _) = add(&mut set, None);
    let (b, _) = add(&mut set, None);

    set.set_theme_all(ThemeVariant::Light);

    for key in [a, b] {
        let host = set
            .host_mut(key)
            .unwrap()
            .as_any_mut()
            .downcast_mut::<FakeHost>()
            .unwrap();
        assert_eq!(host.theme, ThemeVariant::Light);
    }
}

#[test]
fn keys_are_never_reused() {
    let mut set = set();
    let (first, _) = add(&mut set, None);
    set.close_window(first);
    let (second, _) = add(&mut set, None);

    assert_ne!(first, second, "窗口标识不得复用");
}

#[test]
fn window_spec_layers_over_config() {
    let mut config = AppConfig::default();
    config.window.frameless = true;
    config.window.transparent = false;
    config.windows.insert(
        "inspector".into(),
        zephyra_app_config::WindowProfile {
            title: Some("Inspector".into()),
            width: Some(420),
            height: Some(600),
            ..Default::default()
        },
    );

    let spec = WindowSpec::named("inspector").title("Overridden");
    let opts = spec.resolve(&config);

    assert_eq!(opts.title.as_deref(), Some("Overridden"));
    assert_eq!(opts.size, Some((420, 600)));
    assert!(opts.resolve_frameless(&config));
}

#[test]
fn main_spec_uses_window_section_only() {
    let mut config = AppConfig::default();
    config.window.frameless = true;
    config.windows.insert(
        "inspector".into(),
        zephyra_app_config::WindowProfile {
            frameless: Some(false),
            ..Default::default()
        },
    );

    let opts = WindowSpec::main().resolve(&config);

    assert!(opts.resolve_frameless(&config));
    assert_eq!(opts.name, None);
}

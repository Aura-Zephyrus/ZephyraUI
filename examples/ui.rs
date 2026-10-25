//! demo 结构的声明式来源：`ui/demo.zypui` 的运行时入口。
//!
//! # 两条路，一份 IR（与 gallery/src/ui.rs 同一条命门）
//!
//! - **release**：build.rs 用 zypuic 把 `ui/demo.zypui` 编译成二进制，这里
//!   `include_bytes!` + `decode` —— 运行时**零解析**。
//! - **dev**：`zypuic::parse_all` 直读源文件（迁移期 parity 测试在用）。
//!
//! 两条路汇进同一份 [`RawUi`]，实例化只有一份实现。

use std::sync::{Mutex, OnceLock};

use zypui::{Interner, Ui};

static DEMO_UI: OnceLock<Ui> = OnceLock::new();
/// 表达式求值的字符串去重器（动态 key / class 的 `&'static` 来源）。
///
/// 跨帧复用：每帧新建的话，同一个动态串每帧都会被 intern 成新的 'static
/// —— 那正是本仓每帧泄漏纪律要消灭的形状。UI 单线程，锁只在同帧内竞争。
static INTERNER: OnceLock<Mutex<Interner>> = OnceLock::new();

/// demo UI（进程内一份，跨帧复用）。
///
/// 解码失败是**编译期产物的错**（build.rs 与 zypui 版本错位才会发生），
/// 直接 panic —— 这不是运行期能降级的错。
pub fn demo_ui() -> &'static Ui {
    DEMO_UI.get_or_init(|| {
        let bytes: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/demo.zypui.bin"));
        let raw = zypui::decode::decode(bytes).expect("demo.zypui.bin 解码失败（产物错位？）");
        Ui::new(raw).expect("组件重名")
    })
}

/// 每帧一棵树：`Demo` 页实例化。
///
/// `Interner` 跨帧复用（见上）；页名拼错是**错**不是静默空白（`build`
/// 返回 `Result`，这里直接 expect —— 运行期不会走到，parity 测试钉着）。
pub fn demo<H: zypui::Host>(host: &H) -> zephyra_core::element::Element {
    let mut it = INTERNER
        .get_or_init(|| Mutex::new(Interner::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut els = demo_ui()
        .build("Demo", host, &mut it)
        .expect("Demo 页实例化失败");
    assert!(
        els.len() == 1,
        "Demo 页应恰好产出 1 个根元素，实际 {}",
        els.len()
    );
    els.remove(0)
}

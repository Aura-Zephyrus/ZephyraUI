//! 控件结构的声明式来源：`ui/controls.zypui` 的运行时入口。
//!
//! # 两条路，一份 IR（命门）
//!
//! - **release**：build.rs 用 zypuic 把 `ui/controls.zypui` 编译成二进制，
//!   这里 `include_bytes!` + `decode` —— 运行时**零解析**。
//! - **dev**：`zypuic::parse_all` 直读源文件（parity 测试在用）。
//!
//! 两条路汇进同一份 [`RawUi`]，实例化只有一份实现 —— 「dev 对、release 错」
//! 的表示漂移在结构上不可能（round-trip 测试在 zypui/zypuic 两侧钉住）。
//!
//! # 组件只是名字
//!
//! 这里的组件（Button / Card / …）是**模板**：页面 `.zypui` 里 `<Button …/>`
//! 展开成裸元素树。Rust 侧不再有同名构造函数 —— 结构的真相只有这一份。

use std::sync::OnceLock;

use zypui::Ui;

static CONTROLS: OnceLock<Ui> = OnceLock::new();

/// 控件组件库（进程内一份，跨帧复用）。
///
/// 解码失败是**编译期产物的错**（build.rs 与 zypui 版本错位才会发生），
/// 直接 panic —— 这不是运行期能降级的错。
pub fn controls() -> &'static Ui {
    CONTROLS.get_or_init(|| {
        let bytes: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/controls.zypui.bin"));
        let raw = zypui::decode::decode(bytes).expect("controls.zypui.bin 解码失败（产物错位？）");
        Ui::new(raw).expect("控件组件重名")
    })
}

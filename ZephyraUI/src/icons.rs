//! 控件层自己的图标集 —— 框架内置字形 + 播放器专用字形。
//!
//! # 为什么图标在这里而不在底层
//!
//! 底层 `zephyra-vector` 的 `assets::REGISTRY` 只放**通用**字形（菜单、画笔、
//! 撤销…）。播放 / 暂停 / 音量属于**控件层**的词汇，应用专属字形应当由使用方
//! 用 [`IconSet::with`] 追加 —— 这条扩展点正是底层留给上层的。所以 `play.svg`
//! 等资源住在 `ui/icons/`，底层一行不动。
//!
//! 宿主拿 [`icon_set()`] 挂到 `Context::with_icons` / `HostWindow::with_icons`；
//! 不挂则图标位空白（与底层默认行为一致，不报错）。

use std::sync::OnceLock;

use zephyra_core::icon::{IconSet, IconSource};

const PLAY: &str = include_str!("../ui/icons/play.svg");
const PAUSE: &str = include_str!("../ui/icons/pause.svg");
const VOLUME_UP: &str = include_str!("../ui/icons/volume-up.svg");
const VOLUME_OFF: &str = include_str!("../ui/icons/volume-off.svg");
const FULLSCREEN: &str = include_str!("../ui/icons/fullscreen.svg");

/// 控件层的图标集（进程内唯一，解析一次）。
///
/// 与底层 `zephyra_core::icon::builtin()` 同构：`OnceLock` 保证每个名字只有
/// 一份 `'static` 图标数据，宿主侧栅格化缓存以数据地址为键，不能每次新建。
pub fn icon_set() -> &'static IconSet {
    static SET: OnceLock<IconSet> = OnceLock::new();
    SET.get_or_init(|| {
        let set = IconSet::with_builtin()
            .with("play", IconSource::Svg(PLAY))
            .with("pause", IconSource::Svg(PAUSE))
            .with("volume-up", IconSource::Svg(VOLUME_UP))
            .with("volume-off", IconSource::Svg(VOLUME_OFF))
            .with("fullscreen", IconSource::Svg(FULLSCREEN));
        set.freeze();
        set
    })
}

#[cfg(test)]
#[path = "../tests/unit/icons_tests.rs"]
mod tests;

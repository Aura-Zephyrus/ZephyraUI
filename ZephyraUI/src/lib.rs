#![forbid(unsafe_code)]
//! ZephyraUI —— 控件层：样式表、令牌、控件结构声明。
//!
//! # 这个 crate 的边界
//!
//! | 在这里 | 不在这里 |
//! |---|---|
//! | 控件的 `.zstyle` 样式表（[`styles`]） | 应用外壳与页面布局的 `.zstyle` |
//! | `@令牌` 名单镜像（[`tokens`]） | 路由、页面、应用状态 |
//! | 控件结构声明 `ui/controls.zypui` | 任何渲染后端 |
//! | 控件的**状态模型**（[`checkbox`] / [`toggle`] / [`slider`] / [`player`]） | 状态**存哪**（那是宿主的事） |
//! | 控件层自己的图标（[`icons`]） | 底层通用图标表（那个在 `zephyra-vector`） |
//! | 控件**自身语义**的文案（[`i18n`]） | 应用自己的文案表 |
//!
//! # 结构的真相只有一份：`.zypui`
//!
//! 控件结构住在 [`ui/controls.zypui`](ui/controls.zypui)（声明式组件模板），
//! 由**应用**在自己的 `build.rs` 里与页面模板一起编译成二进制 —— 见
//! `gallery/build.rs`：它直接读本目录下的这份文件，所以结构真相全仓只有
//! 一份，gallery 不复制它。
//!
//! # 两条不能碰的规矩
//!
//! 1. **外观不进源码。** 控件只发 `NodeTag` / `ElementKey` / 分类名 / 内容，
//!    颜色与尺寸全在 `.zstyle`。只靠框架数据能判的规则（令牌存在、motion 档位
//!    有规则、模板无字面文案）在本仓 `tests/widget_gates.rs`；需要整棵树 ×
//!    全部页面 × 全部交互态的规则仍在 `gallery/tests/gates.rs`（要应用自己的 View
//!    才能造树）。
//!
//! 2. **本 crate 不依赖渲染后端。** 控件库的用户不该被迫引入 D3D12。
//!    后端由应用（`gallery/`）用 feature 选。
//!
//! # i18n：控件只发 key
//!
//! 控件模板的文案参数收**语义 key**，翻译发生在控件内部（`tr()` → 宿主的
//! 文案表）。于是控件层**永远不发面向用户的文案** —— 模板源码的门禁做在
//! 本仓 `tests/widget_gates.rs`，应用树上的门禁在 `gallery/tests/gates.rs`。
//!
//! 控件**自身语义**的文案（复选框组组头的「全选」）归 [`i18n`]，自带三语言
//! 默认译文，应用可覆盖。

pub mod animation;
pub mod catalog;
pub mod checkbox;
pub mod i18n;
pub mod icons;
pub mod liquid_button;
pub mod motion;
pub mod player;
pub mod popup;
pub mod scroll_bar;
pub mod scroll_view;
pub mod slider;
pub mod styles;
pub mod text_area;
pub mod text_buffer;
pub mod text_field;
pub mod toggle;
pub mod tokens;

pub use catalog::{
    AvatarState, BusyIndicatorState, CardState, ColorPickerState, ExpanderState, HyperlinkState,
    InfoBadgeState, InfoBarState, PageIndicatorState, Progress, SelectorBarState,
    SettingExpanderState, SettingItemState, SpinBoxState, ToggleState, ToolButtonState,
    ToolTipState,
};
pub use checkbox::{CheckState, Checkbox, CheckboxGroup};
pub use i18n::{
    builtin_messages, is_key_param, merge_builtin, resolve, BUILTIN_KEYS, CHECKBOX_SELECT_ALL,
    KEY_PARAMS, NAMESPACE,
};
pub use icons::icon_set;
pub use liquid_button::{LiquidButton, LIQUID_BUTTON_H, LIQUID_BUTTON_W};
pub use motion::{
    morph_class, morph_height, reveal_items, visible, DropdownMotion, BTN_H, CLOSE_MS, MENU_H,
    MORPH_CLASSES, MORPH_STEPS, OPEN_MS, REVEAL_AT,
};
pub use player::{
    PlaybackState, PlayerState, CLASS_BUFFERING, CLASS_ENDED, CLASS_PAUSED, CLASS_PLAYING,
    KEY_FULLSCREEN, KEY_MUTE, KEY_PLAY_PAUSE, KEY_RATE, KEY_SEEK, KEY_SEEK_FILL, KEY_VOLUME,
    KEY_VOLUME_FILL, RATES, SEEK_TRACK_W, VOLUME_TRACK_W,
};
pub use popup::Popup;
pub use scroll_bar::{ScrollBar, ScrollBarAxis, MIN_THUMB_LENGTH, PAGE_FRACTION};
pub use scroll_view::{ScrollView, SCROLL_DRAG_SLOP, SCROLL_WHEEL_DELTA};
pub use slider::{Slider, DRAG_SLOP, THUMB_H, THUMB_PRESSED_SCALE, THUMB_W, TRACK_H};
pub use styles::{stylesheet, STYLESHEETS};
pub use text_area::TextArea;
pub use text_buffer::TextBuffer;
pub use text_field::TextField;
pub use toggle::{RadioGroup, RadioOption, ToggleSwitch, KNOB_PRESSED_SCALE, KNOB_TRAVEL};

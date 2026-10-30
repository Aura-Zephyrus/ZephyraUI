#![forbid(unsafe_code)]
//! winit → [`zephyra_core::input`] 的输入翻译，三个后端共用一份。
//!
//! # 为什么这个 crate 存在
//!
//! 三个宿主（`zephyra-winit` / `zephyra-winit-d3d12` / `zephyra-vulkan`）
//! **互不依赖** —— 那是仓库的硬边界（D3D11 / D3D12 / Vulkan 是三套词汇，
//! 揉进一处后「画面一空时是哪个后端的问题」就没法回答了）。但「winit 的
//! `WindowEvent` 怎么变成 core 的 `TextEvent`」这件事与后端**无关**，
//! 抄三份就意味着三份会漂移。
//!
//! 实测已经漂移过：`logical_key_name` 在 D3D11 与 D3D12 宿主各存一份拷贝，
//! Vulkan 宿主则把 `key_events` / `scroll` 直接写死成空值 —— 键盘与滚轮在
//! Linux 那条链路上从未接通过，而且没有任何东西会报错。
//!
//! 这个 crate 就是那份「后端无关的一半」。它放在 `zephyra-base/` 下是因为
//! 它依赖 winit（平台词汇），而 core / fx 都不许装。
//!
//! # 三条约束
//!
//! 1. **不做路由、不做状态机。** 这里只做词汇翻译：winit 类型 → core 类型。
//!    「哪个控件拿到这个键」是 `zephyra_core::input::router` 与焦点系统的事。
//! 2. **不碰窗口。** 唯一的宿主侧输入是 IME 的插入点坐标 —— 那必须由宿主给
//!    （core 不知道光标画在哪）。
//! 3. **`no unsafe`。** 翻译是纯数据搬运，没有理由 unsafe。
//!
//! # 用法
//!
//! ```ignore
//! // 字段
//! translator: zephyra_winit_input::Translator,
//!
//! // 事件
//! WindowEvent::ModifiersChanged(m) => translator.on_modifiers(m.state()),
//! WindowEvent::KeyboardInput { event, .. } => {
//!     translator.key_events(&event, &mut self.pending_text_events);
//! }
//! WindowEvent::Ime(ime) => {
//!     translator.ime(&ime, self.ime_caret(), &mut self.pending_text_events);
//! }
//! ```
//!
//! 三个方法都是「往 `out` 里追加」，因为一次按键确实可能同时产出**边沿**与
//! **字符**两个事件（见 [`zephyra_core::input::TextEvent`] 的文档）。
//!
//! # 为什么有 [`KeyParts`] 这一层
//!
//! `winit::event::KeyEvent` 的 `platform_specific` 字段是 `pub(crate)`，**外部
//! crate 构造不出来**。若翻译逻辑直接吃 `&KeyEvent`，那它就成了不可测代码 ——
//! 而这段逻辑正是「Ctrl+C 不该插入 c」「抬起不该插字符」这类判据的所在地。
//!
//! 所以判据写在 [`Translator::translate`]（吃 [`KeyParts`]，可直接构造），
//! [`Translator::key_events`] 只做 `&KeyEvent → KeyParts` 的薄适配。

use winit::event::{ElementState, Ime as WIme, KeyEvent};
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};

use zephyra_core::input::{Ime, Key, KeyInput, Modifiers, TextEvent};

/// 一次按键事件里**与翻译有关**的那几个字段。
///
/// 与 winit 的 `KeyEvent` 同形，但去掉了 `physical_key` / `location` /
/// `platform_specific`（翻译不用它们，而后者在 winit 里是 `pub(crate)`）。
/// 于是这个结构可以在测试里直接构造 —— 判据可见、可断言。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyParts<'a> {
    /// 按下还是抬起。
    pub pressed: bool,
    /// 操作系统的自动重复（按住不放时反复送来）。
    pub repeat: bool,
    /// 逻辑键（受键盘布局与修饰键影响后的语义）。
    pub logical: &'a WKey,
    /// 这次按键产生的可见文本。`None` = 不可打印（功能键、Ctrl 组合等）。
    pub text: Option<&'a str>,
}

impl<'a> KeyParts<'a> {
    /// 一次「无修饰、不可打印」的按下 —— 测试里最常用的底子。
    pub fn press(logical: &'a WKey) -> Self {
        Self {
            pressed: true,
            repeat: false,
            logical,
            text: None,
        }
    }

    /// 一次可打印的按下（`text` 就是按出来的那串）。
    pub fn typed(logical: &'a WKey, text: &'a str) -> Self {
        Self {
            pressed: true,
            repeat: false,
            logical,
            text: Some(text),
        }
    }

    /// 同一按键的抬起。
    pub fn release(self) -> Self {
        Self {
            pressed: false,
            ..self
        }
    }
}

impl<'a> From<&'a KeyEvent> for KeyParts<'a> {
    fn from(event: &'a KeyEvent) -> Self {
        Self {
            pressed: event.state == ElementState::Pressed,
            repeat: event.repeat,
            logical: &event.logical_key,
            text: event.text.as_deref(),
        }
    }
}

/// winit 的按键事件 → core 的文本事件。
///
/// # 为什么是有状态的
///
/// 修饰键是**独立事件**（`WindowEvent::ModifiersChanged`），而按键事件里不带
/// 修饰键快照。所以翻译器必须记住「现在按着哪些修饰键」，在按键时快照进去。
/// 这与 winit 的设计一致，也是唯一不需要宿主自己维护状态的做法。
#[derive(Debug, Default, Clone)]
pub struct Translator {
    modifiers: Modifiers,
}

impl Translator {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前修饰键快照（宿主调试用）。
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// 修饰键状态变化。宿主在 `WindowEvent::ModifiersChanged` 里调用。
    ///
    /// `ModifiersState` 只报告 ctrl / shift / alt / super 四个 —— 与
    /// [`Modifiers`] 一一对应，不做任何取舍。Caps Lock / Num Lock 不在其中，
    /// 因为它们不改变按键语义（caps 已经体现在 `text` 里）。
    pub fn on_modifiers(&mut self, state: ModifiersState) {
        self.modifiers = Modifiers::new(
            state.control_key(),
            state.shift_key(),
            state.alt_key(),
            state.super_key(),
        );
    }

    /// 一次键盘事件产出的全部 core 事件，按「**边沿在前、字符在后**」追加。
    pub fn key_events(&self, event: &KeyEvent, out: &mut Vec<TextEvent>) {
        self.translate(KeyParts::from(event), out);
    }

    /// [`Translator::key_events`] 的可测内核。
    ///
    /// # 顺序为什么重要
    ///
    /// 控件先看到 `Key(Enter)` 再看到 `Char`（如果有），就能用「按键处理过就
    /// 吞掉后面的字符」这条规则；反过来则无法撤回已经插入的字符。
    ///
    /// # 哪些事件不产出
    ///
    /// - `Released`：**仍然产出** `Key`（控件要清自己的按下集合），但**不产出**
    ///   `Char` —— 字符只在按下时进入文档。
    /// - 带 Ctrl / Alt / Super 的按下：不产出 `Char`（见 [`printable`]）。
    pub fn translate(&self, parts: KeyParts<'_>, out: &mut Vec<TextEvent>) {
        out.push(TextEvent::Key(self.key(parts)));
        if let Some(c) = printable(parts.text, parts.pressed, self.modifiers) {
            out.push(TextEvent::Char(c));
        }
    }

    /// 单个按键边沿（含键名、重复标志、修饰键快照）。
    pub fn key(&self, parts: KeyParts<'_>) -> KeyInput {
        KeyInput {
            key: key_from_logical(parts.logical),
            name: key_name(parts.logical),
            pressed: parts.pressed,
            repeat: parts.repeat,
            modifiers: self.modifiers,
        }
    }

    /// 这次按键要插入的字符（`None` = 不插入）。
    pub fn text(&self, parts: KeyParts<'_>) -> Option<char> {
        printable(parts.text, parts.pressed, self.modifiers)
    }

    /// winit 的 IME 事件 → core 的 [`Ime`]，追加到 `out`。
    ///
    /// `caret` 是**插入点在客户区的逻辑像素坐标**，由宿主提供 —— core 不知道
    /// 光标画在哪。它只在 `Ime::Enabled` 时被用到（候选窗要贴到那里）。
    ///
    /// # 四个变体的对应
    ///
    /// | winit | core | 说明 |
    /// |---|---|---|
    /// | `Enabled` | [`Ime::Start`] | 开始组字，带插入点 |
    /// | `Preedit(text, cursor)` | [`Ime::Update`] | 组字中，`cursor` 是**字节**下标对 |
    /// | `Commit(text)` | [`Ime::Commit`] | 文本进文档 |
    /// | `Disabled` | [`Ime::Cancel`] | 组字丢弃 |
    ///
    /// ⚠ winit 在 `Commit` 之前会先发一个空 `Preedit("", None)`。这里**照样**
    /// 翻成 `Update { text: "" }` 而不是吞掉：控件若把空 preedit 当成「组字
    /// 结束」的清理信号，吞掉它就会留下一条下划线。
    pub fn ime(&self, event: &WIme, caret: (f32, f32), out: &mut Vec<TextEvent>) {
        let translated = match event {
            WIme::Enabled => Ime::Start {
                x: caret.0,
                y: caret.1,
            },
            WIme::Preedit(text, cursor) => Ime::Update {
                text: text.clone(),
                cursor: *cursor,
            },
            WIme::Commit(text) => Ime::Commit { text: text.clone() },
            WIme::Disabled => Ime::Cancel,
        };
        out.push(TextEvent::Ime(translated));
    }
}

/// 「这次按键该插入什么字符」的**唯一**判据。
///
/// # 四道闸
///
/// 1. **必须按下** —— release_inserts_no_char，否则每次输入都变成两个。
/// 2. **不带 Ctrl / Alt / Super** —— Ctrl+C 复制出 'c' 是文本控件最经典的
///    bug。winit 的 `text` 通常已经为空，但这里再挡一道：不同平台的 IME 与
///    输入法组合会给出不一致的 `text`。Shift 不挡（Shift+A 就是 'A'）。
/// 3. **winit 给了 `text`** —— 功能键没有。
/// 4. **恰好一个字符** —— 多字符的 `text`（死键组合、某些 IME 的快速输入）
///    整串丢弃而不是取首字符：少插一个字符比插错一个字符容易发现，而且这类
///    输入本来就该走 [`Translator::ime`] 的 `Commit` 通道。
///
/// 另加一条：**控制字符不走这里**。换行来自 Enter、制表符来自 Tab，由控件
/// 按 [`Key`] 自己判；再从 `Char` 走一遍会让「按 Enter 插两个换行」这种 bug
/// 出现。
pub fn printable(text: Option<&str>, pressed: bool, modifiers: Modifiers) -> Option<char> {
    if !pressed {
        return None;
    }
    if modifiers.ctrl() || modifiers.alt() || modifiers.super_key() {
        return None;
    }
    let text = text?;
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if c.is_control() {
        return None;
    }
    Some(c)
}

/// winit 逻辑键 → core 的语义键。
///
/// 只映射**编辑类控件真的会解释**的那一小撮；其余全部落进 [`Key::Other`]，
/// 身份由 [`key_name`] 保留在 [`KeyInput::name`] 里。
pub fn key_from_logical(key: &WKey) -> Key {
    match key {
        WKey::Named(named) => named_key(*named),
        // 字符键在控件层是 `Char` 通道的事，语义键不需要它。
        // 死键 / 未识别键没有稳定的语义，同样归 `Other`。
        _ => Key::Other,
    }
}

fn named_key(named: NamedKey) -> Key {
    match named {
        NamedKey::Backspace => Key::Backspace,
        NamedKey::Delete => Key::Delete,
        NamedKey::Enter => Key::Enter,
        NamedKey::Escape => Key::Escape,
        // 空格是**激活键**（与 Enter 一起触发按钮点击），升为语义键而不是落
        // `Key::Other`——激活是热路径上的判断，不该退化成运行期比串。
        NamedKey::Space => Key::Space,
        NamedKey::Tab => Key::Tab,
        NamedKey::ArrowLeft => Key::Left,
        NamedKey::ArrowRight => Key::Right,
        NamedKey::ArrowUp => Key::Up,
        NamedKey::ArrowDown => Key::Down,
        NamedKey::Home => Key::Home,
        NamedKey::End => Key::End,
        NamedKey::PageUp => Key::PageUp,
        NamedKey::PageDown => Key::PageDown,
        _ => Key::Other,
    }
}

/// winit 逻辑键 → 规范键名（MCP `press_key` 与快捷键表比对的那套名字）。
///
/// 与旧宿主里两份 `logical_key_name` 逐字一致 —— 那是既有行为，改它会漂移
/// 应用侧的快捷键表。
pub fn key_name(key: &WKey) -> String {
    match key {
        WKey::Character(text) => text.to_string(),
        WKey::Named(named) => format!("{named:?}"),
        other => format!("{other:?}"),
    }
}

/// 由**规范名**造一次完整按键：按下边沿 →（可打印时）字符 → 抬起边沿。
///
/// # 谁在用
///
/// MCP 的 `press_key <name>`：AI 侧给的是 `"Enter"` / `"a"` 这种名字，而不是
/// winit 事件。三个宿主的 `inject_key` 都走这里，于是「哪个名字该插字符」
/// 只有一份判据。
///
/// # 判据
///
/// - 名字能被 [`Key::from_name`] 认出来（`Enter` / `ArrowDown` …）⇒ 只发边沿。
///   这些键的语义效果（换行、移动光标）由控件按 [`Key`] 自己执行；再从字符
///   通道走一遍就会变成「按一次 Enter 插两个换行」。
/// - 认不出来且**恰好一个**非控制字符（`"a"` / `"中"`）⇒ 边沿 + 字符。
/// - 其余（`"F5"` / `"ab"`）⇒ 只发边沿，身份留在
///   [`KeyInput::name`](zephyra_core::input::KeyInput::name) 里。
///
/// 无修饰键：MCP 的 `press_key` 参数里没有修饰键位（见 `bridge.rs` 的解析）。
pub fn injected_events(name: &str) -> Vec<TextEvent> {
    let key = Key::from_name(name);
    let edge = |pressed: bool| {
        TextEvent::Key(KeyInput {
            key,
            name: name.to_string(),
            pressed,
            repeat: false,
            modifiers: Modifiers::new(false, false, false, false),
        })
    };

    let mut events = vec![edge(true)];
    if key == Key::Other {
        let mut chars = name.chars();
        if let Some(c) = chars.next() {
            if chars.next().is_none() && !c.is_control() {
                events.push(TextEvent::Char(c));
            }
        }
    }
    events.push(edge(false));
    events
}

#[cfg(test)]
#[path = "tests/translator_tests.rs"]
mod tests;

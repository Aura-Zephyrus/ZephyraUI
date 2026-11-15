//! 单行文本输入框（TextField）的控件状态模型。
//!
//! # 蓝本（WinUI `TextBox` / Android `EditText`）
//!
//! 封装 [`TextBuffer`]（文本内容、光标、选区），并提供键盘 / 字符 /
//! IME 事件的处理入口。宿主在 `View::on_input` 中把 `TextEvent` 分发给
//! 对应 key 的 `TextField` 实例，再拿回 `text()` / `cursor()` / `selection()`
//! 填进 `view()` 的 zypui 绑定。
//!
//! # 分工
//!
//! | 关注 | 住在哪 |
//! |---|---|
//! | 文本缓冲、编辑规则、光标/选区 | [`TextBuffer`]（`text_buffer.rs`） |
//! | 键盘/字符/IME 事件分发与编辑命令 | 本模块（`TextField`） |
//! | 结构（光标节点、选区块、`focused` 类） | `ui/controls.zypui` |
//! | 光标宽度 / 选区背景 / 光标闪烁 | `controls-text-input.zstyle` |
//! | `text` / `cursor` / `selection` 值的存留 | 宿主应用 |
//!
//! # 与 `ActivationState` 的关系
//!
//! `TextField` 声明 `focusable` / `activatable` 后，Space/Enter 由宿主的
//! `ActivationState` 转成 pressed 通道，**不**由本模块处理。本模块只处理
//! 真正的文本编辑键（Backspace / Delete / 方向键 / Home / End / 可打印字符）。

use zephyra_core::input::text::{Ime, Key, KeyInput, TextEvent};
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use crate::text_buffer::TextBuffer;

/// 输入法正在组字时的中间状态（供宿主渲染候选窗）。
///
/// 宿主在 `view()` 中根据 `TextField::is_composing()` 决定是否弹出
/// [`crate::Popup`] 候选窗，并用 `anchor` 坐标定位。
#[derive(Debug, Clone, PartialEq)]
pub struct ImeComposing {
    /// 组字后的文本。
    pub text: String,
    /// 组字区间（字节下标），`None` = 不显示选区。
    pub cursor: Option<(usize, usize)>,
    /// 组字开始时的插入点坐标（DIP），宿主据此定位候选窗。
    pub anchor: Option<(f32, f32)>,
}

/// 单行文本输入框的状态模型。
#[derive(Debug, Clone, PartialEq)]
pub struct TextField {
    key: &'static str,
    label_key: Option<&'static str>,
    buffer: TextBuffer,
    disabled: bool,
    /// 输入法正在组字时的中间状态（`None` = 未组字）。
    ime_composing: Option<ImeComposing>,
}

impl TextField {
    /// 建一个空文本框。
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            label_key: None,
            buffer: TextBuffer::new(),
            disabled: false,
            ime_composing: None,
        }
    }

    /// 用给定文本建文本框，光标置末尾。
    pub fn with_text(key: &'static str, text: impl Into<String>) -> Self {
        Self {
            key,
            label_key: None,
            buffer: TextBuffer::with_text(text),
            disabled: false,
            ime_composing: None,
        }
    }

    /// 文本框的稳定 key（事件路由身份）。
    pub const fn key(&self) -> &'static str {
        self.key
    }

    /// 文本内容。
    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    /// 光标位置（字节下标）。
    pub fn cursor(&self) -> usize {
        self.buffer.cursor()
    }

    /// 选区（`None` = 无选区）。
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.buffer.selection()
    }

    /// 输入法是否正在组字。
    pub fn is_composing(&self) -> bool {
        self.ime_composing.is_some()
    }

    /// 组字中间状态（`None` = 未组字）。
    pub fn ime_composing(&self) -> Option<&ImeComposing> {
        self.ime_composing.as_ref()
    }

    /// 是否禁用。
    pub const fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// 设置禁用。
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
    }

    /// 复制当前选区到剪贴板（返回选中文本，宿主负责写入平台剪贴板）。
    ///
    /// 返回 `None` 表示无选区，无需操作。
    pub fn copy(&self) -> Option<String> {
        self.buffer
            .selection()
            .map(|(begin, end)| self.buffer.text()[begin..end].to_string())
    }

    /// 粘贴剪贴板文本到光标处（宿主从平台剪贴板读取后调用）。
    ///
    /// 若有选区，先删除选区再插入。
    pub fn paste(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.buffer.insert_str(text);
    }

    /// 设置文本（宿主外部受控模式调用），光标置末尾。
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.buffer.set_text(text);
    }

    /// 把光标置给定位置（字节下标）。
    pub fn set_cursor(&mut self, pos: usize) {
        self.buffer.set_cursor(pos);
    }

    /// 选中全部文本。
    pub fn select_all(&mut self) {
        self.buffer.select_all();
    }

    /// 把选区设为字节下标区间（宿主按字符索引换算后调用）。
    pub fn set_selection<R: std::ops::RangeBounds<usize>>(&mut self, range: R) {
        self.buffer.set_selection(range);
    }

    /// 把当前状态写入元素（每帧由宿主的 `view()` 调用）。
    ///
    /// 把 `cursor`、`selection_begin`/`selection_end`、分类名
    ///（`focused` / `disabled`）写进元素，供 `.zypui` 模板绑定。
    /// `text` 由宿主在 `view()` 中直接绑定（动态文本，不适合走静态属性）。
    pub fn apply_to_element(
        &self,
        element: zephyra_core::element::Element,
    ) -> zephyra_core::element::Element {
        let (sel_begin, sel_end) = self
            .buffer
            .selection()
            .unwrap_or((self.buffer.cursor(), self.buffer.cursor()));
        element
            .with(
                &props::CURSOR,
                PropertyValue::float(self.buffer.cursor() as f32),
            )
            .with(
                &props::SELECTION_BEGIN,
                PropertyValue::float(sel_begin as f32),
            )
            .with(&props::SELECTION_END, PropertyValue::float(sel_end as f32))
            .with(&props::DISABLED, PropertyValue::bool(self.disabled))
    }

    /// 处理键盘按键边沿。返回 `true` 表示事件已被消耗。
    ///
    /// 支持：Backspace / Delete / 方向键 / Home / End（配合 Shift 选区），
    /// 以及 Ctrl+A 全选、Ctrl+C 复制占位符。
    pub fn on_key(&mut self, input: &KeyInput) -> bool {
        if self.disabled {
            return false;
        }
        if !input.pressed {
            return false;
        }
        // 抬起不处理（重复按键靠 repeat 字段）。
        if input.repeat && matches!(input.key, Key::Backspace | Key::Delete) {
            // 退格/删除支持连续删除。
        } else if input.repeat {
            return false;
        }

        let ctrl = input.modifiers.ctrl();
        let shift = input.modifiers.shift();

        match input.key {
            Key::Backspace => {
                self.buffer.delete_backward();
                true
            }
            Key::Delete => {
                self.buffer.delete_forward();
                true
            }
            Key::Left => {
                if shift {
                    // Shift+Left：扩大选区向左
                    self.extend_selection_left();
                } else {
                    self.buffer.move_left(1);
                }
                true
            }
            Key::Right => {
                if shift {
                    self.extend_selection_right();
                } else {
                    self.buffer.move_right(1);
                }
                true
            }
            Key::Home => {
                if shift {
                    self.extend_selection_home();
                } else {
                    self.buffer.move_home();
                }
                true
            }
            Key::End => {
                if shift {
                    self.extend_selection_end();
                } else {
                    self.buffer.move_end();
                }
                true
            }
            Key::Other if ctrl && input.name.as_str() == "A" => {
                self.buffer.select_all();
                true
            }
            // Ctrl+C：复制选区到剪贴板。实际写入平台剪贴板由宿主负责
            // （`copy()` 返回选中文本，宿主调用平台 API 写入）。
            Key::Other if ctrl && input.name.as_str() == "C" => {
                let _ = self.copy();
                true
            }
            // Ctrl+V：粘贴剪贴板文本。实际读取平台剪贴板由宿主负责
            //（宿主读取后调用 `paste(text)`）。
            Key::Other if ctrl && input.name.as_str() == "V" => true,
            _ => false,
        }
    }

    /// 处理可打印字符。返回 `true` 表示事件已被消耗。
    pub fn on_char(&mut self, c: char) -> bool {
        if self.disabled {
            return false;
        }
        // 控制字符不插入。
        if c.is_control() {
            return false;
        }
        self.buffer.insert_char(c);
        true
    }

    /// 处理输入法事件。返回 `true` 表示事件已被消耗。
    pub fn on_ime(&mut self, ime: &Ime) -> bool {
        if self.disabled {
            return false;
        }
        match ime {
            Ime::Start { x, y } => {
                // 开始组字：记录锚点坐标，供宿主定位候选窗。
                self.ime_composing = Some(ImeComposing {
                    text: String::new(),
                    cursor: None,
                    anchor: Some((*x, *y)),
                });
                true
            }
            Ime::Update { text, cursor } => {
                // 组字内容更新：保留锚点，更新文本与光标。
                if let Some(comp) = self.ime_composing.as_mut() {
                    comp.text = text.clone();
                    comp.cursor = *cursor;
                } else {
                    self.ime_composing = Some(ImeComposing {
                        text: text.clone(),
                        cursor: *cursor,
                        anchor: None,
                    });
                }
                true
            }
            Ime::Commit { text } => {
                // 组字确认：文本进入文档，清除组字状态。
                self.buffer.insert_str(text);
                self.ime_composing = None;
                true
            }
            Ime::Cancel => {
                // 组字取消：丢弃组字内容。
                self.ime_composing = None;
                true
            }
        }
    }

    /// 处理任意 `TextEvent`。返回 `true` 表示事件已被消耗。
    pub fn on_text_event(&mut self, event: &TextEvent) -> bool {
        match event {
            TextEvent::Key(input) => self.on_key(input),
            TextEvent::Char(c) => self.on_char(*c),
            TextEvent::Ime(ime) => self.on_ime(ime),
        }
    }

    // ---- 选区辅助（Shift + 方向键）----

    fn extend_selection_left(&mut self) {
        let (begin, end) = self
            .buffer
            .selection()
            .unwrap_or((self.buffer.cursor(), self.buffer.cursor()));
        let new_begin = begin.saturating_sub(1);
        self.buffer.set_selection(new_begin..end);
        self.buffer.set_cursor_keep_selection(new_begin);
    }

    fn extend_selection_right(&mut self) {
        let (begin, end) = self
            .buffer
            .selection()
            .unwrap_or((self.buffer.cursor(), self.buffer.cursor()));
        let new_end = (end + 1).min(self.buffer.text().len());
        self.buffer.set_selection(begin..new_end);
        self.buffer.set_cursor_keep_selection(new_end);
    }

    fn extend_selection_home(&mut self) {
        let (_, end) = self
            .buffer
            .selection()
            .unwrap_or((self.buffer.cursor(), self.buffer.cursor()));
        self.buffer.set_selection(0..end);
        self.buffer.set_cursor_keep_selection(0);
    }

    fn extend_selection_end(&mut self) {
        let (begin, _) = self
            .buffer
            .selection()
            .unwrap_or((self.buffer.cursor(), self.buffer.cursor()));
        self.buffer.set_selection(begin..self.buffer.text().len());
        self.buffer
            .set_cursor_keep_selection(self.buffer.text().len());
    }
}

impl Default for TextField {
    fn default() -> Self {
        Self::new("text-field")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zephyra_core::input::text::Modifiers;

    fn key_press(key: Key) -> KeyInput {
        KeyInput::press(key)
    }

    fn ctrl_key_press(key: Key) -> KeyInput {
        KeyInput {
            key,
            name: key.as_str().to_string(),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::new(true, false, false, false),
        }
    }

    #[test]
    fn insert_char_via_on_char() {
        let mut f = TextField::new("tf");
        assert!(f.on_char('a'));
        assert!(f.on_char('b'));
        assert_eq!(f.text(), "ab");
        assert_eq!(f.cursor(), 2);
    }

    #[test]
    fn backspace_via_on_key() {
        let mut f = TextField::with_text("tf", "abc");
        f.set_cursor(2);
        let ev = TextEvent::key(key_press(Key::Backspace));
        assert!(f.on_text_event(&ev));
        assert_eq!(f.text(), "ac");
        assert_eq!(f.cursor(), 1);
    }

    #[test]
    fn delete_forward_via_on_key() {
        let mut f = TextField::with_text("tf", "abc");
        f.set_cursor(1);
        let ev = TextEvent::key(key_press(Key::Delete));
        assert!(f.on_text_event(&ev));
        assert_eq!(f.text(), "ac");
        assert_eq!(f.cursor(), 1);
    }

    #[test]
    fn ctrl_a_select_all() {
        let mut f = TextField::with_text("tf", "hello");
        // Key::Other with name "A" — 我们用 from_name 造一个 Other
        let mut ev = ctrl_key_press(Key::Other);
        ev.name = "A".to_string();
        assert!(f.on_text_event(&TextEvent::Key(ev)));
        assert_eq!(f.selection(), Some((0, 5)));
    }

    #[test]
    fn shift_left_extends_selection() {
        let mut f = TextField::with_text("tf", "hello");
        f.set_cursor(5);
        let ev = TextEvent::key(KeyInput {
            key: Key::Left,
            name: "ArrowLeft".to_string(),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::new(false, true, false, false),
        });
        assert!(f.on_text_event(&ev));
        // 光标在 4，选区 (4, 5)
        assert_eq!(f.cursor(), 4);
        assert_eq!(f.selection(), Some((4, 5)));
    }

    #[test]
    fn disabled_ignores_events() {
        let mut f = TextField::new("tf");
        f.set_disabled(true);
        assert!(!f.on_char('a'));
        assert!(!f.on_text_event(&TextEvent::key(key_press(Key::Backspace))));
    }

    #[test]
    fn ime_start_records_anchor() {
        let mut f = TextField::new("tf");
        let ev = TextEvent::Ime(Ime::Start { x: 100.0, y: 200.0 });
        assert!(f.on_text_event(&ev));
        assert!(f.is_composing());
        assert_eq!(f.ime_composing().unwrap().anchor, Some((100.0, 200.0)));
        assert_eq!(f.ime_composing().unwrap().text, "");
    }

    #[test]
    fn ime_update_updates_text_and_cursor() {
        let mut f = TextField::new("tf");
        f.on_text_event(&TextEvent::Ime(Ime::Start { x: 0.0, y: 0.0 }));
        f.on_text_event(&TextEvent::Ime(Ime::Update {
            text: "中".to_string(),
            cursor: Some((0, 3)),
        }));
        assert!(f.is_composing());
        assert_eq!(f.ime_composing().unwrap().text, "中");
        assert_eq!(f.ime_composing().unwrap().cursor, Some((0, 3)));
        // 文档内容不应改变（组字未确认）
        assert_eq!(f.text(), "");
    }

    #[test]
    fn ime_commit_inserts_and_clears() {
        let mut f = TextField::new("tf");
        f.on_text_event(&TextEvent::Ime(Ime::Start { x: 0.0, y: 0.0 }));
        f.on_text_event(&TextEvent::Ime(Ime::Update {
            text: "abc".to_string(),
            cursor: None,
        }));
        f.on_text_event(&TextEvent::Ime(Ime::Commit {
            text: "abc".to_string(),
        }));
        // 确认后文本进入文档，组字状态清除
        assert_eq!(f.text(), "abc");
        assert!(!f.is_composing());
        assert_eq!(f.ime_composing(), None);
    }

    #[test]
    fn ime_cancel_clears_composing() {
        let mut f = TextField::new("tf");
        f.on_text_event(&TextEvent::Ime(Ime::Start { x: 0.0, y: 0.0 }));
        assert!(f.is_composing());
        f.on_text_event(&TextEvent::Ime(Ime::Cancel));
        assert!(!f.is_composing());
        assert_eq!(f.text(), "");
    }

    #[test]
    fn ime_update_without_start_still_works() {
        // 有些宿主可能不发 Start 直接发 Update，也应能正常处理。
        let mut f = TextField::new("tf");
        f.on_text_event(&TextEvent::Ime(Ime::Update {
            text: "xyz".to_string(),
            cursor: None,
        }));
        assert!(f.is_composing());
        assert_eq!(f.ime_composing().unwrap().text, "xyz");
        assert_eq!(f.ime_composing().unwrap().anchor, None);
    }

    #[test]
    fn copy_returns_selected_text() {
        let mut f = TextField::with_text("tf", "hello");
        f.set_selection(1..4); // 选 "ell"
        assert_eq!(f.copy(), Some("ell".to_string()));
    }

    #[test]
    fn copy_returns_none_without_selection() {
        let f = TextField::with_text("tf", "hello");
        assert_eq!(f.copy(), None);
    }

    #[test]
    fn paste_inserts_at_cursor() {
        let mut f = TextField::with_text("tf", "ab");
        f.set_cursor(1);
        f.paste("XYZ");
        assert_eq!(f.text(), "aXYZb");
        assert_eq!(f.cursor(), 4);
    }

    #[test]
    fn paste_replaces_selection() {
        let mut f = TextField::with_text("tf", "hello");
        f.set_selection(1..4); // 选 "ell"
        f.paste("X");
        assert_eq!(f.text(), "hXo");
        assert_eq!(f.cursor(), 2);
    }

    #[test]
    fn paste_empty_is_noop() {
        let mut f = TextField::with_text("tf", "hello");
        f.paste("");
        assert_eq!(f.text(), "hello");
    }
}

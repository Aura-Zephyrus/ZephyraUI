//! 多行文本区域（TextArea）的控件状态模型。
//!
//! 基于 [`TextBuffer`]，支持 `\n` 换行、跨行光标移动、滚动偏移。
//!
//! # 分工
//!
//! | 关注 | 住在哪 |
//! |---|---|
//! | 多行文本缓冲、光标、选区 | [`TextBuffer`]（`text_buffer.rs`） |
//! | 多行编辑命令、滚动偏移 | 本模块（`TextArea`） |
//! | 结构（逐行文本节点、光标、滚动容器） | `ui/controls.zypui` |
//! | 行高、滚动条、选区背景 | `controls-text-input.zstyle` |
//! | 滚动偏移的存留与触摸/滚轮驱动 | 宿主应用 |

use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use crate::text_buffer::TextBuffer;

/// 多行文本区域的状态模型。
#[derive(Debug, Clone, PartialEq)]
pub struct TextArea {
    key: &'static str,
    buffer: TextBuffer,
    disabled: bool,
    /// 滚动偏移（DIP），由宿主按滚轮/拖动更新。
    scroll_offset: (f32, f32),
}

impl TextArea {
    /// 建一个空文本区。
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            buffer: TextBuffer::new(),
            disabled: false,
            scroll_offset: (0.0, 0.0),
        }
    }

    /// 用给定文本建文本区，光标置末尾。
    pub fn with_text(key: &'static str, text: impl Into<String>) -> Self {
        Self {
            key,
            buffer: TextBuffer::with_text(text),
            disabled: false,
            scroll_offset: (0.0, 0.0),
        }
    }

    /// 文本区的稳定 key（事件路由身份）。
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

    /// 是否禁用。
    pub const fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// 设置禁用。
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
    }

    /// 滚动偏移（DIP）。
    pub const fn scroll_offset(&self) -> (f32, f32) {
        self.scroll_offset
    }

    /// 设置滚动偏移（宿主按滚轮/拖动调用）。
    pub fn set_scroll_offset(&mut self, x: f32, y: f32) {
        self.scroll_offset = (x, y);
    }

    /// 把光标左移一个字符。
    pub fn move_left(&mut self) {
        self.buffer.move_left(1);
    }

    /// 把光标右移一个字符。
    pub fn move_right(&mut self) {
        self.buffer.move_right(1);
    }

    /// 把光标移到行首。
    pub fn move_home(&mut self) {
        self.buffer.move_home();
    }

    /// 把光标移到行末。
    pub fn move_end(&mut self) {
        self.buffer.move_end();
    }

    /// 把光标移到上一行。
    pub fn move_up(&mut self) {
        self.buffer.move_up();
    }

    /// 把光标移到下一行。
    pub fn move_down(&mut self) {
        self.buffer.move_down();
    }

    /// 向后删除一个字符（Delete）。
    pub fn delete_forward(&mut self) {
        self.buffer.delete_forward();
    }

    /// 向前删除一个字符（Backspace）。
    pub fn delete_backward(&mut self) {
        self.buffer.delete_backward();
    }

    /// 在光标处插入字符。
    pub fn insert_char(&mut self, ch: char) {
        self.buffer.insert_char(ch);
    }

    /// 在光标处插入字符串。
    pub fn insert_str(&mut self, s: &str) {
        self.buffer.insert_str(s);
    }

    /// 选中全部文本。
    pub fn select_all(&mut self) {
        self.buffer.select_all();
    }

    /// 设置文本（宿主外部受控模式调用），光标置末尾。
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.buffer.set_text(text);
    }

    /// 把光标置给定位置（字节下标）。
    pub fn set_cursor(&mut self, pos: usize) {
        self.buffer.set_cursor(pos);
    }

    /// 把当前状态写入元素（每帧由宿主的 `view()` 调用）。
    ///
    /// 把 `scroll_offset`、`cursor`、`selection_begin`/`selection_end`、
    /// 分类名（`disabled`）写进元素，供 `.zypui` 模板绑定。
    /// `text` 由宿主在 `view()` 中逐行绑定。
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
                &props::SELECTION_BEGIN,
                PropertyValue::float(sel_begin as f32),
            )
            .with(&props::SELECTION_END, PropertyValue::float(sel_end as f32))
            .with(
                &props::CURSOR,
                PropertyValue::float(self.buffer.cursor() as f32),
            )
            .with(
                &props::SCROLL_OFFSET_X,
                PropertyValue::float(self.scroll_offset.0),
            )
            .with(
                &props::SCROLL_OFFSET_Y,
                PropertyValue::float(self.scroll_offset.1),
            )
            .with(&props::DISABLED, PropertyValue::bool(self.disabled))
    }
}

impl Default for TextArea {
    fn default() -> Self {
        Self::new("text-area")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_textarea_is_empty() {
        let a = TextArea::new("ta");
        assert_eq!(a.text(), "");
        assert_eq!(a.cursor(), 0);
        assert_eq!(a.scroll_offset(), (0.0, 0.0));
    }

    #[test]
    fn insert_and_cursor() {
        let mut a = TextArea::new("ta");
        a.insert_char('a');
        a.insert_char('\n');
        a.insert_char('b');
        assert_eq!(a.text(), "a\nb");
        assert_eq!(a.cursor(), 3);
    }

    #[test]
    fn move_up_down_across_lines() {
        let mut a = TextArea::with_text("ta", "line1\nline2\nline3");
        a.move_end();
        assert_eq!(a.cursor(), 17); // 文本长度
        a.move_up();
        assert_eq!(a.cursor(), 6); // 第 1 行行首 "line2"
        a.move_up();
        assert_eq!(a.cursor(), 0); // 第 0 行行首 "line1"
        a.move_down();
        assert_eq!(a.cursor(), 6);
        a.move_down();
        assert_eq!(a.cursor(), 12); // 第 2 行行首 "line3"
    }

    #[test]
    fn scroll_offset_default_and_set() {
        let mut a = TextArea::new("ta");
        a.set_scroll_offset(10.0, 20.0);
        assert_eq!(a.scroll_offset(), (10.0, 20.0));
    }
}

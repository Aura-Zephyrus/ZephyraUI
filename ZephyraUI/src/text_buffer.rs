//! 可编辑文本缓冲 —— TextField / TextArea 的编辑状态模型。
//!
//! # 蓝本（WinUI `TextBox` / Android `Editable`）
//!
//! 文本缓冲持有文本内容、光标位置与选区，并提供插入、删除、选区操作。
//! 它**不碰 GPU、不碰字形图集**：纯字符串 + 字节下标运算，可在无 GPU
//! 环境下单测。
//!
//! # 分工
//!
//! | 关注 | 住在哪 |
//! |---|---|
//! | 文本内容、光标、选区、编辑规则 | 本模块（`TextBuffer`） |
//! | 结构（光标节点、选区高亮块、`focused` 类） | `ui/controls.zypui` |
//! | 光标闪烁、选区背景色、光标宽度 | `controls-*.zstyle` |
//! | 键盘/IME 事件 → 缓冲操作的分发 | 宿主 `View::on_input` |
//!
//! # 为什么光标是字节下标
//!
//! `String` 是 UTF-8，字符下标 ≠ 字节下标。控件最终要切片 `&str`，
//! 而字节边界是 `&str` 直接可用的；按字符下标操作会让每次插入/删除
//! 都走一遍 `char_indices` 换算，且 IME 的 `cursor` 也是字节下标（与
//! winit 的 `Preedit` 同源）。统一用字节下标，避免两套换算。
//!
//! # 选区语义
//!
//! `selection: Option<(usize, usize)>`，`begin <= end`，都是**字节下标**。
//! `None` = 无选区，光标即插入点。`Some((b, e))` = 选中 `[b, e)`，光标
//! 落在 `e`（或由宿主决定显示位置）。

use std::ops::RangeBounds;

/// 可编辑文本缓冲。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBuffer {
    /// 文本内容（UTF-8）。
    text: String,
    /// 光标位置（字节下标，`0..=text.len()`）。
    cursor: usize,
    /// 选区（字节下标对，`None` = 无选区）。
    selection: Option<(usize, usize)>,
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextBuffer {
    /// 建一个空缓冲。
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            selection: None,
        }
    }

    /// 用给定文本建缓冲，光标置末尾。
    pub fn with_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let len = text.len();
        Self {
            text,
            cursor: len,
            selection: None,
        }
    }

    /// 文本内容。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 光标位置（字节下标）。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 选区（`None` = 无选区）。
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.selection
    }

    /// 是否有选区。
    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    /// 把光标置给定位置（字节下标），超出范围钳到 `[0, len]`，并清选区。
    pub fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos.clamp(0, self.text.len());
        self.selection = None;
    }

    /// 把光标置给定位置（字节下标），**不清选区**。
    ///
    /// 用于 Shift+方向键扩展选区时：选区已由 `set_selection` 设好，
    /// 光标应落在选区的活跃端，不应清掉选区。
    pub fn set_cursor_keep_selection(&mut self, pos: usize) {
        self.cursor = pos.clamp(0, self.text.len());
    }

    /// 选中全部文本。
    pub fn select_all(&mut self) {
        if self.text.is_empty() {
            self.selection = None;
        } else {
            self.selection = Some((0, self.text.len()));
            self.cursor = self.text.len();
        }
    }

    /// 清除选区。
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// 把光标左移 `n` 字节（按字符边界），不越过 0。
    pub fn move_left(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        let mut pos = self.cursor;
        for _ in 0..n {
            if pos == 0 {
                break;
            }
            // 按字符边界退一位：找小于 pos 的最大 char start。
            pos = self
                .text
                .char_indices()
                .rev()
                .find(|(i, _)| *i < pos)
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
        self.cursor = pos;
        self.selection = None;
    }

    /// 把光标右移 `n` 字节（按字符边界），不越过末尾。
    pub fn move_right(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        let mut pos = self.cursor;
        for _ in 0..n {
            if pos >= self.text.len() {
                break;
            }
            // 按字符边界进一位：找大于 pos 的最小 char start。
            pos = self
                .text
                .char_indices()
                .skip_while(|(i, _)| *i <= pos)
                .nth(0)
                .map(|(i, _)| i)
                .unwrap_or(self.text.len());
            if pos == self.cursor {
                break;
            }
        }
        self.cursor = pos;
        self.selection = None;
    }

    /// 把光标移到行首（0）。
    pub fn move_home(&mut self) {
        self.cursor = 0;
        self.selection = None;
    }

    /// 把光标移到行末（text.len()）。
    pub fn move_end(&mut self) {
        self.cursor = self.text.len();
        self.selection = None;
    }

    /// 返回每行的字节范围（不含换行符 `\n`）。
    ///
    /// 空文本返回 `Vec::new()`。每行范围是 `[start, end)`，`end` 指向
    /// 换行符或文本末尾。
    pub fn line_ranges(&self) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        let mut start = 0;
        for (i, ch) in self.text.char_indices() {
            if ch == '\n' {
                ranges.push((start, i));
                start = i + 1;
            }
        }
        if start < self.text.len() || self.text.ends_with('\n') {
            // 如果文本以换行符结束，最后一行是空行
            if start == self.text.len() && !self.text.is_empty() && self.text.ends_with('\n') {
                ranges.push((start, start));
            } else if start <= self.text.len() {
                ranges.push((start, self.text.len()));
            }
        }
        ranges
    }

    /// 返回给定字节位置所在的行索引（0-based）。
    pub fn line_of_pos(&self, pos: usize) -> usize {
        let pos = pos.clamp(0, self.text.len());
        let mut line = 0;
        for (i, ch) in self.text.char_indices() {
            if i >= pos {
                break;
            }
            if ch == '\n' {
                line += 1;
            }
        }
        line
    }

    /// 返回给定行在文本中的字节起始位置。
    ///
    /// 行超出范围时返回文本长度。
    pub fn line_start(&self, line: usize) -> usize {
        let mut current_line = 0;
        let mut start = 0;
        for (i, ch) in self.text.char_indices() {
            if current_line == line {
                return start;
            }
            if ch == '\n' {
                current_line += 1;
                start = i + 1;
            }
        }
        if current_line == line {
            return start.clamp(0, self.text.len());
        }
        self.text.len()
    }

    /// 把光标移到上一行（同列近似），不越过第一行。
    pub fn move_up(&mut self) {
        let line = self.line_of_pos(self.cursor);
        if line == 0 {
            return;
        }
        let target_line = line - 1;
        let line_start = self.line_start(target_line);
        let _line_end = self
            .line_ranges()
            .get(target_line)
            .map(|(_, end)| *end)
            .unwrap_or(self.text.len());
        // 简单策略：移到上一行行首（后续可按列对齐改进）
        self.cursor = line_start;
        self.selection = None;
    }

    /// 把光标移到下一行（同列近似），不越过最后一行。
    pub fn move_down(&mut self) {
        let line = self.line_of_pos(self.cursor);
        let last_line = self.line_ranges().len().saturating_sub(1);
        if line >= last_line {
            return;
        }
        let target_line = line + 1;
        let line_start = self.line_start(target_line);
        self.cursor = line_start;
        self.selection = None;
    }

    /// 在光标处插入字符，光标随之右移。若有选区，先删除选区再插入。
    pub fn insert_char(&mut self, ch: char) {
        self.delete_selection();
        let byte_len = ch.len_utf8();
        self.text.insert(self.cursor, ch);
        self.cursor += byte_len;
    }

    /// 插入字符串，光标随之右移。若有选区，先删除选区再插入。
    pub fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// 向后删除一个字符（Delete 键）：光标处的字符被删除，光标不移。
    /// 若有选区，删除选区。
    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor >= self.text.len() {
            return;
        }
        let next = self
            .text
            .char_indices()
            .skip_while(|(i, _)| *i <= self.cursor)
            .nth(0)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        if next == self.cursor {
            return;
        }
        self.text.drain(self.cursor..next);
    }

    /// 向前删除一个字符（Backspace 键）：光标前一个字符被删除，光标左移。
    /// 若有选区，删除选区。
    pub fn delete_backward(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        // 找小于 cursor 的最大 char start，即前一个字符的起始。
        let prev = self
            .text
            .char_indices()
            .rev()
            .find(|(i, _)| *i < self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.text.drain(prev..self.cursor);
        self.cursor = prev;
    }

    /// 删除当前选区（若有），返回是否删除了非空选区。
    fn delete_selection(&mut self) -> bool {
        if let Some((begin, end)) = self.selection.take() {
            if begin < end {
                self.text.drain(begin..end);
                self.cursor = begin;
                return true;
            }
        }
        false
    }

    /// 用给定文本替换整个缓冲，光标置末尾。
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
        self.selection = None;
    }

    /// 把选区设为字节下标区间，超出范围钳到 `[0, len]`，`begin > end` 时互换。
    pub fn set_selection<R: RangeBounds<usize>>(&mut self, range: R) {
        let begin = match range.start_bound() {
            std::ops::Bound::Included(&b) => b,
            std::ops::Bound::Excluded(&b) => b + 1,
            std::ops::Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            std::ops::Bound::Included(&e) => e + 1,
            std::ops::Bound::Excluded(&e) => e,
            std::ops::Bound::Unbounded => self.text.len(),
        };
        let begin = begin.clamp(0, self.text.len());
        let end = end.clamp(begin, self.text.len());
        if begin == end {
            self.selection = None;
            self.cursor = begin;
        } else {
            self.selection = Some((begin, end));
            self.cursor = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buffer_is_empty() {
        let b = TextBuffer::new();
        assert_eq!(b.text(), "");
        assert_eq!(b.cursor(), 0);
        assert!(!b.has_selection());
    }

    #[test]
    fn insert_and_cursor_advance() {
        let mut b = TextBuffer::new();
        b.insert_char('a');
        b.insert_char('b');
        assert_eq!(b.text(), "ab");
        assert_eq!(b.cursor(), 2);
    }

    #[test]
    fn insert_str_unicode() {
        let mut b = TextBuffer::new();
        b.insert_str("中文");
        assert_eq!(b.text(), "中文");
        assert_eq!(b.cursor(), 6); // 3 字节 × 2
    }

    #[test]
    fn delete_backward_moves_cursor() {
        let mut b = TextBuffer::with_text("abc");
        b.set_cursor(2);
        b.delete_backward();
        assert_eq!(b.text(), "ac");
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn delete_forward_keeps_cursor() {
        let mut b = TextBuffer::with_text("abc");
        b.set_cursor(1);
        b.delete_forward();
        assert_eq!(b.text(), "ac");
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn selection_delete_on_insert() {
        let mut b = TextBuffer::with_text("hello");
        b.set_selection(1..4); // 选 "ell"
        b.insert_char('x');
        // 删 "ell" 剩 "ho"，光标在 1，insert 'x' → "hxo"
        assert_eq!(b.text(), "hxo");
        assert_eq!(b.cursor(), 2);
        assert!(!b.has_selection());
    }

    #[test]
    fn select_all_and_clear() {
        let mut b = TextBuffer::with_text("abc");
        b.select_all();
        assert_eq!(b.selection(), Some((0, 3)));
        b.clear_selection();
        assert!(!b.has_selection());
    }

    #[test]
    fn move_left_right_unicode_boundary() {
        let mut b = TextBuffer::with_text("a中b");
        b.move_end(); // 光标在末尾，字节下标 5
        assert_eq!(b.cursor(), 5);
        b.move_left(1); // 退到 'b' 前，字节下标 4
        assert_eq!(b.cursor(), 4);
        b.move_left(1); // 退到 '中' 前，字节下标 1
        assert_eq!(b.cursor(), 1);
        b.move_right(1); // 回到 'b' 前
        assert_eq!(b.cursor(), 4);
    }

    #[test]
    fn cursor_clamped_to_bounds() {
        let mut b = TextBuffer::with_text("hi");
        b.set_cursor(100);
        assert_eq!(b.cursor(), 2);
        b.set_cursor(usize::MAX);
        assert_eq!(b.cursor(), 2);
    }

    #[test]
    fn line_ranges_splits_on_newline() {
        let b = TextBuffer::with_text("line1\nline2\nline3");
        let ranges = b.line_ranges();
        assert_eq!(ranges, vec![(0, 5), (6, 11), (12, 17)]);
    }

    #[test]
    fn line_ranges_empty_text() {
        let b = TextBuffer::new();
        assert!(b.line_ranges().is_empty());
    }

    #[test]
    fn line_ranges_trailing_newline() {
        let b = TextBuffer::with_text("a\n");
        let ranges = b.line_ranges();
        assert_eq!(ranges, vec![(0, 1), (2, 2)]);
    }

    #[test]
    fn line_of_pos_returns_correct_line() {
        let b = TextBuffer::with_text("line1\nline2\nline3");
        assert_eq!(b.line_of_pos(0), 0);
        assert_eq!(b.line_of_pos(3), 0);
        assert_eq!(b.line_of_pos(5), 0); // '\n' 位置，仍属第 0 行
        assert_eq!(b.line_of_pos(6), 1);
        assert_eq!(b.line_of_pos(11), 1); // '\n' 位置
        assert_eq!(b.line_of_pos(12), 2);
    }

    #[test]
    fn line_start_returns_correct_offset() {
        let b = TextBuffer::with_text("line1\nline2\nline3");
        assert_eq!(b.line_start(0), 0);
        assert_eq!(b.line_start(1), 6);
        assert_eq!(b.line_start(2), 12);
        assert_eq!(b.line_start(99), 17); // 超出范围钳到末尾
    }

    #[test]
    fn move_up_down_between_lines() {
        let mut b = TextBuffer::with_text("line1\nline2\nline3");
        b.move_end(); // 光标在末尾（第 2 行末尾）
        b.move_up();
        assert_eq!(b.cursor(), 6); // 第 1 行行首 "line2"
        b.move_up();
        assert_eq!(b.cursor(), 0); // 第 0 行行首 "line1"
        b.move_up(); // 已在第一行，不动
        assert_eq!(b.cursor(), 0);
        b.move_down();
        assert_eq!(b.cursor(), 6);
        b.move_down();
        assert_eq!(b.cursor(), 12);
        b.move_down(); // 已在最后一行，不动
        assert_eq!(b.cursor(), 12);
    }
}

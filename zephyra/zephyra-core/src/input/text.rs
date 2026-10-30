//! 键盘与输入法（IME）事件词汇表。
//!
//! # 为什么这一层在核心里
//!
//! 与 [`super::pointer`] 同一个理由：翻译是**框架**职责而非应用职责。同一段
//! winit 事件要同时喂给 D3D11 / D3D12 / Vulkan 三条宿主链路，而翻译若住在宿主侧，
//! 每条链路就要各写一份 —— 实测这个坑已经踩过：`logical_key_name` 在 D3D11 与
//! D3D12 宿主各存一份拷贝，Vulkan 宿主则直接把 `key_events` 写死成空值，
//! 键盘与滚轮在那条链路上从未接通过。
//!
//! # 三层分工
//!
//! 1. **本模块**：定义纯数据词汇（`Key` / `Modifiers` / `KeyInput` / `Ime` /
//!    `TextEvent`），零依赖，禁 `unsafe`。
//! 2. **`zephyra-winit-input`**（base 下的共用 crate）：唯一一份
//!    winit → 本词汇的翻译。三个宿主共用它，翻译逻辑天然只有一份。
//! 3. **控件层**：只解释 [`TextEvent`]，不接触 winit 类型。换后端时控件零改动。
//!
//! # 一个必须在这里定死的决策
//!
//! **IME 文本用 `String` 而不是 `&'static str`**。理由：输入事件是每帧
//! `take` 走就丢的**瞬态**，不进元素树、不跨帧存活，因此不构成泄漏。先例：
//! `zephyra-fx` 的 `InputState::languages` 同样是 `Vec<String>`。
//! 若强行用 `&'static str`，winit 给的组成串就必须 leak 或 interner，而输入
//! 路径每次按键都会调用它 —— 那才是真正的堆放大器。

use std::fmt;

/// 修饰键快照。
///
/// 不是位域：修饰键的数量是固定的四个，用 `bool` 字段比位运算更直白，
/// 而且 `Copy` + `PartialEq` 全免费。判据写成语义方法而不是位掩码运算，
/// 控件里读起来是 `mods.ctrl()` 而不是 `mods & CTRL != 0`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    ctrl: bool,
    shift: bool,
    alt: bool,
    super_key: bool,
}

impl Modifiers {
    /// 一次性构造全量修饰键。
    pub const fn new(ctrl: bool, shift: bool, alt: bool, super_key: bool) -> Self {
        Self {
            ctrl,
            shift,
            alt,
            super_key,
        }
    }

    /// Ctrl（Windows 上的 Ctrl；macOS 上对应 Control）。
    #[inline]
    pub const fn ctrl(self) -> bool {
        self.ctrl
    }

    /// Shift。
    #[inline]
    pub const fn shift(self) -> bool {
        self.shift
    }

    /// Alt（macOS 上对应 Option）。
    #[inline]
    pub const fn alt(self) -> bool {
        self.alt
    }

    /// Super（Windows 上的 Win 键；macOS 上的 Command）。
    #[inline]
    pub const fn super_key(self) -> bool {
        self.super_key
    }
}

/// 控件需要解释的按键 —— 一个**刻意收窄**的集合。
///
/// # 为什么不直接暴露 winit 的 `KeyCode`
///
/// `KeyCode` 是物理布局词汇（`KeyA` 是键盘上 A 的位置，不保证按出来是 `a`），
/// 而控件要的是**语义**（左箭头、退格、回车）。把物理键码推给控件层，等于让
/// 每个文本控件自己写一遍「这个键在 QWERTY 上是左、在 AZERTY 上也是左」的
/// 映射 —— 那正是本模块要收口的东西。
///
/// 编辑类控件实际消费的按键就是下面这些；其余键走 [`Key::Other`]，身份由
/// [`KeyInput::name`] 携带（宿主给的原名，供快捷键表匹配）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Backspace,
    Delete,
    Enter,
    Escape,
    /// 空格。与 [`Key::Enter`] 一起构成**控件激活键**（WinUI 的
    /// `KeyPress::ButtonBase::IsPress`：Space 与 Enter 等价于指针按下）。
    ///
    /// 单列出来而不是塞进 [`Key::Other`]：按钮要靠它触发点击，而 `Other` 的身份
    /// 住在运行期 `name` 里、要字符串比较——激活是热路径上的语义判断，不该退化成比串。
    Space,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// 不在上面集合里的键。控件一般不直接 match 它，而是比对 [`KeyInput::name`]。
    ///
    /// 收窄成 `Other` 而不是穷举全部键位的原因：穷举会把 winit 的键位表原样
    /// 搬进 core，等于把平台词汇泄漏进来 —— 那正是这个 crate 存在的理由。
    Other,
}

impl Key {
    /// 枚举键的规范名。
    ///
    /// 这张表是**枚举**这一半的键名真相：宿主翻译、MCP `press_key`、
    /// [`Key::from_name`] 全部走它。名字取 winit 风格的 PascalCase
    /// （`ArrowDown` 而不是 `down`），因为 MCP 的 `press_key` 已经用这套名字
    /// 很久了，改名等于破坏调用方。
    ///
    /// ⚠ [`Key::Other`] 在这里是 `"Other"`，**不是**宿主给的原名。原名住在
    /// [`KeyInput::name`] —— 枚举必须能 `Copy` + `match`，装不下运行期字符串。
    pub const fn as_str(self) -> &'static str {
        match self {
            Key::Backspace => "Backspace",
            Key::Delete => "Delete",
            Key::Enter => "Enter",
            Key::Escape => "Escape",
            Key::Space => "Space",
            Key::Tab => "Tab",
            Key::Left => "ArrowLeft",
            Key::Right => "ArrowRight",
            Key::Up => "ArrowUp",
            Key::Down => "ArrowDown",
            Key::Home => "Home",
            Key::End => "End",
            Key::PageUp => "PageUp",
            Key::PageDown => "PageDown",
            Key::Other => "Other",
        }
    }

    /// 由规范名反查（[`Key::as_str`] 的逆）。
    ///
    /// 不认识的名字一律落进 [`Key::Other`] —— 区分「真的叫 Other」与「不认识」
    /// 在控件层没有任何用处（两者都只看 [`KeyInput::name`]），多一个分支只会
    /// 逼调用方多写一行。
    pub fn from_name(name: &str) -> Key {
        match name {
            "Backspace" => Key::Backspace,
            "Delete" => Key::Delete,
            "Enter" | "NumpadEnter" => Key::Enter,
            "Escape" => Key::Escape,
            "Space" => Key::Space,
            "Tab" => Key::Tab,
            "ArrowLeft" => Key::Left,
            "ArrowRight" => Key::Right,
            "ArrowUp" => Key::Up,
            "ArrowDown" => Key::Down,
            "Home" => Key::Home,
            "End" => Key::End,
            "PageUp" => Key::PageUp,
            "PageDown" => Key::PageDown,
            _ => Key::Other,
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一次按键边沿事件。
///
/// # 为什么既有 `key` 又有 `name`
///
/// 两者回答不同的问题：
///
/// - `key` 是**语义**（`Key::Backspace`），控件 `match` 它做编辑命令。
/// - `name` 是**身份**（`"F5"` / `"a"` / `"Escape"`），快捷键表比对它。
///
/// 一个字段做不到：`Key` 必须 `Copy` 才能无脑 `match`，而键名是运行期字符串
/// （winit 的 `Key::Character` 本身就是 `SmolStr`）。合并只能二选一 ——
/// 要么丢掉快捷键的身份，要么让编辑命令走字符串比较。
///
/// 成本：每次按键一次小分配（`name`）。按键是**用户节奏**（每秒个位数），
/// 不在每帧热路径上；没有按键的帧零分配。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInput {
    /// 语义键。控件 match 这个。
    pub key: Key,
    /// 规范键名。`Key::Other` 时是宿主给的原名（"F5"），其余等于 `key.as_str()`。
    pub name: String,
    /// `true` = 按下，`false` = 抬起。
    pub pressed: bool,
    /// 按住时操作系统送来的自动重复。
    ///
    /// 文本控件要消费它（长按退格连续删除），而快捷键表通常要忽略它
    /// （长按 Ctrl+S 不该连发保存）。
    pub repeat: bool,
    /// 按下/抬起那一刻的修饰键快照。
    pub modifiers: Modifiers,
}

impl KeyInput {
    /// 便捷构造：无修饰、非重复的单次按下。
    pub fn press(key: Key) -> Self {
        Self {
            key,
            name: key.as_str().to_string(),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::new(false, false, false, false),
        }
    }

    /// 便捷构造：无修饰的抬起。
    pub fn release(key: Key) -> Self {
        Self {
            key,
            name: key.as_str().to_string(),
            pressed: false,
            repeat: false,
            modifiers: Modifiers::new(false, false, false, false),
        }
    }
}

/// 输入法事件。
///
/// # 三个阶段
///
/// 一次完整的中文/日文输入走 `Start` → `Update`* → (`Commit` | `Cancel`)：
///
/// - [`Ime::Start`]：输入法**开始组字**。宿主此刻要把候选窗贴到插入点，
///   所以带坐标（逻辑像素，与指针坐标同源）。
/// - [`Ime::Update`]：组字内容或光标位置变化。`cursor` 是一对**字节**下标
///   `(begin, end)`，与 winit 的 `Preedit` 同源：`None` = 组字中不显示光标，
///   `Some((b, e))` = 光标/选区的起止。用字节而不是字符，是因为控件最终要
///   切片 `String`，而字节边界是 `&str` 直接可用的；换成字符下标反而每帧都要
///   走一遍 `char_indices` 换算。
/// - [`Ime::Commit`]：用户确认，组字文本进入文档。宿主此后不再送 `Update`。
/// - [`Ime::Cancel`]：组字被丢弃（用户按 Esc、切输入法、或输入框失焦）。
///
/// # 为什么不用 `&'static str`
///
/// 见模块文档末尾：输入事件是每帧取走即丢的瞬态，`String` 是安全且正确的
/// 选择；强行用 `&'static str` 只会让每次按键都 leak 一块内存。
///
/// 注：`Start` 带 `f32` 坐标，所以只能 `PartialEq` 不能 `Eq`（浮点没有全序）。
#[derive(Debug, Clone, PartialEq)]
pub enum Ime {
    /// 开始组字。`x` / `y` 是插入点在窗口客户区的坐标（逻辑像素）。
    Start { x: f32, y: f32 },
    /// 组字内容更新。`cursor` 是 `(begin, end)` **字节**下标对；`None` = 不显示光标。
    Update {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    /// 组字确认，文本进入文档。
    Commit { text: String },
    /// 组字取消，内容丢弃。
    Cancel,
}

/// 一个文本输入事件 —— 键盘边沿、字符、输入法三者的和。
///
/// # 为什么三者要分开
///
/// 「按下 A 键」与「文档里出现 `a`」不是一回事：前者在 Ctrl+A 时含义是全选，
/// 后者才是文本插入。合并成一个事件会逼控件自己判 `if !mods.ctrl() { insert(...) }`
/// —— 那是每个控件都要重写一遍的坑，所以在词汇层就分开。
#[derive(Debug, Clone, PartialEq)]
pub enum TextEvent {
    /// 键盘边沿（按下或抬起，含自动重复）。
    Key(KeyInput),
    /// 一个可打印字符（宿主已按修饰键筛过：Ctrl+C 不会走到这里）。
    ///
    /// 与 [`TextEvent::Key`] 正交：同一次按键可能同时产出两者（物理键 → 语义
    /// 边沿；对应字符 → 文本插入），控件按需消费其一或两者。
    Char(char),
    /// 输入法事件。
    Ime(Ime),
}

impl TextEvent {
    /// 便捷构造：一次按键边沿。
    pub const fn key(input: KeyInput) -> Self {
        TextEvent::Key(input)
    }

    /// 便捷构造：一个可打印字符。
    pub const fn ch(c: char) -> Self {
        TextEvent::Char(c)
    }
}

#[cfg(test)]
#[path = "tests/text_tests.rs"]
mod tests;

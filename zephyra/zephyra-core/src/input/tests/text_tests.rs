// 本文件由 text.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;

// ─────────────────────── 键名表：两个方向都必须自洽 ───────────────────────

/// `from_name` 与 `as_str` 是同一张表的两半。这条测的是**双向闭合**：
/// 每个键 `from_name(as_str(k)) == k`。漏一个分支就会得到 `Other`——
/// 而 `Other` 在控件层只会让快捷键失效，不会报错。
#[test]
fn 每个命名键的规范名都能反查回自己() {
    const NAMED: &[Key] = &[
        Key::Backspace,
        Key::Delete,
        Key::Enter,
        Key::Escape,
        Key::Tab,
        Key::Left,
        Key::Right,
        Key::Up,
        Key::Down,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
    ];

    for &k in NAMED {
        let name = k.as_str();
        assert_ne!(name, "Other", "命名键不该叫 Other");
        assert_eq!(Key::from_name(name), k, "键名 `{name}` 反查不回自己");
    }
}

/// 反向：`as_str` 产出的每个名字都必须被 `from_name` 认出来。
///
/// 这一条是上一条的补集判据 —— 上一条遍历的是**写死的键**，漏掉的是
/// `as_str` 有分支而 `NAMED` 没列出的键。
#[test]
fn as_str_产出的名字全在反查表里() {
    const ALL: &[Key] = &[
        Key::Backspace,
        Key::Delete,
        Key::Enter,
        Key::Escape,
        Key::Tab,
        Key::Left,
        Key::Right,
        Key::Up,
        Key::Down,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
        Key::Other,
    ];

    for &k in ALL {
        let back = Key::from_name(k.as_str());
        // Other 是「不认识」的收纳箱，反查回来仍是 Other，不算失败。
        if k == Key::Other {
            assert_eq!(back, Key::Other);
        } else {
            assert_eq!(back, k);
        }
    }
}

#[test]
fn 小键盘回车与小键盘的其它名字也能进_enter() {
    assert_eq!(Key::from_name("NumpadEnter"), Key::Enter);
    assert_eq!(Key::Enter.as_str(), "Enter");
}

#[test]
fn 不认识的名字落进_other_而不是报错() {
    assert_eq!(Key::from_name("F5"), Key::Other);
    assert_eq!(Key::from_name(""), Key::Other);
    // 大小写敏感是**有意的**：这张表与 MCP 的 press_key 共用，宽进会让
    // "arrowdown" 与 "ArrowDown" 两条路产出不同行为，排查时无从下手。
    assert_eq!(Key::from_name("arrowdown"), Key::Other);
}

#[test]
fn 快捷键身份住在_name_而不是枚举上() {
    // `Key::Other` 的 `as_str()` 是 "Other" —— 它只说明「不在这 13 个里」。
    // 真正的身份（"F5"）在 `KeyInput::name`，由宿主填写。
    let f5 = KeyInput {
        key: Key::Other,
        name: "F5".to_string(),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::default(),
    };
    assert_eq!(f5.key.as_str(), "Other");
    assert_eq!(f5.name, "F5");
}

#[test]
fn 命名键的_name_与_as_str_一致() {
    // 便捷构造器让两者同源；宿主翻译侧也有同样的一条门禁（zephyra-winit-input）。
    for &k in &[Key::Enter, Key::Left, Key::PageDown] {
        let input = KeyInput::press(k);
        assert_eq!(input.name, k.as_str());
    }
}

#[test]
fn display_与_as_str_一致() {
    use std::fmt::Write;

    for &k in &[Key::Tab, Key::Left, Key::PageDown, Key::Other] {
        let mut s = String::new();
        let _ = write!(s, "{k}");
        assert_eq!(s, k.as_str());
    }
}

// ───────────────────────── 修饰键：语义方法不是位运算 ─────────────────────────

#[test]
fn 修饰键快照逐位如实上报() {
    let all = Modifiers::new(true, true, true, true);
    assert!(all.ctrl());
    assert!(all.shift());
    assert!(all.alt());
    assert!(all.super_key());

    let none = Modifiers::default();
    assert!(!none.ctrl());
    assert!(!none.shift());
    assert!(!none.alt());
    assert!(!none.super_key());
}

/// 构造函数的参数顺序是 `(ctrl, shift, alt, super)`。写错顺序不会报错，
/// 只会让 Ctrl 变成 Shift —— 这条把顺序钉住。
#[test]
fn 构造函数参数顺序是_ctrl_shift_alt_super() {
    let only_ctrl = Modifiers::new(true, false, false, false);
    assert!(only_ctrl.ctrl());
    assert!(!only_ctrl.shift());

    let only_super = Modifiers::new(false, false, false, true);
    assert!(only_super.super_key());
    assert!(!only_super.ctrl());
}

// ─────────────────────────── KeyInput：边沿 + 重复 + 修饰 ───────────────────────────

#[test]
fn press_与_release_的默认快照无修饰且非重复() {
    let p = KeyInput::press(Key::Backspace);
    assert!(p.pressed);
    assert!(!p.repeat);
    assert_eq!(p.modifiers, Modifiers::default());

    let r = KeyInput::release(Key::Backspace);
    assert!(!r.pressed);
    assert!(!r.repeat);
    assert_eq!(r.modifiers, Modifiers::default());
}

#[test]
fn 自动重复与手动按下可区分() {
    let held = KeyInput {
        repeat: true,
        ..KeyInput::press(Key::Delete)
    };
    assert!(held.pressed);
    assert!(
        held.repeat,
        "按住重复必须能和首次按下区分 —— 长按 Ctrl+S 不该连发保存"
    );
}

#[test]
fn 修饰键随按键快照一起送达() {
    let save = KeyInput {
        key: Key::Other,
        name: "S".to_string(),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::new(true, false, false, false),
    };
    assert!(save.modifiers.ctrl());
    assert_eq!(save.name, "S");
}

// ─────────────────────────────── Ime：三个阶段 ───────────────────────────────

#[test]
fn ime_start_带插入点坐标() {
    let start = Ime::Start { x: 120.5, y: 64.0 };
    match start {
        Ime::Start { x, y } => {
            assert!((x - 120.5).abs() < f32::EPSILON);
            assert!((y - 64.0).abs() < f32::EPSILON);
        }
        _ => panic!("Start 被解成了别的变体"),
    }
}

#[test]
fn ime_update_的_cursor_是字节下标对() {
    // "中文" = 2 个字符 / 6 个字节。`Some((3, 3))` 表示光标在两个汉字之间。
    let update = Ime::Update {
        text: "中文".to_string(),
        cursor: Some((3, 3)),
    };
    match update {
        Ime::Update { text, cursor } => {
            assert_eq!(text.chars().count(), 2);
            assert_eq!(cursor, Some((3, 3)));
            assert_eq!(text.len(), 6, "字节与字符数必须能被区分开");
            // 字节下标必须落在字符边界上才切得动。
            assert!(text.is_char_boundary(3));
        }
        _ => panic!("Update 被解成了别的变体"),
    }
}

#[test]
fn ime_update_可以不带光标() {
    // winit 的 `Preedit(text, None)` = 组字中但隐藏光标。
    let update = Ime::Update {
        text: "a".to_string(),
        cursor: None,
    };
    assert!(matches!(update, Ime::Update { cursor: None, .. }));
}

#[test]
fn ime_文本能装下任意长度且不要求_static() {
    // 这条测的是**类型形状**而不是数值：`Ime` 的文本字段必须是 `String`。
    // 若哪天有人改回 `&'static str`，这个函数编译不过 —— 而改动者会看见
    // 模块文档里「输入事件是瞬态，不构成泄漏」的理由。
    let long: String = "a".repeat(4096);
    let commit = Ime::Commit { text: long.clone() };
    match commit {
        Ime::Commit { text } => assert_eq!(text.len(), 4096),
        _ => panic!("Commit 被解成了别的变体"),
    }
}

#[test]
fn ime_cancel_不带载荷() {
    assert_eq!(Ime::Cancel, Ime::Cancel);
    assert_ne!(
        Ime::Cancel,
        Ime::Commit {
            text: String::new()
        }
    );
}

// ────────────────────────────── TextEvent：三者正交 ──────────────────────────────

#[test]
fn key_与_char_是正交的两条通道() {
    let edge = TextEvent::key(KeyInput {
        modifiers: Modifiers::new(true, false, false, false),
        ..KeyInput::press(Key::Other)
    });
    let text = TextEvent::ch('a');

    assert_ne!(edge, text, "物理键边沿与文本插入必须是不同事件");
    match edge {
        TextEvent::Key(k) => assert_eq!(k.key, Key::Other),
        _ => panic!("key() 没造出 Key 变体"),
    }
    assert_eq!(text, TextEvent::Char('a'));
}

#[test]
fn 不可打印字符也能走_char_通道() {
    // 宿主负责按修饰键筛：Ctrl+C 不会送 Char('c')。但换行/制表符这类
    // 控制字符在某些 IME 组合下会从字符通道过来，词汇层不做二次过滤。
    assert_eq!(TextEvent::ch('\n'), TextEvent::Char('\n'));
    assert_eq!(TextEvent::ch('\t'), TextEvent::Char('\t'));
}

#[test]
fn ime_事件能塞进同一张表() {
    // 这张表就是 `InputState::text_events` 的形状：三种事件共用一个队列。
    // 用 `Vec::from(数组)` 而不是 `vec![]` —— 后者在 clippy 眼里是「没必要的
    // Vec」，而这里**要的就是** Vec（队列类型本身是被测对象）。
    let events: Vec<TextEvent> = Vec::from([
        TextEvent::Ime(Ime::Start { x: 0.0, y: 0.0 }),
        TextEvent::Ime(Ime::Update {
            text: "ni".into(),
            cursor: Some((2, 2)),
        }),
        TextEvent::Ime(Ime::Commit { text: "你".into() }),
    ]);

    assert_eq!(events.len(), 3);
    assert!(matches!(events[0], TextEvent::Ime(Ime::Start { .. })));
}

#[test]
fn clone_是深拷贝而不是共用缓冲() {
    let a = TextEvent::Ime(Ime::Commit {
        text: "原文".into(),
    });
    let b = a.clone();
    // `String` 的 Clone 是深拷贝；这条不是测标准库，而是钉住「Ime 里的文本
    // 是可变的独立缓冲」这个前提 —— 控件要能就地编辑它。
    assert_eq!(a, b);
    if let (TextEvent::Ime(Ime::Commit { text: ta }), TextEvent::Ime(Ime::Commit { text: tb })) =
        (&a, &b)
    {
        assert_ne!(ta.as_ptr(), tb.as_ptr(), "Clone 不该共享缓冲");
    } else {
        panic!("变体对不上");
    }
}

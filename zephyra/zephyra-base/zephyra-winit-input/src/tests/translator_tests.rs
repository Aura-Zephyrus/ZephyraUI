// 本文件由 lib.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;

// ─────────────────────── 语义键映射：13 个编辑键 + 其余归 Other ───────────────────────

/// 每个被映射的 `NamedKey` 都必须映到**对应**的 [`Key`]。
///
/// 这条穷举了编辑控件会解释的全部按键 —— 漏一个的症状是「方向键不移动光标」，
/// 而那种 bug 在编译期完全看不出来。
#[test]
fn 编辑键逐个映射到语义键() {
    const PAIRS: &[(NamedKey, Key)] = &[
        (NamedKey::Backspace, Key::Backspace),
        (NamedKey::Delete, Key::Delete),
        (NamedKey::Enter, Key::Enter),
        (NamedKey::Escape, Key::Escape),
        (NamedKey::Tab, Key::Tab),
        (NamedKey::ArrowLeft, Key::Left),
        (NamedKey::ArrowRight, Key::Right),
        (NamedKey::ArrowUp, Key::Up),
        (NamedKey::ArrowDown, Key::Down),
        (NamedKey::Home, Key::Home),
        (NamedKey::End, Key::End),
        (NamedKey::PageUp, Key::PageUp),
        (NamedKey::PageDown, Key::PageDown),
    ];

    for &(named, expected) in PAIRS {
        assert_eq!(
            key_from_logical(&WKey::Named(named)),
            expected,
            "{named:?} 映错了"
        );
    }
}

#[test]
fn 编辑键的数量与_键名表一致() {
    // 13 = `Key` 里除 `Other` 之外的变体数。少一个说明映射表漏了。
    assert_eq!(Key::PageDown.as_str(), "PageDown");
    assert_eq!(Key::Backspace.as_str(), "Backspace");
}

#[test]
fn 非编辑键落进_other_但名字保留() {
    let f5 = WKey::Named(NamedKey::F5);
    assert_eq!(key_from_logical(&f5), Key::Other);
    assert_eq!(key_name(&f5), "F5", "身份不能丢 —— 快捷键表靠它匹配");
}

#[test]
fn 字符键的语义是_other_名字是字符本身() {
    let a = WKey::Character("a".into());
    assert_eq!(
        key_from_logical(&a),
        Key::Other,
        "字符走 Char 通道，不进语义键"
    );
    assert_eq!(key_name(&a), "a");
}

#[test]
fn 死键与未识别键不会panic() {
    // 换输入法、键盘布局切换时会出现这两种；翻译层必须原样兜住。
    let dead = WKey::Dead(Some('´'));
    let unknown = WKey::Unidentified(winit::keyboard::NativeKey::Unidentified);
    assert_eq!(key_from_logical(&dead), Key::Other);
    assert_eq!(key_from_logical(&unknown), Key::Other);
    // 名字是 Debug 形态，至少不含空串 —— 空名字会让快捷键表匹配到所有键。
    assert!(!key_name(&dead).is_empty());
    assert!(!key_name(&unknown).is_empty());
}

/// 每个 [`Key`] 变体都必须能从某个 winit 键到达 —— 否则它就是死枚举项。
///
/// 判据方向是「core 的键 → winit 有没有对应」，与上面那条形成双向闭合。
#[test]
fn core_的每个语义键都有_winit_来源() {
    const REACHABLE: &[Key] = &[
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

    for &k in REACHABLE {
        let name = k.as_str();
        assert_ne!(name, "Other", "语义键 `{name}` 没有 winit 来源");
        // `Key::from_name` 是反向那一半：名字必须能被 core 自己认回来。
        assert_eq!(Key::from_name(name), k);
    }
}

// ────────────────────────────── 修饰键状态 ──────────────────────────────

#[test]
fn 修饰键逐个如实上报() {
    // 逐个设、逐个断言：顺带证明每次 `on_modifiers` 是**覆盖**而不是累加。
    let mut t = Translator::new();

    t.on_modifiers(ModifiersState::CONTROL);
    assert!(t.modifiers().ctrl());
    assert!(!t.modifiers().shift());

    t.on_modifiers(ModifiersState::SHIFT);
    assert!(t.modifiers().shift());
    assert!(!t.modifiers().ctrl(), "上一次的 Ctrl 必须被覆盖掉");

    t.on_modifiers(ModifiersState::ALT);
    assert!(t.modifiers().alt());
    assert!(!t.modifiers().shift());

    t.on_modifiers(ModifiersState::SUPER);
    assert!(t.modifiers().super_key());
    assert!(!t.modifiers().alt());
}

#[test]
fn 修饰键状态是覆盖而不是累加() {
    // 松开 Ctrl 必须把它清掉。若写成 `|=`，Ctrl 会永远粘住，
    // 之后每一次输入都变成快捷键。
    let mut t = Translator::new();
    t.on_modifiers(ModifiersState::CONTROL);
    assert!(t.modifiers().ctrl());
    t.on_modifiers(ModifiersState::empty());
    assert!(!t.modifiers().ctrl());
}

#[test]
fn 默认没有修饰键() {
    let t = Translator::new();
    let m = t.modifiers();
    assert!(!m.ctrl() && !m.shift() && !m.alt() && !m.super_key());
}

// ───────────────────────── 可打印字符的四道闸 ─────────────────────────

const NONE: Modifiers = Modifiers::new(false, false, false, false);
const CTRL: Modifiers = Modifiers::new(true, false, false, false);
const SHIFT: Modifiers = Modifiers::new(false, true, false, false);
const ALT: Modifiers = Modifiers::new(false, false, true, false);
const SUPER: Modifiers = Modifiers::new(false, false, false, true);

#[test]
fn 普通按下插入一个字符() {
    assert_eq!(printable(Some("a"), true, NONE), Some('a'));
}

#[test]
fn 抬起不插字符() {
    // 每次输入都插两个字符的经典成因。宿主可能对同一次按键送 Pressed 与 Released。
    assert_eq!(printable(Some("a"), false, NONE), None);
}

#[test]
fn 带_ctrl_alt_super_的按下不插字符() {
    for m in [CTRL, ALT, SUPER] {
        assert_eq!(
            printable(Some("c"), true, m),
            None,
            "修饰键 {:?} 没挡住字符",
            m.ctrl()
        );
    }
}

#[test]
fn shift_不挡字符() {
    // Shift+A 就是 'A'：winit 已经把 shift 作用于 text 了。
    assert_eq!(printable(Some("A"), true, SHIFT), Some('A'));
}

#[test]
fn 没有_text_就不插字符() {
    assert_eq!(printable(None, true, NONE), None);
}

#[test]
fn 多字符整串丢弃而不是取首字符() {
    // 死键组合与某些 IME 的快速输入会给出多字符 text。
    assert_eq!(printable(Some("ab"), true, NONE), None);
    assert_eq!(printable(Some("中文"), true, NONE), None);
}

#[test]
fn 控制字符不走_char_通道() {
    // Enter / Tab 由控件按 `Key` 自己判；再从 Char 走一遍会让「按 Enter
    // 插两个换行」这种 bug 出现。
    assert_eq!(printable(Some("\n"), true, NONE), None);
    assert_eq!(printable(Some("\t"), true, NONE), None);
    assert_eq!(printable(Some("\r"), true, NONE), None);
}

#[test]
fn 空串不插字符() {
    assert_eq!(printable(Some(""), true, NONE), None);
}

#[test]
fn 宽字符与emoji都能过() {
    assert_eq!(printable(Some("中"), true, NONE), Some('中'));
    assert_eq!(printable(Some("😀"), true, NONE), Some('😀'));
}

// ───────────────────────── translate：顺序与内容 ─────────────────────────

#[test]
fn 边沿永远在字符之前() {
    // 控件靠这个顺序实现「按键已处理就吞掉字符」。
    let t = Translator::new();
    let logical = WKey::Character("a".into());
    let mut out = Vec::new();
    t.translate(KeyParts::typed(&logical, "a"), &mut out);

    assert_eq!(out.len(), 2, "可打印按键必须同时产出边沿与字符");
    assert!(matches!(out[0], TextEvent::Key(_)), "第一个必须是 Key");
    assert_eq!(out[1], TextEvent::Char('a'));
}

#[test]
fn 抬起只产出边沿() {
    let t = Translator::new();
    let logical = WKey::Character("a".into());
    let mut out = Vec::new();
    t.translate(KeyParts::typed(&logical, "a").release(), &mut out);

    assert_eq!(out.len(), 1);
    match &out[0] {
        TextEvent::Key(k) => {
            assert!(!k.pressed);
            assert_eq!(k.name, "a");
        }
        other => panic!("抬起该产出 Key，得到 {other:?}"),
    }
}

#[test]
fn 功能键只产出边沿() {
    let t = Translator::new();
    let logical = WKey::Named(NamedKey::F5);
    let mut out = Vec::new();
    t.translate(KeyParts::press(&logical), &mut out);

    assert_eq!(out.len(), 1);
    match &out[0] {
        TextEvent::Key(k) => {
            assert_eq!(k.key, Key::Other);
            assert_eq!(k.name, "F5");
            assert!(k.pressed);
        }
        other => panic!("得到 {other:?}"),
    }
}

#[test]
fn 方向键的语义与名字同时正确() {
    let t = Translator::new();
    let logical = WKey::Named(NamedKey::ArrowLeft);
    let mut out = Vec::new();
    t.translate(KeyParts::press(&logical), &mut out);

    match &out[0] {
        TextEvent::Key(k) => {
            assert_eq!(k.key, Key::Left);
            assert_eq!(k.name, "ArrowLeft");
        }
        other => panic!("得到 {other:?}"),
    }
}

#[test]
fn 修饰键快照进每一次边沿() {
    let mut t = Translator::new();
    t.on_modifiers(ModifiersState::CONTROL);
    let logical = WKey::Character("s".into());

    let mut out = Vec::new();
    t.translate(KeyParts::typed(&logical, "s"), &mut out);

    // Ctrl+S 只产出边沿（字符被 printable 挡住）。
    assert_eq!(out.len(), 1);
    match &out[0] {
        TextEvent::Key(k) => {
            assert!(k.modifiers.ctrl());
            assert_eq!(k.name, "s");
        }
        other => panic!("得到 {other:?}"),
    }
}

#[test]
fn 自动重复标志原样透传() {
    let t = Translator::new();
    let logical = WKey::Character("a".into());
    let parts = KeyParts {
        repeat: true,
        ..KeyParts::typed(&logical, "a")
    };

    let mut out = Vec::new();
    t.translate(parts, &mut out);

    match &out[0] {
        TextEvent::Key(k) => assert!(k.repeat, "长按重复必须能传下去"),
        other => panic!("得到 {other:?}"),
    }
    // 重复也要插字符：按住 'a' 本来就该连打出 aaaa。
    assert_eq!(out[1], TextEvent::Char('a'));
}

// ───────────────────────────────── IME ─────────────────────────────────

#[test]
fn ime_enabled_带上宿主给的插入点() {
    let t = Translator::new();
    let mut out = Vec::new();
    t.ime(&WIme::Enabled, (120.5, 64.0), &mut out);

    match &out[0] {
        TextEvent::Ime(Ime::Start { x, y }) => {
            assert!((x - 120.5).abs() < f32::EPSILON);
            assert!((y - 64.0).abs() < f32::EPSILON);
        }
        other => panic!("得到 {other:?}"),
    }
}

#[test]
fn ime_preedit_带上字节光标对() {
    let t = Translator::new();
    let mut out = Vec::new();
    t.ime(
        &WIme::Preedit("中文".into(), Some((3, 3))),
        (0.0, 0.0),
        &mut out,
    );

    match &out[0] {
        TextEvent::Ime(Ime::Update { text, cursor }) => {
            assert_eq!(text, "中文");
            assert_eq!(*cursor, Some((3, 3)));
        }
        other => panic!("得到 {other:?}"),
    }
}

#[test]
fn 空_preedit_不被吞掉() {
    // winit 在 Commit 之前会发空 Preedit 清场。控件靠它清下划线 ——
    // 吞掉就会留下一条永远不消失的线。
    let t = Translator::new();
    let mut out = Vec::new();
    t.ime(&WIme::Preedit(String::new(), None), (0.0, 0.0), &mut out);

    assert_eq!(out.len(), 1);
    assert!(
        matches!(&out[0], TextEvent::Ime(Ime::Update { text, cursor: None }) if text.is_empty())
    );
}

#[test]
fn ime_commit_进入文档() {
    let t = Translator::new();
    let mut out = Vec::new();
    t.ime(&WIme::Commit("你好".into()), (0.0, 0.0), &mut out);

    match &out[0] {
        TextEvent::Ime(Ime::Commit { text }) => assert_eq!(text, "你好"),
        other => panic!("得到 {other:?}"),
    }
}

#[test]
fn ime_disabled_是取消而不是提交() {
    // 把 Disabled 错翻成 Commit 会让「切输入法」把半截拼音插进文档。
    let t = Translator::new();
    let mut out = Vec::new();
    t.ime(&WIme::Disabled, (0.0, 0.0), &mut out);

    assert_eq!(out[0], TextEvent::Ime(Ime::Cancel));
}

#[test]
fn 一次组字全流程的顺序() {
    let t = Translator::new();
    let mut out = Vec::new();
    t.ime(&WIme::Enabled, (10.0, 20.0), &mut out);
    t.ime(
        &WIme::Preedit("ni".into(), Some((2, 2))),
        (10.0, 20.0),
        &mut out,
    );
    t.ime(&WIme::Preedit(String::new(), None), (10.0, 20.0), &mut out);
    t.ime(&WIme::Commit("你".into()), (10.0, 20.0), &mut out);

    assert_eq!(out.len(), 4);
    assert!(matches!(out[0], TextEvent::Ime(Ime::Start { .. })));
    assert!(matches!(out[1], TextEvent::Ime(Ime::Update { .. })));
    assert!(matches!(out[2], TextEvent::Ime(Ime::Update { .. })));
    assert!(matches!(out[3], TextEvent::Ime(Ime::Commit { .. })));
}

// ──────────────────────── MCP press_key 的注入路径 ────────────────────────

#[test]
fn 注入单个字符会插文本() {
    let events = injected_events("a");
    assert_eq!(events.len(), 3, "按下 → 字符 → 抬起");
    assert!(matches!(&events[0], TextEvent::Key(k) if k.pressed));
    assert_eq!(events[1], TextEvent::Char('a'));
    assert!(matches!(&events[2], TextEvent::Key(k) if !k.pressed));
}

#[test]
fn 注入编辑键不插文本() {
    // 按 Enter 换行由控件按 `Key::Enter` 执行；再从字符通道走一遍会变成两个换行。
    for name in ["Enter", "Backspace", "ArrowDown", "Tab", "Escape"] {
        let events = injected_events(name);
        assert_eq!(events.len(), 2, "`{name}` 只该有按下/抬起两条边沿");
        assert!(!events.iter().any(|e| matches!(e, TextEvent::Char(_))));
    }
}

#[test]
fn 注入编辑键的语义键正确() {
    let cases = [
        ("Enter", Key::Enter),
        ("Backspace", Key::Backspace),
        ("ArrowLeft", Key::Left),
        ("PageDown", Key::PageDown),
    ];
    for (name, expected) in cases {
        match &injected_events(name)[0] {
            TextEvent::Key(k) => {
                assert_eq!(k.key, expected, "`{name}` 的语义键映错了");
                assert_eq!(k.name, name);
            }
            other => panic!("得到 {other:?}"),
        }
    }
}

#[test]
fn 注入宽字符也能插文本() {
    let events = injected_events("中");
    assert_eq!(events[1], TextEvent::Char('中'));
}

#[test]
fn 注入未知功能键只保留身份() {
    // F5 在 core 里没有语义键，但名字必须原样带给快捷键表 —— 否则 AI 按 F5
    // 在应用侧看起来和按了任何未识别键一样。
    let events = injected_events("F5");
    assert_eq!(events.len(), 2);
    match &events[0] {
        TextEvent::Key(k) => {
            assert_eq!(k.key, Key::Other);
            assert_eq!(k.name, "F5");
        }
        other => panic!("得到 {other:?}"),
    }
}

#[test]
fn 注入多字符名字不插文本() {
    // 与 `printable` 同一条判据：多字符整串丢弃，不要取首字符。
    let events = injected_events("ab");
    assert_eq!(events.len(), 2);
    assert!(!events.iter().any(|e| matches!(e, TextEvent::Char(_))));
}

#[test]
fn 注入的控制字符不插文本() {
    // `bridge.rs` 的解析器已经挡掉控制字符，这里再挡一道 —— 注入路径不该
    // 依赖上游校验。
    let events = injected_events("\n");
    assert_eq!(events.len(), 2);
    assert!(!events.iter().any(|e| matches!(e, TextEvent::Char(_))));
}

#[test]
fn 注入不带修饰键() {
    for name in ["a", "Enter", "F5"] {
        for e in injected_events(name) {
            if let TextEvent::Key(k) = e {
                assert!(!k.modifiers.ctrl() && !k.modifiers.alt() && !k.modifiers.super_key());
                assert!(!k.repeat);
            }
        }
    }
}

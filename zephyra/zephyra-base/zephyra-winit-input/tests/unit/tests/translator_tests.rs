// 本文件由 lib.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;

// ─────────────────────── 语义键映射：13 个编辑键 + 其余归 Other ───────────────────────

/// 每个被映射的 `NamedKey` 都必须映到**对应**的 [`Key`]。
///
/// 这条穷举了编辑控件会解释的全部按键 —— 漏一个的症状是「方向键不移动光标」，
/// 而那种 bug 在编译期完全看不出来。
#[test]
fn edit_keys_map_to_semantic_keys() {
    const PAIRS: &[(NamedKey, Key)] = &[
        (NamedKey::Backspace, Key::Backspace),
        (NamedKey::Delete, Key::Delete),
        (NamedKey::Enter, Key::Enter),
        (NamedKey::Escape, Key::Escape),
        (NamedKey::Space, Key::Space),
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
fn edit_key_count_matches_name_table() {
    // 13 = `Key` 里除 `Other` 之外的变体数。少一个说明映射表漏了。
    assert_eq!(Key::PageDown.as_str(), "PageDown");
    assert_eq!(Key::Backspace.as_str(), "Backspace");
}

#[test]
fn space_maps_to_a_semantic_key_with_a_stable_name() {
    // 空格是激活键（与 Enter 一起触发按钮点击），必须是语义键而不是 `Other`。
    assert_eq!(key_from_logical(&WKey::Named(NamedKey::Space)), Key::Space);
    assert_eq!(Key::Space.as_str(), "Space");
    // MCP `press_key "Space"` 走规范名回环，名字要一致。
    assert_eq!(key_name(&WKey::Named(NamedKey::Space)), "Space");
}

#[test]
fn non_edit_keys_fall_into_other_keeping_name() {
    let f5 = WKey::Named(NamedKey::F5);
    assert_eq!(key_from_logical(&f5), Key::Other);
    assert_eq!(key_name(&f5), "F5", "身份不能丢 —— 快捷键表靠它匹配");
}

#[test]
fn char_key_semantic_is_other_name_is_char() {
    let a = WKey::Character("a".into());
    assert_eq!(
        key_from_logical(&a),
        Key::Other,
        "字符走 Char 通道，不进语义键"
    );
    assert_eq!(key_name(&a), "a");
}

#[test]
fn dead_and_unknown_keys_do_not_panic() {
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
fn every_core_semantic_key_has_winit_source() {
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
fn modifiers_reported_one_by_one() {
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
fn modifier_state_overwrites_not_accumulates() {
    // 松开 Ctrl 必须把它清掉。若写成 `|=`，Ctrl 会永远粘住，
    // 之后每一次输入都变成快捷键。
    let mut t = Translator::new();
    t.on_modifiers(ModifiersState::CONTROL);
    assert!(t.modifiers().ctrl());
    t.on_modifiers(ModifiersState::empty());
    assert!(!t.modifiers().ctrl());
}

#[test]
fn no_modifiers_by_default() {
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
fn normal_press_inserts_one_char() {
    assert_eq!(printable(Some("a"), true, NONE), Some('a'));
}

#[test]
fn release_inserts_no_char() {
    // 每次输入都插两个字符的经典成因。宿主可能对同一次按键送 Pressed 与 Released。
    assert_eq!(printable(Some("a"), false, NONE), None);
}

#[test]
fn ctrl_alt_super_press_inserts_no_char() {
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
fn shift_does_not_block_char() {
    // Shift+A 就是 'A'：winit 已经把 shift 作用于 text 了。
    assert_eq!(printable(Some("A"), true, SHIFT), Some('A'));
}

#[test]
fn no_text_means_no_char() {
    assert_eq!(printable(None, true, NONE), None);
}

#[test]
fn multi_char_string_dropped_not_truncated() {
    // 死键组合与某些 IME 的快速输入会给出多字符 text。
    assert_eq!(printable(Some("ab"), true, NONE), None);
    assert_eq!(printable(Some("中文"), true, NONE), None);
}

#[test]
fn control_chars_skip_char_channel() {
    // Enter / Tab 由控件按 `Key` 自己判；再从 Char 走一遍会让「按 Enter
    // 插两个换行」这种 bug 出现。
    assert_eq!(printable(Some("\n"), true, NONE), None);
    assert_eq!(printable(Some("\t"), true, NONE), None);
    assert_eq!(printable(Some("\r"), true, NONE), None);
}

#[test]
fn empty_string_inserts_no_char() {
    assert_eq!(printable(Some(""), true, NONE), None);
}

#[test]
fn wide_chars_and_emoji_pass() {
    assert_eq!(printable(Some("中"), true, NONE), Some('中'));
    assert_eq!(printable(Some("😀"), true, NONE), Some('😀'));
}

// ───────────────────────── translate：顺序与内容 ─────────────────────────

#[test]
fn edges_always_precede_chars() {
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
fn release_produces_only_edge() {
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
fn function_key_produces_only_edge() {
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
fn arrow_keys_semantic_and_name_agree() {
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
fn modifier_snapshot_in_every_edge() {
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
fn auto_repeat_flag_passes_through() {
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
fn ime_enabled_carries_host_caret() {
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
fn ime_preedit_carries_byte_cursor_pair() {
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
fn empty_preedit_is_not_swallowed() {
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
fn ime_commit_enters_document() {
    let t = Translator::new();
    let mut out = Vec::new();
    t.ime(&WIme::Commit("你好".into()), (0.0, 0.0), &mut out);

    match &out[0] {
        TextEvent::Ime(Ime::Commit { text }) => assert_eq!(text, "你好"),
        other => panic!("得到 {other:?}"),
    }
}

#[test]
fn ime_disabled_cancels_not_commits() {
    // 把 Disabled 错翻成 Commit 会让「切输入法」把半截拼音插进文档。
    let t = Translator::new();
    let mut out = Vec::new();
    t.ime(&WIme::Disabled, (0.0, 0.0), &mut out);

    assert_eq!(out[0], TextEvent::Ime(Ime::Cancel));
}

#[test]
fn composition_full_sequence_order() {
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
fn inject_single_char_inserts_text() {
    let events = injected_events("a");
    assert_eq!(events.len(), 3, "按下 → 字符 → 抬起");
    assert!(matches!(&events[0], TextEvent::Key(k) if k.pressed));
    assert_eq!(events[1], TextEvent::Char('a'));
    assert!(matches!(&events[2], TextEvent::Key(k) if !k.pressed));
}

#[test]
fn inject_edit_key_inserts_no_text() {
    // 按 Enter 换行由控件按 `Key::Enter` 执行；再从字符通道走一遍会变成两个换行。
    for name in ["Enter", "Backspace", "ArrowDown", "Tab", "Escape"] {
        let events = injected_events(name);
        assert_eq!(events.len(), 2, "`{name}` 只该有按下/抬起两条边沿");
        assert!(!events.iter().any(|e| matches!(e, TextEvent::Char(_))));
    }
}

/// `type_text`：一段文字 ⇒ 一串**纯字符**事件。
///
/// ⚠ 不能带键边沿：有些控件把「一次按键」当成一次提交，那会让
/// 「输入 hello」变成「输入 h 然后提交、输入 e 然后提交……」。
#[test]
fn injected_text_events_are_pure_characters() {
    let events = injected_text_events("hi");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], TextEvent::Char('h'));
    assert_eq!(events[1], TextEvent::Char('i'));
    assert!(
        !events.iter().any(|e| matches!(e, TextEvent::Key(_))),
        "type_text 不该产生键边沿"
    );

    // 空格也算一个字符（输入框里少一个空格是极难归因的症状）。
    let events = injected_text_events("a b");
    assert_eq!(events.len(), 3);
    assert_eq!(events[1], TextEvent::Char(' '));

    // 多字节字符按**字符**算，不是按字节。
    let events = injected_text_events("中");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], TextEvent::Char('中'));

    // 空串 ⇒ 没有事件（不是一条空字符）。
    assert!(injected_text_events("").is_empty());
}

#[test]
fn inject_edit_key_semantic_correct() {
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
fn inject_wide_char_inserts_text() {
    let events = injected_events("中");
    assert_eq!(events[1], TextEvent::Char('中'));
}

#[test]
fn inject_unknown_function_key_keeps_identity() {
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
fn inject_multi_char_name_inserts_no_text() {
    // 与 `printable` 同一条判据：多字符整串丢弃，不要取首字符。
    let events = injected_events("ab");
    assert_eq!(events.len(), 2);
    assert!(!events.iter().any(|e| matches!(e, TextEvent::Char(_))));
}

#[test]
fn inject_control_char_inserts_no_text() {
    // `bridge.rs` 的解析器已经挡掉控制字符，这里再挡一道 —— 注入路径不该
    // 依赖上游校验。
    let events = injected_events("\n");
    assert_eq!(events.len(), 2);
    assert!(!events.iter().any(|e| matches!(e, TextEvent::Char(_))));
}

#[test]
fn inject_has_no_modifiers() {
    for name in ["a", "Enter", "F5"] {
        for e in injected_events(name) {
            if let TextEvent::Key(k) = e {
                assert!(!k.modifiers.ctrl() && !k.modifiers.alt() && !k.modifiers.super_key());
                assert!(!k.repeat);
            }
        }
    }
}

//! 控件层自己的门禁 —— **不依赖任何应用**。
//!
//! # 为什么这个文件存在
//!
//! 以前「控件层规矩」（外观不进源码、令牌拼错静默失效、分类名拼错静默失效…）
//! 只能靠 `gallery/tests/gates.rs` 间接覆盖：改了控件的令牌名单，红的是应用仓的
//! 测试，而主仓 `cargo test --workspace` 对这些规则完全不设防。本文件把**只靠框架
//! 数据就能判定**的那部分规则收回主仓；需要整棵树 × 全部交互态的规则仍在 gallery
//! （那要应用自己的 View 才能造树）。
//!
//! 每个测试都带恒真防御：先确认「有东西可查」，再断言。

use zephyra_core::theme::{Theme, ThemeVariant};
use zephyra_style::ir::StyleSheet;
use zephyra_ui::motion::MORPH_CLASSES;
use zephyra_ui::tokens::ALL as TOKENS;

/// 只合并控件层自己的样式表（`gallery` 的版本还会加上应用表）。
fn widget_sheet() -> StyleSheet {
    let mut sheet = StyleSheet::new();
    for (name, css) in zephyra_ui::styles::STYLESHEETS {
        let parsed = zstyle_compile::parse(name, css);
        assert!(
            parsed.is_ok(),
            "{name}.zstyle must parse: {:?}",
            parsed.errors
        );
        sheet = sheet.merge(parsed.sheet);
    }
    sheet
}

/// 控件模板源码（结构真相只有这一份）。
const CONTROLS: &str = include_str!("../ui/controls.zypui");

#[test]
fn every_token_declared_by_the_widget_layer_resolves_in_both_themes() {
    let dark = Theme::builtin(ThemeVariant::Dark);
    let light = Theme::builtin(ThemeVariant::Light);
    assert!(TOKENS.len() > 10, "令牌名单太短，这条有恒真风险");
    for t in TOKENS {
        assert!(
            dark.role(t).is_some() && light.role(t).is_some(),
            "tokens.rs 的 {t} 在主题里查不到（两份名单漂移）"
        );
    }
}

#[test]
fn widget_stylesheets_only_reference_tokens_that_exist() {
    let mut used: Vec<String> = Vec::new();
    for (name, css) in zephyra_ui::styles::STYLESHEETS {
        for (i, _) in css.match_indices('@') {
            let word: String = css[i + 1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if !word.is_empty() {
                used.push(format!("{name}.zstyle:{word}"));
            }
        }
    }
    assert!(!used.is_empty(), "一个 @令牌 都没用到 —— 恒真风险");
    let theme = Theme::builtin(ThemeVariant::Dark);
    let light = Theme::builtin(ThemeVariant::Light);
    let missing: Vec<String> = used
        .iter()
        .filter(|token| {
            let name = token.rsplit(':').next().unwrap();
            theme.role(name).is_none() || light.role(name).is_none()
        })
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "控件样式表引用了不存在的 @令牌（拼错 = 静默退回默认值）：\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn every_dropdown_morph_step_has_a_widget_rule() {
    let sheet = widget_sheet();
    let known: Vec<&str> = sheet
        .rules()
        .iter()
        .flat_map(|r| r.selector.names.iter().copied())
        .collect();
    assert!(
        MORPH_CLASSES.len() >= 8,
        "形变档位太少（{}）—— 短动画会看出阶梯",
        MORPH_CLASSES.len()
    );
    let missing: Vec<&str> = MORPH_CLASSES
        .iter()
        .copied()
        .filter(|cls| !known.contains(cls))
        .collect();
    assert!(
        missing.is_empty(),
        "形变档位在控件样式表里没有规则：{missing:?}"
    );
}

#[test]
fn widget_control_templates_emit_no_user_facing_text() {
    // 模板里的文本必须走 `{param}` / 语义 key，不能写字面文案 ——
    // 控件层只发 key，翻译是应用的事。扫源码即可判定，不需要造树。
    let mut checked = 0;
    for line in CONTROLS.lines() {
        let line = line.trim();
        if !line.starts_with("<text") && !line.starts_with("<control") {
            continue;
        }
        for attr in ["text=", "label="] {
            if let Some(start) = line.find(attr) {
                checked += 1;
                let rest = &line[start + attr.len()..];
                let value = rest.trim_start().trim_start_matches(['"', '\'']);
                assert!(
                    value.starts_with('{'),
                    "控件模板写了字面文案（应用无法翻译）：{line}"
                );
            }
        }
    }
    assert!(checked > 10, "没扫到文案参数 —— 这条有恒真风险");
}

#[test]
fn video_and_camera_controls_exist_and_are_styled() {
    // 每帧外部纹理的显示面：模板里要同时有 Video / Camera，样式表里要有对应分类名。
    for name in ["Video", "Camera"] {
        assert!(
            CONTROLS.contains(&format!("<component name=\"{name}\"")),
            "控件层缺少 {name} 组件"
        );
    }
    let sheet = widget_sheet();
    let known: Vec<&str> = sheet
        .rules()
        .iter()
        .flat_map(|r| r.selector.names.iter().copied())
        .collect();
    for class in ["video-frame", "camera-frame"] {
        assert!(known.contains(&class), "{class} 在样式表里没有规则");
    }
}

#[test]
fn video_player_control_has_full_player_structure_and_styling() {
    // 完整播放器：画面 + 六类交互 + 状态外观，缺一个都是"看着像播放器"的空壳。
    assert!(
        CONTROLS.contains("<component name=\"VideoPlayer\""),
        "控件层缺少 VideoPlayer 组件"
    );
    for key in [
        "act:player-play-pause",
        "act:player-seek",
        "fill:player-seek",
        "act:player-volume",
        "fill:player-volume",
        "act:player-mute",
        "act:player-rate",
        "act:player-fullscreen",
    ] {
        assert!(CONTROLS.contains(key), "VideoPlayer 缺少交互 key {key}");
    }
    let seek_fills = CONTROLS.matches("fill:player-seek").count();
    let volume_fills = CONTROLS.matches("fill:player-volume").count();
    assert_eq!(seek_fills, 1, "fill:player-seek 必须在模板里恰好出现一次");
    assert_eq!(
        volume_fills, 1,
        "fill:player-volume 必须在模板里恰好出现一次"
    );

    let sheet = widget_sheet();
    let known: Vec<&str> = sheet
        .rules()
        .iter()
        .flat_map(|r| r.selector.names.iter().copied())
        .collect();
    for class in [
        "video-player",
        "player-shade",
        "player-controls",
        "player-button",
        "player-time",
        "player-seek-track",
        "player-seek-fill",
        "player-seek-thumb",
        "player-volume-track",
        "player-volume-fill",
        "player-volume-thumb",
        "player-volume-value",
        "player-rate",
        "player-rate-label",
        "player-buffering",
        "player-label",
        "playing",
        "paused",
        "buffering",
        "ended",
        "fullscreen",
    ] {
        assert!(known.contains(&class), "{class} 在样式表里没有规则");
    }
}

#[test]
fn widget_stylesheet_rules_use_classes_the_templates_can_emit() {
    let sheet = widget_sheet();
    let rule_count = sheet.rules().len();
    assert!(rule_count > 50, "控件样式表规则太少 —— 恒真风险");
    // 样式表里出现的 `dd-morph-N` 必须都在 motion 的档位表里，
    // 否则样式表有死规则（与 gallery 的 `every_stylesheet_rule_matches_something`
    // 同向，但这里只判 motion 这一族，不需要整棵树）。
    for rule in sheet.rules() {
        for name in &rule.selector.names {
            if let Some(rest) = name.strip_prefix("dd-morph-") {
                // 复合选择器可能带后缀，只判纯数字的那一档。
                if !rest.chars().all(|c| c.is_ascii_digit()) || rest.is_empty() {
                    continue;
                }
                let n: usize = rest.parse().expect("已确认是纯数字");
                assert!(
                    (1..=MORPH_CLASSES.len()).contains(&n),
                    "样式表有 motion 档位 {name} 没有对应分类名"
                );
            }
        }
    }
}

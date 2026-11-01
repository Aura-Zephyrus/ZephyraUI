//! 复选框状态模型的门禁。
//!
//! 这些规则（流转、聚合、级联）是**画面上看不出来**的部分：模型算错了，
//! 界面仍然是「一个能点的复选框」，只是点的行为不对。所以它们只能靠单测钉住。

use super::{aggregate, CheckState, Checkbox, CheckboxGroup, CLASS_CHECKED, CLASS_INDETERMINATE};

// ─────────────────────────────── 流转 ───────────────────────────────

/// 两态循环：未选 → 选中 → 未选。
#[test]
fn two_state_cycle_is_checked_then_unchecked() {
    assert_eq!(CheckState::Unchecked.toggled(), CheckState::Checked);
    assert_eq!(CheckState::Checked.toggled(), CheckState::Unchecked);
}

/// 半选点一下是**全选**，不是清空。
///
/// 与 WinUI / HTML / macOS 一致：半选是「有选择但不全」的派生态，
/// 点它表达的是「我要全要」。若点它变成全不选，用户得点两次才能全选。
#[test]
fn indeterminate_toggles_to_checked() {
    assert_eq!(CheckState::Indeterminate.toggled(), CheckState::Checked);
}

/// 分类词：未选中是**空串**（不发分类名），另两个是各自的名字。
#[test]
fn class_words_match_the_stylesheet_vocabulary() {
    assert_eq!(CheckState::Unchecked.class_word(), "");
    assert_eq!(CheckState::Checked.class_word(), "checked");
    assert_eq!(CheckState::Indeterminate.class_word(), "indeterminate");
    assert_eq!(CLASS_CHECKED, "checked");
    assert_eq!(CLASS_INDETERMINATE, "indeterminate");
}

/// 分类词必须与 `controls-checkbox.zstyle` 里的选择器同名。
///
/// 拼错是**静默失效**：画面上只是没有那个态，没有任何报错。这条把
/// 「模型说的词」与「样式表认的词」钉在一起。
#[test]
fn class_words_exist_in_the_checkbox_stylesheet() {
    let sheet = crate::styles::stylesheet("checkbox").expect("复选框样式表必须在册");
    for word in [CLASS_CHECKED, CLASS_INDETERMINATE] {
        assert!(
            sheet.contains(&format!(".{word}")),
            "样式表里没有 .{word} 规则 —— 模型发出的分类词会静默失效"
        );
    }
}

// ─────────────────────────────── 聚合 ───────────────────────────────

/// 聚合规则：全不选 → 未选中；部分 → 半选；全选 → 选中。
#[test]
fn aggregate_follows_the_four_cases() {
    assert_eq!(aggregate(&[]), CheckState::Unchecked);
    assert_eq!(
        aggregate(&[CheckState::Unchecked, CheckState::Unchecked]),
        CheckState::Unchecked
    );
    assert_eq!(
        aggregate(&[CheckState::Checked, CheckState::Unchecked]),
        CheckState::Indeterminate
    );
    assert_eq!(
        aggregate(&[
            CheckState::Checked,
            CheckState::Unchecked,
            CheckState::Unchecked
        ]),
        CheckState::Indeterminate
    );
    assert_eq!(
        aggregate(&[CheckState::Checked, CheckState::Checked]),
        CheckState::Checked
    );
}

/// 子项里有半选时，组头也是半选（半选向上冒泡）。
#[test]
fn aggregate_propagates_indeterminate() {
    assert_eq!(
        aggregate(&[CheckState::Checked, CheckState::Indeterminate]),
        CheckState::Indeterminate
    );
}

// ─────────────────────────────── 单个复选框 ───────────────────────────────

#[test]
fn checkbox_toggles_between_two_states() {
    let mut cb = Checkbox::new("k", "ui.label");
    assert_eq!(cb.state(), CheckState::Unchecked);
    cb.toggle();
    assert_eq!(cb.state(), CheckState::Checked);
    cb.toggle();
    assert_eq!(cb.state(), CheckState::Unchecked);
}

/// 禁用的框点了没反应 —— 否则「禁用」只是画成灰的，点下去还会变。
#[test]
fn disabled_checkbox_does_not_toggle() {
    let mut cb = Checkbox::new("k", "ui.label").with_disabled(true);
    cb.toggle();
    assert_eq!(cb.state(), CheckState::Unchecked);
    assert!(cb.is_disabled());
}

// ─────────────────────────────── 组 ───────────────────────────────

fn group_of_three() -> CheckboxGroup {
    let mut g = CheckboxGroup::new("group");
    g.push("a", "ui.a");
    g.push("b", "ui.b");
    g.push("c", "ui.c");
    g
}

#[test]
fn group_header_is_derived_from_children() {
    let mut g = group_of_three();
    assert_eq!(g.header_state(), CheckState::Unchecked);

    g.toggle_item("a");
    assert_eq!(g.header_state(), CheckState::Indeterminate);

    g.toggle_item("b");
    g.toggle_item("c");
    assert_eq!(g.header_state(), CheckState::Checked);

    g.toggle_item("c");
    assert_eq!(g.header_state(), CheckState::Indeterminate);
}

/// 点子项**只改自己**，别的子项不动。
#[test]
fn toggling_an_item_leaves_the_others_alone() {
    let mut g = group_of_three();
    g.toggle_item("b");
    assert_eq!(g.item("a").unwrap().state(), CheckState::Unchecked);
    assert_eq!(g.item("b").unwrap().state(), CheckState::Checked);
    assert_eq!(g.item("c").unwrap().state(), CheckState::Unchecked);
}

/// 点组头：全不选 → 全选；半选 → 全选；全选 → 全不选。
#[test]
fn toggle_all_covers_all_deselect_all_and_the_middle_case() {
    let mut g = group_of_three();

    // 全不选 → 全选
    g.toggle_all();
    assert_eq!(g.header_state(), CheckState::Checked);
    assert!(g.items().iter().all(|i| i.state().is_checked()));

    // 全选 → 全不选
    g.toggle_all();
    assert_eq!(g.header_state(), CheckState::Unchecked);
    assert!(g.items().iter().all(|i| i.state() == CheckState::Unchecked));

    // 半选 → 全选（不是全不选）
    g.toggle_item("a");
    assert_eq!(g.header_state(), CheckState::Indeterminate);
    g.toggle_all();
    assert_eq!(g.header_state(), CheckState::Checked);
}

/// 空组：组头未选中，且「全选」无副作用。
#[test]
fn empty_group_is_unchecked_and_toggle_all_is_a_no_op() {
    let mut g = CheckboxGroup::new("group");
    assert!(g.is_empty());
    assert_eq!(g.header_state(), CheckState::Unchecked);
    g.toggle_all();
    assert_eq!(g.header_state(), CheckState::Unchecked);
    assert!(g.is_empty());
}

/// 不存在的 key 不该改坏任何东西。
#[test]
fn unknown_key_is_reported_not_ignored() {
    let mut g = group_of_three();
    assert!(!g.toggle_item("nope"));
    assert!(!g.set_item_state("nope", CheckState::Checked));
    assert_eq!(g.header_state(), CheckState::Unchecked);
}

/// 被禁用的子项：点它没反应，「全选」也不捎带改它。
#[test]
fn disabled_item_is_neither_clickable_nor_bulk_toggled() {
    let mut g = group_of_three();
    g.push("d", "ui.d").set_disabled(true);

    assert!(!g.toggle_item("d"));
    assert_eq!(g.item("d").unwrap().state(), CheckState::Unchecked);

    g.toggle_all();
    assert!(g.item("d").unwrap().state() == CheckState::Unchecked);
    assert!(g.item("a").unwrap().state().is_checked());
}

/// 禁用的子项**不参与组头聚合** —— 否则组头会卡死。
///
/// 3 个子项、1 个禁用且未选：若把它算进聚合，组头永远是半选，而点组头
/// 「全选」跳过它 → 什么都不变 → 组头永远半选、点它永远没反应。
#[test]
fn disabled_item_does_not_freeze_the_group_header() {
    let mut g = group_of_three();
    g.push("d", "ui.d").set_disabled(true);

    // 聚合只看两个可切换的子项 → 未选中（不是半选）。
    assert_eq!(g.header_state(), CheckState::Unchecked);

    // 点组头 → 两个可切换的都选中 → 组头到「全选」。
    g.toggle_all();
    assert_eq!(g.header_state(), CheckState::Checked);

    // 再点 → 都取消 → 组头回到未选中。
    g.toggle_all();
    assert_eq!(g.header_state(), CheckState::Unchecked);
}

/// 全组都禁用时，组头是未选中，「全选」无副作用。
#[test]
fn all_disabled_group_is_inert() {
    let mut g = CheckboxGroup::new("group");
    g.push("a", "ui.a").set_disabled(true);
    g.push("b", "ui.b").set_disabled(true);

    assert_eq!(g.header_state(), CheckState::Unchecked);
    g.toggle_all();
    assert_eq!(g.header_state(), CheckState::Unchecked);
    assert!(g.item("a").unwrap().state() == CheckState::Unchecked);
}

/// 组头的 `state` 字段改了也不影响派生状态 —— 组状态永远从子项算。
#[test]
fn header_state_is_never_stored() {
    let mut g = group_of_three();
    g.header_mut().set_state(CheckState::Checked);
    assert_eq!(g.header_state(), CheckState::Unchecked);
}

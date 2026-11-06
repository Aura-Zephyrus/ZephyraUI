//! 开关族状态模型的门禁。
//!
//! 这些规则（两态流转、单选互斥）是**画面上看不出来**的部分：模型算错了，
//! 界面仍然是「一个能点的开关」，只是点的行为不对。所以它们只能靠单测钉住。

use super::{RadioGroup, ToggleSwitch, CLASS_CHECKED, CLASS_ON, KNOB_DRAG_SLOP, KNOB_TRAVEL};

// ─────────────────────────────── 分类词 ───────────────────────────────

/// 开关：关是**空串**（不发分类名），开是 `on`。
#[test]
fn the_switch_class_words_match_the_stylesheet_vocabulary() {
    assert_eq!(CLASS_ON, "on");
    assert_eq!(CLASS_CHECKED, "checked");

    let off = glass("k", "ui.label");
    assert_eq!(off.class_word(), "");
    let on = off.with_checked(true);
    assert_eq!(on.class_word(), "on");
}

/// 分类词必须与 `controls-toggle.zstyle` 里的选择器同名。
///
/// 拼错是**静默失效**：画面上只是没有那个态，没有任何报错。这条把
/// 「模型说的词」与「样式表认的词」钉在一起。
#[test]
fn class_words_exist_in_the_toggle_stylesheet() {
    let sheet = crate::styles::stylesheet("toggle").expect("开关族样式表必须在册");
    for word in [CLASS_ON, CLASS_CHECKED] {
        assert!(
            sheet.contains(&format!(".{word}")),
            "样式表里没有 .{word} 规则 —— 模型发出的分类词会静默失效"
        );
    }
}

// ─────────────────────────────── 单个开关 ───────────────────────────────

#[test]
fn a_switch_toggles_between_two_states() {
    let mut sw = glass("k", "ui.label");
    assert!(!sw.is_checked());
    sw.toggle();
    assert!(sw.is_checked());
    sw.toggle();
    assert!(!sw.is_checked());
}

/// 禁用的开关点了没反应 —— 否则「禁用」只是画成灰的，点下去还会变。
#[test]
fn a_disabled_switch_does_not_toggle() {
    let mut sw = glass("k", "ui.label").with_disabled(true);
    sw.toggle();
    assert!(!sw.is_checked());
    assert!(sw.is_disabled());
}

// ─────────────────────────────── 按压动画 ───────────────────────────────

/// 按下目标 → 进度朝 1 推进；松开 → 朝 0 回落；收敛后**钉死**在端点。
///
/// # 为什么钉死重要
///
/// 不钉的话进度永远停在 0.999 —— 属性 diff 每帧都变，稳态零重建被破坏
/// （画面看不出来，但整棵树每帧重排）。
#[test]
fn press_progress_advances_toward_the_target_and_lands_on_it() {
    let mut sw = glass("k", "ui.label");
    assert_eq!(sw.press_progress(), 0.0);

    sw.set_pressed(true);
    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert_eq!(
        sw.press_progress(),
        1.0,
        "按住 600 帧后必须收敛到 1（并钉死）"
    );

    sw.set_pressed(false);
    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert_eq!(sw.press_progress(), 0.0, "松开后必须回落到 0（并钉死）");
}

/// 动画有**中间帧** —— 不是瞬时跳变：按下即推进进度。
///
/// 原版 `pressProgressAnimationSpec = spring(1f, 1000f)`：按下即开始动画，
/// 没有延迟。
#[test]
fn press_progress_is_animated_not_instant() {
    let mut sw = glass("k", "ui.label");
    sw.set_pressed(true);
    // 按下即开始动画：第一帧后进度已在 (0,1)
    sw.advance(16.0);
    let p = sw.press_progress();
    assert!(
        p > 0.0 && p < 1.0,
        "按下即推进，进度 {p} 应在 (0,1) —— 快弹簧不是瞬跳"
    );
}

/// 禁用的开关不进动画 —— 进度恒 0。
#[test]
fn a_disabled_switch_never_animates() {
    let mut sw = glass("k", "ui.label").with_disabled(true);
    sw.set_pressed(true);
    sw.advance(16.0);
    assert_eq!(sw.press_progress(), 0.0);
}

// ─────────────────────────────── 把手动画 ───────────────────────────────

/// `with_checked(true)` 必须把把手**直接**钉在开端 —— 启动时不该看到它滑过来。
#[test]
fn with_checked_places_the_knob_immediately() {
    let sw = glass("k", "ui.label").with_checked(true);
    assert_eq!(sw.fraction(), 1.0);
    assert_eq!(sw.knob_x(), crate::KNOB_TRAVEL);
}

/// 翻转后 `fraction` 向 1 推进，收敛后**钉死**在端点。
#[test]
fn fraction_advances_toward_the_target_and_lands_on_it() {
    let mut sw = glass("k", "ui.label");
    sw.toggle();
    assert_eq!(sw.fraction(), 0.0, "toggle 本身不瞬移把手");

    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert_eq!(sw.fraction(), 1.0, "开后必须收敛到 1（并钉死）");
    assert_eq!(sw.knob_x(), crate::KNOB_TRAVEL);

    sw.toggle();
    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert_eq!(sw.fraction(), 0.0, "关后必须回落到 0（并钉死）");
    assert_eq!(sw.knob_x(), 0.0);
}

/// 把手位移 = fraction × 行程（不是别的换算）。
#[test]
fn knob_x_tracks_fraction_linearly() {
    let mut sw = glass("k", "ui.label");
    sw.toggle();
    sw.advance(16.0);
    let p = sw.fraction();
    assert!((sw.knob_x() - p * crate::KNOB_TRAVEL).abs() < 1e-6);
}

/// 按压时把手 X/Y 缩放分别趋向 1.5×（松开回 1），且与 fraction 是独立的轴。
#[test]
fn knob_scale_follows_press_progress() {
    let mut sw = glass("k", "ui.label");
    assert_eq!(sw.knob_scale_x(), 1.0);
    assert_eq!(sw.knob_scale_y(), 1.0);

    sw.set_pressed(true);
    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert_eq!(sw.knob_scale_x(), crate::KNOB_PRESSED_SCALE);
    assert_eq!(sw.knob_scale_y(), crate::KNOB_PRESSED_SCALE);

    // 按压与位置正交：翻转状态不影响缩放。
    sw.toggle();
    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert_eq!(sw.knob_scale_x(), crate::KNOB_PRESSED_SCALE);
    assert_eq!(sw.fraction(), 1.0);
}

/// 缩放弹簧现按**临界阻尼**（ζ=1，√250）积分：按压过程单调趋近 `PRESSED`。
///
/// # 为什么还留一条
///
/// 欠阻尼的「到位末端先挤扁再撑宽」正是用户验收退回的原因；这条钉住
/// **不允许过冲**：`knob_scale_x/y` 在按压全程不得超过 `1.5 + ε`。
/// 不是线性 lerp 校验——指数逼近也满足这个谓词；它要抓的是有人把
/// `spring_step` 改回欠阻尼或误删钉死规则。
#[test]
fn knob_scale_never_overshoots_while_pressing() {
    let mut sw = glass("k", "ui.label");
    sw.set_pressed(true);
    let mut max_x = 1.0f32;
    let mut max_y = 1.0f32;
    for _ in 0..60 {
        sw.advance(16.0);
        max_x = max_x.max(sw.knob_scale_x());
        max_y = max_y.max(sw.knob_scale_y());
    }
    assert!(
        max_x <= crate::KNOB_PRESSED_SCALE + 1e-3,
        "X 缩放峰值 {max_x} 越过 1.5 —— 临界阻尼不该过冲"
    );
    assert!(
        max_y <= crate::KNOB_PRESSED_SCALE + 1e-3,
        "Y 缩放峰值 {max_y} 越过 1.5 —— 临界阻尼不该过冲"
    );
    assert!(
        max_x >= crate::KNOB_PRESSED_SCALE - 0.01,
        "X 缩放峰值 {max_x} 应逼近 1.5（press 弹簧还在跑）"
    );
    assert!(
        max_y >= crate::KNOB_PRESSED_SCALE - 0.01,
        "Y 缩放峰值 {max_y} 应逼近 1.5（press 弹簧还在跑）"
    );
}

/// 程序/单击切换（非手势）时，把手**不**被速度弹簧拉伸。
///
/// 单击后 fraction 由临界弹簧自行逼近目标；本仓已**整体删除**原版 `layerBlock`
/// 的速度拉伸项（单击时 velocity 只在弹簧自主推进期非零，正是「莫名其妙变长」
/// 的根源；拖拽期 fraction 直接由 Move 写入、velocity 恒等于 0）。
/// 这条钉住两个轴在任何帧都相等，直到两轴弹簧自行收敛到 1.0。
#[test]
fn programmatic_checked_flip_does_not_stretch_the_knob() {
    let mut sw = glass("k", "ui.label");
    sw.toggle(); // fraction 0 → 1（自主临界弹簧）
    for i in 0..600 {
        sw.advance(16.0);
        assert!(
            (sw.knob_scale_x() - sw.knob_scale_y()).abs() < 1e-6,
            "第 {i} 帧自主位移不应产生速度拉伸（scale.x - scale.y = {}）",
            sw.knob_scale_x() - sw.knob_scale_y()
        );
    }
    assert_eq!(sw.knob_scale_x(), 1.0);
    assert_eq!(sw.knob_scale_y(), 1.0);
}

/// `KNOB_TRAVEL` 必须与 `controls-toggle.zstyle` 的度量对齐。
///
/// 这里直接断言来源数值（track 64 / padding 2 / knob 40）——
/// 改样式表忘改常量时这条红。
#[test]
fn knob_travel_matches_the_stylesheet_geometry() {
    assert_eq!(crate::KNOB_TRAVEL, 64.0 - 2.0 * 2.0 - 40.0);
}

/// `is_animating()` 的三态：静止→false、动作中→true、收敛后→精确 false。
///
/// # 为什么这条是宿主的帧泵门禁
///
/// D3D12/Vulkan 停留在保留式节奏（无事件不画帧）。视图回答不出「还要帧」时，
/// 动画只在事件到来的那一拍推进一格，随后事件循环 `Wait` 睡死 —— 把手是
/// 「跳」过去的。判据必须是**精确**比较：`advance` 收敛时把所有量钉死在端点，
/// 所以动画结束后这里必然为 false，宿主才能回到零帧静止（否则空转烧 GPU）。
#[test]
fn is_animating_tracks_the_springs() {
    let mut sw = glass("k", "ui.label");
    assert!(!sw.is_animating(), "新造的开关不该声称在动画");

    sw.toggle();
    assert!(sw.is_animating(), "状态翻转后弹簧还没收敛，必须继续泵帧");

    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert!(
        !sw.is_animating(),
        "收敛后必须精确为 false（宿主回到零帧静止）"
    );

    sw.set_pressed(true);
    // 原版无延迟：按下即有缩放动画
    assert!(sw.is_animating(), "按下即有缩放弹簧要跑");
    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert!(!sw.is_animating(), "按住稳定后同样要停");
}

/// 拖拽手势：Down → Move(右拖 20px) → Up 使 fraction 从 0 → 1 并翻转 checked。
///
/// # 为什么这条测试必须用 `process_pointer_events` 而不是直接设 fraction
///
/// `process_pointer_events` 是**唯一**能触发拖拽逻辑的入口：
/// - `Down` + `pressed=true` 进入拖拽模式
/// - `Move` 直接设 fraction（不走弹簧）
/// - `Up` 退出拖拽、翻转 checked
///
/// 若测试只设 fraction 会绕过拖拽状态机，漏掉 `dragging` 标记的切换。
#[test]
fn drag_toggles_checked_via_pointer_events() {
    use zephyra_fx::{PointerInput, PointerInputPhase};

    let mut sw = glass("k", "ui.label");
    assert_eq!(sw.fraction(), 0.0);
    assert!(!sw.is_checked());

    // 模拟：按下（pressed=true）+ 向右拖满一个行程 + 松开
    sw.set_pressed(true);
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    assert!(sw.dragging, "Down+pressed 应进入拖拽模式");

    // 指针要走的距离 = 行程 + 阈值：阈值那一段被吞掉（不进入拖动量），
    // 与 Compose 的 `overSlop` 同一条约定，见 `KNOB_DRAG_SLOP`。
    let cursor_x = 100.0 + KNOB_TRAVEL + KNOB_DRAG_SLOP;
    sw.process_pointer_events(
        &[PointerInput {
            x: cursor_x,
            y: 200.0,
            phase: PointerInputPhase::Move,
        }],
        1.0,
    );
    assert!(
        (sw.fraction() - 1.0).abs() < 0.001,
        "拖满行程 fraction 应为 1"
    );

    // 松开：拖拽结束，目标 checked 由宿主取走并应用
    sw.process_pointer_events(
        &[PointerInput {
            x: cursor_x,
            y: 200.0,
            phase: PointerInputPhase::Up,
        }],
        1.0,
    );
    assert!(!sw.dragging, "Up 应退出拖拽模式");
    // 模拟宿主 click 处理：取走拖拽完成的目标状态并应用
    if let Some(target) = sw.take_drag_completed_target_checked() {
        sw.set_checked(target);
    }
    assert!(sw.is_checked(), "拖过中点松开应翻转为开");

    // 再拖回去：按下 + 向左拖满一个行程 + 松开
    sw.set_pressed(true);
    sw.process_pointer_events(
        &[PointerInput {
            x: cursor_x,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Move,
        }],
        1.0,
    );
    assert!(
        (sw.fraction() - 0.0).abs() < 0.001,
        "拖回起点 fraction 应为 0"
    );
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Up,
        }],
        1.0,
    );
    // 模拟宿主 click 处理
    if let Some(target) = sw.take_drag_completed_target_checked() {
        sw.set_checked(target);
    }
    assert!(!sw.is_checked(), "拖回左端松开应翻转为关");
}

/// 不在 pressed 状态下的 Down 不应开始拖拽（点击别处再拖动不应误触发）。
#[test]
fn drag_does_not_start_without_pressed() {
    use zephyra_fx::{PointerInput, PointerInputPhase};

    let mut sw = glass("k", "ui.label");
    // pressed=false 时 Down
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    assert!(!sw.dragging, "未按下时 Down 不应进入拖拽");
    // 后续 Move 也不应改变 fraction
    sw.process_pointer_events(
        &[PointerInput {
            x: 120.0,
            y: 200.0,
            phase: PointerInputPhase::Move,
        }],
        1.0,
    );
    assert_eq!(sw.fraction(), 0.0, "未拖拽时 fraction 不应变");
}

/// 点一下（Down → Up，中间没有 Move）**不能**把宿主已经翻转的 `checked` 改回去。
///
/// # 这条钉的是什么
///
/// 宿主在释放那一帧先走 `click`（把 `checked` 翻成 true），再调
/// `process_pointer_events`。旧实现在 Up 里无条件写
/// `checked = fraction > 0.5` —— 此时 `fraction` 还停在 0（弹簧还没动），
/// 于是刚翻过去的 true 立刻被写回 false。症状是**按下去有玻璃、松手什么都没变**，
/// 而模型单测与 `--dump-cpu` 都是绿的（它们各自只看一半）。
#[test]
fn a_tap_does_not_undo_the_host_toggle() {
    use zephyra_fx::{PointerInput, PointerInputPhase};

    let mut sw = glass("k", "ui.label");
    sw.set_pressed(true);
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    // 宿主在释放那一帧的激活路径（鼠标 / 键盘 / UIA 都走这里）。
    sw.toggle();
    assert!(sw.is_checked(), "前提：宿主已经把状态翻开了");

    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Up,
        }],
        1.0,
    );
    assert!(!sw.dragging, "Up 仍要退出拖拽模式");
    assert!(
        sw.is_checked(),
        "没拖过就松开时，模型不该覆盖宿主已经翻转的状态"
    );
}

/// 拖出去再拖回原点仍算「拖过」：松手按 `fraction` 提交，而不是当成点击。
///
/// 判据是**逐事件**的位移（原版 `didDrag = dragAmount.x != 0f`），不是累计位移
/// —— 累计会在这里判成 0，于是松手什么都不发生，把手停在轨迹中间。
#[test]
fn a_round_trip_drag_still_commits_by_threshold() {
    use zephyra_fx::{PointerInput, PointerInputPhase};

    let mut sw = glass("k", "ui.label");
    sw.set_pressed(true);
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    // 先拖到右端，再拖回起点：累计位移为 0，但确实拖过。
    for x in [110.0, 120.0, 110.0, 100.0] {
        sw.process_pointer_events(
            &[PointerInput {
                x,
                y: 200.0,
                phase: PointerInputPhase::Move,
            }],
            1.0,
        );
    }
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Up,
        }],
        1.0,
    );
    assert!(
        !sw.is_checked(),
        "拖回左端松手应提交为关（且不能靠宿主再翻一次）"
    );
}

// ─────────────────────────────── 单选组 ───────────────────────────────

fn group_of_three() -> RadioGroup {
    let mut g = RadioGroup::new();
    g.push("a", "ui.a");
    g.push("b", "ui.b");
    g.push("c", "ui.c");
    g
}

/// 选中一项 ⇒ **其余全部取消**。
#[test]
fn selecting_one_cancels_the_others() {
    let mut g = group_of_three();
    assert!(g.select("a"));
    assert_eq!(g.selected_key(), Some("a"));
    assert!(g.is_selected("a"));
    assert!(!g.is_selected("b"));
    assert!(!g.is_selected("c"));

    assert!(g.select("c"));
    assert_eq!(g.selected_key(), Some("c"));
    assert!(!g.is_selected("a"), "上一项必须被取消");
    assert!(g.is_selected("c"));
}

/// 点已选中的那一项 ⇒ **保持选中**（不是取消选中）。
///
/// 与 WinUI / HTML / macOS 一致。单选钮是「挑一个」，不是「勾选」。
#[test]
fn selecting_the_selected_one_is_a_no_op() {
    let mut g = group_of_three();
    assert!(g.select("b"));
    assert!(!g.select("b"), "已经是选中项了 —— 状态没变");
    assert_eq!(g.selected_key(), Some("b"));
    assert!(g.is_selected("b"), "点已选中的单选钮，它保持选中");
}

/// 不存在的 key 不该改坏任何东西。
#[test]
fn an_unknown_key_is_reported_not_ignored() {
    let mut g = group_of_three();
    assert!(!g.select("nope"));
    assert_eq!(g.selected_key(), None);
}

/// 禁用的选项**选不上**，但仍留在组里。
#[test]
fn a_disabled_option_cannot_be_selected() {
    let mut g = group_of_three();
    g.push("d", "ui.d").set_disabled(true);

    assert!(!g.select("d"));
    assert_eq!(g.selected_key(), None);
    assert_eq!(g.len(), 4, "禁用项仍留在组里（画成灰的）");
}

/// 选中别的项之后，被禁用的那项**不会**被顺带取消成「没选中过」——
/// 它本来就没被选中过，而选中态是**只有一项**的事，不存在「取消」。
#[test]
fn selecting_does_not_disturb_a_disabled_option() {
    let mut g = group_of_three();
    g.push("d", "ui.d").set_disabled(true);

    g.select("a");
    assert_eq!(g.selected_key(), Some("a"));
    assert!(!g.is_selected("d"));
}

/// 空组：没有任何项被选中，「选中」不存在的 key 无副作用。
#[test]
fn an_empty_group_has_no_selection() {
    let mut g = RadioGroup::new();
    assert!(g.is_empty());
    assert_eq!(g.selected_key(), None);
    assert!(!g.select("nope"));
    assert_eq!(g.selected_key(), None);
}

/// 组的分类词口径：只有被选中的那一项发 `checked`。
#[test]
fn only_the_selected_option_gets_the_checked_class() {
    let mut g = group_of_three();
    g.select("b");
    assert_eq!(g.option_class_word("a"), "");
    assert_eq!(g.option_class_word("b"), "checked");
    assert_eq!(g.option_class_word("c"), "");
}

// ─────────────────────────────── 1:1 拖拽语义 ───────────────────────────────

/// 拖过端点再拖回来：越界的那部分不会"攒起来"。
///
/// 原版 `fraction = fraction + dragAmount.x / dragWidth` 每一步都 clamp；
/// 本仓曾用绝对位置映射，导致越过右端再往回拖时，fraction 会停在 1 而不是
/// 从 1 退回去。
#[test]
fn drag_overshoot_is_not_banked_when_dragging_back() {
    use zephyra_fx::{PointerInput, PointerInputPhase};

    let mut sw = glass("k", "ui.label");
    sw.set_checked(true); // fraction = 1
    sw.set_pressed(true);
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    // 向右拖 30px（越过端点）：f 仍是 1
    sw.process_pointer_events(
        &[PointerInput {
            x: 130.0,
            y: 200.0,
            phase: PointerInputPhase::Move,
        }],
        1.0,
    );
    assert_eq!(sw.fraction(), 1.0);
    // 再往左拖 10px：应从 1 退回 0.5，而不是继续保持 1
    sw.process_pointer_events(
        &[PointerInput {
            x: 120.0,
            y: 200.0,
            phase: PointerInputPhase::Move,
        }],
        1.0,
    );
    assert!(
        (sw.fraction() - 0.5).abs() < 1e-6,
        "拖回 10px 应让 fraction 从 1 退到 0.5，实际 {}",
        sw.fraction()
    );
}

/// 外部切换（键盘 / UIA / 宿主 set_checked）同样走原版 `animateToValue` 的
/// press 保持期：状态变化时，按压弹簧不会立刻回落。
#[test]
fn programmatic_toggle_holds_press_until_fraction_settles() {
    let mut sw = glass("k", "ui.label");
    sw.toggle();
    sw.advance(16.0);
    assert!(sw.is_animating(), "外部切换也要泵帧");
    assert!(
        sw.press_progress() > 0.0,
        "原版外部 selected 变化会触发 animateToValue → press()，进度不应瞬时为 0"
    );
    // 跑完后必须收敛
    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert!(!sw.is_animating());
    assert_eq!(sw.press_progress(), 0.0);
}

/// 点一下（无拖拽）时，release 保持期同样等 fraction 到位后才回落。
#[test]
fn tap_holds_press_until_knob_reaches_the_target() {
    let mut sw = glass("k", "ui.label");
    sw.set_pressed(true);
    for _ in 0..120 {
        sw.advance(16.0);
    }
    let p_before_release = sw.press_progress();
    // 宿主：松手 + click 激活漏斗（toggle）
    sw.set_pressed(false);
    sw.toggle();
    sw.advance(16.0);
    assert!(
        sw.press_progress() >= p_before_release,
        "松手后 press 不会立刻掉下来（原版 release() 先等 value 收敛）：before {p_before_release}, after {}",
        sw.press_progress()
    );
    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert_eq!(sw.press_progress(), 0.0, "把手到位后 press 必须回落到 0");
}

/// set_checked(同一个值) 不触发按压保持期。
#[test]
fn set_checked_with_the_same_value_is_a_no_op_for_animation() {
    let mut sw = glass("k", "ui.label");
    sw.set_checked(false);
    sw.advance(16.0);
    assert_eq!(sw.press_progress(), 0.0);
    assert!(!sw.is_animating());
}

/// 点击期间指针抖 1–3 像素**不能**把这次点击吞掉。
///
/// # 这条钉的是什么
///
/// 原版判据是 `didDrag = dragAmount.x != 0f`。它只在触屏上等价于「这是不是
/// 一次拖动」——桌面鼠标每次点按都带几像素抖动，于是点按一律走拖拽提交，
/// 而拖拽提交在净位移不过半时**什么都不做**。症状：点开关不切换（把手被
/// 抖了一下又弹回原位）。实测：gallery 里 455→457 的 2 像素抖动点击后，
/// `switch:demo` 停在原状态。
///
/// 判据改成「净位移越过 [`KNOB_DRAG_SLOP`]」，且阈值以内把手一动不动。
#[test]
fn a_jittery_click_still_toggles() {
    use crate::toggle::KNOB_DRAG_SLOP;
    use zephyra_fx::{PointerInput, PointerInputPhase};

    for jitter in [1.0_f32, 2.0, KNOB_DRAG_SLOP - 0.5] {
        let mut sw = glass("k", "ui.label");
        sw.set_pressed(true);
        sw.process_pointer_events(
            &[PointerInput {
                x: 100.0,
                y: 200.0,
                phase: PointerInputPhase::Down,
            }],
            1.0,
        );
        sw.process_pointer_events(
            &[PointerInput {
                x: 100.0 + jitter,
                y: 200.0,
                phase: PointerInputPhase::Move,
            }],
            1.0,
        );
        assert_eq!(
            sw.fraction(),
            0.0,
            "阈值内的抖动不该挪动把手（jitter={jitter}）"
        );
        sw.process_pointer_events(
            &[PointerInput {
                x: 100.0 + jitter,
                y: 200.0,
                phase: PointerInputPhase::Up,
            }],
            1.0,
        );
        assert!(
            sw.take_drag_completed_target_checked().is_none(),
            "阈值内的抖动不算拖拽，松手必须交给宿主的激活路径（jitter={jitter}）"
        );
        // 宿主的激活漏斗
        sw.toggle();
        assert!(
            sw.is_checked(),
            "抖动点击之后必须真的翻转（jitter={jitter}）"
        );
    }
}

/// 越过阈值之后，把手跟手：越过那一步只应用超出阈值的部分。
#[test]
fn a_drag_past_the_slop_follows_the_pointer_one_to_one() {
    use crate::toggle::{KNOB_DRAG_SLOP, KNOB_TRAVEL};
    use zephyra_fx::{PointerInput, PointerInputPhase};

    let mut sw = glass("k", "ui.label");
    sw.set_pressed(true);
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    // 净位移 = slop + 半步行程 ⇒ fraction 恰好 0.5（阈值那段被吞掉）。
    let travel = KNOB_DRAG_SLOP + KNOB_TRAVEL * 0.5;
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0 + travel,
            y: 200.0,
            phase: PointerInputPhase::Move,
        }],
        1.0,
    );
    assert!(
        (sw.fraction() - 0.5).abs() < 1e-6,
        "越过阈值后应只补上超出部分，实际 {}",
        sw.fraction()
    );
    // 之后一步一步跟手，不再扣阈值。
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0 + travel + KNOB_TRAVEL * 0.25,
            y: 200.0,
            phase: PointerInputPhase::Move,
        }],
        1.0,
    );
    assert!(
        (sw.fraction() - 0.75).abs() < 1e-6,
        "实际 {}",
        sw.fraction()
    );
}

/// 拖拽中途停手超过一帧的「大步长」保护**不能**把把手拉回端点。
///
/// # 这条钉的是什么
///
/// 宿主只在 `is_animating()` 时说「还要帧」。按住不动时按压/缩放弹簧很快收敛，
/// 拖拽中的 `fraction` 又被有意排除在 `is_animating` 之外 —— 于是宿主睡死，
/// 下一次移动那一帧的 `dt_ms` 是真实的一两秒。`advance` 的「一步到位」分支
/// 曾无条件把 `fraction` 写回 `checked` 的端点：把手瞬移回起点，后续增量从
/// 起点重新累加。症状：拖到另一半、停一下、再拖 —— 被拉回原侧，松手也不翻转。
#[test]
fn a_large_dt_does_not_snap_a_live_drag_back_to_the_origin() {
    use zephyra_fx::{PointerInput, PointerInputPhase};

    let mut sw = glass("k", "ui.label");
    sw.set_pressed(true);
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    sw.process_pointer_events(
        &[PointerInput {
            x: 130.0,
            y: 200.0,
            phase: PointerInputPhase::Move,
        }],
        1.0,
    );
    assert!(sw.fraction() > 0.5, "前提：已经拖过中点");
    let held = sw.fraction();

    // 停手 3 秒（宿主没有泵帧）之后的第一帧。
    sw.advance(3000.0);
    assert!(
        (sw.fraction() - held).abs() < 1e-6,
        "拖拽中 fraction 是用户的手，大步长保护不能动它：{held} → {}",
        sw.fraction()
    );

    // 抬手仍然按阈值提交。
    sw.process_pointer_events(
        &[PointerInput {
            x: 130.0,
            y: 200.0,
            phase: PointerInputPhase::Up,
        }],
        1.0,
    );
    assert_eq!(
        sw.take_drag_completed_target_checked(),
        Some(true),
        "拖过中点松手应提交为开"
    );
}

/// 上一段手势没被取走的提交目标，不能在下一段手势里当结论用。
#[test]
fn a_stale_drag_target_is_dropped_when_the_next_press_starts() {
    use zephyra_fx::{PointerInput, PointerInputPhase};

    let mut sw = glass("k", "ui.label");
    sw.set_pressed(true);
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    sw.process_pointer_events(
        &[PointerInput {
            x: 130.0,
            y: 200.0,
            phase: PointerInputPhase::Move,
        }],
        1.0,
    );
    sw.process_pointer_events(
        &[PointerInput {
            x: 130.0,
            y: 200.0,
            phase: PointerInputPhase::Up,
        }],
        1.0,
    );
    // 宿主这一帧没跑激活漏斗（例如控件在松手前就被禁用 / 失焦），目标留了下来。
    assert_eq!(sw.take_drag_completed_target_checked(), Some(true));

    // 造一个残留：直接再走一遍，然后不取走，紧接着开始下一次按下。
    sw.set_pressed(true);
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    sw.process_pointer_events(
        &[PointerInput {
            x: 130.0,
            y: 200.0,
            phase: PointerInputPhase::Move,
        }],
        1.0,
    );
    sw.process_pointer_events(
        &[PointerInput {
            x: 130.0,
            y: 200.0,
            phase: PointerInputPhase::Up,
        }],
        1.0,
    );
    // 这一次按下（新的一段手势）必须把上一段留下的目标丢掉。
    sw.set_pressed(true);
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    assert_eq!(
        sw.take_drag_completed_target_checked(),
        None,
        "新的一次按下必须清掉上一段手势留下的提交目标"
    );
}

// ───────────────────── 长帧间隔下的弹簧稳定性 ─────────────────────

/// 拖拽期间宿主**会睡死**：`is_animating` 故意不把拖拽中的 `fraction` 算作
/// 「还没收敛」（否则按住不动 = 每秒 60 帧空转）。于是下一次输入带来的真实
/// dt 常在 130–500 ms 之间 —— 而半隐式欧拉的稳定上限是 `2/ω`，最快的一根
/// 弹簧（√300）只有 ≈116 ms。一步跨过去不是收敛而是**发散**：实测释放那一帧
/// dt=381.6 ms，`scale_x` 从 1.5 跳到 −16.7，把手被翻转放大到整块空白
/// （看起来就是「按住左右滑动之后动画卡住」）。
///
/// 这条钉住 `spring_step` 的子步积分：任意 dt 下缩放都必须留在
/// `[0.5, 1.6]` 的合理邻域内，且有限。
#[test]
fn a_long_frame_gap_never_makes_the_scale_spring_explode() {
    for gap_ms in [130.0_f32, 200.0, 300.0, 381.6, 480.0] {
        let mut sw = glass("k", "ui.label");
        sw.set_pressed(true);
        for _ in 0..120 {
            sw.advance(16.0);
        }
        assert!(
            (sw.knob_scale_x() - 1.5).abs() < 1e-4,
            "前提：按压弹簧已收敛"
        );

        sw.set_pressed(false);
        sw.advance(gap_ms);

        for (axis, s) in [("x", sw.knob_scale_x()), ("y", sw.knob_scale_y())] {
            assert!(
                s.is_finite(),
                "dt={gap_ms}ms 后 scale_{axis} 变成非有限值 {s}"
            );
            assert!(
                (0.5..=1.6).contains(&s),
                "dt={gap_ms}ms 后 scale_{axis}={s} 越界 —— 弹簧发散了（把手的缩放会翻转放大）"
            );
        }

        // 继续泵帧必须收敛回 1.0，而不是永远在震荡。
        for _ in 0..600 {
            sw.advance(16.0);
        }
        assert!(!sw.is_animating(), "dt={gap_ms}ms 之后动画必须能收敛");
        assert_eq!(sw.knob_scale_x(), 1.0);
        assert_eq!(sw.knob_scale_y(), 1.0);
    }
}

/// 用户实测路径：按住 → 在轨道**中间**左右来回滑 → 松手。
///
/// 每一步都隔着一次「宿主睡死后的长帧」（300 ms 量级）。要求：
/// 1. 拖拽期间 `fraction` 每步都跟着手走（不能停住）；
/// 2. 缩放全程有限且不越界；
/// 3. 松手那一帧（dt 可能刚好落在 130–500 ms 的危险窗口）不再爆掉。
#[test]
fn sliding_back_and_forth_in_the_middle_keeps_the_knob_alive() {
    use zephyra_fx::{PointerInput, PointerInputPhase};

    fn mv(sw: &mut ToggleSwitch, x: f32) {
        sw.process_pointer_events(
            &[PointerInput {
                x,
                y: 200.0,
                phase: PointerInputPhase::Move,
            }],
            1.0,
        );
    }

    let mut sw = glass("k", "ui.label");
    sw.set_pressed(true);
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    // 按压弹簧收敛后宿主就睡了 —— 后续帧的 dt 都是真实的一两百毫秒。
    for _ in 0..120 {
        sw.advance(16.0);
    }

    // 越过 4 DIP 的 slop，把手进到中段（fraction ≈ 0.3）。
    sw.advance(300.0);
    mv(&mut sw, 110.0);
    let mut prev = sw.fraction();
    assert!(prev > 0.1 && prev < 0.9, "前提：把手在中段，实际 {prev}");

    // 中间小幅左右滑动（±3 px，不碰端点），每次隔一次长帧。
    for (i, x) in [107.0_f32, 113.0, 107.0, 113.0, 107.0, 113.0]
        .into_iter()
        .enumerate()
    {
        sw.advance(300.0);
        mv(&mut sw, x);
        let now = sw.fraction();
        assert!(
            (now - prev).abs() > 1e-4,
            "第 {i} 步 fraction 没动（{prev} → {now}）—— 中间滑动时把手停住了"
        );
        assert!(
            now > 0.05 && now < 0.95,
            "第 {i} 步 fraction={now} 跑出中段，前提被破坏"
        );
        prev = now;
        for (axis, s) in [("x", sw.knob_scale_x()), ("y", sw.knob_scale_y())] {
            assert!(s.is_finite(), "第 {i} 步 scale_{axis} 非有限：{s}");
            assert!(
                (0.5..=1.6).contains(&s),
                "第 {i} 步 scale_{axis}={s} 越界 —— 弹簧发散"
            );
        }
    }

    // 松手那一帧带着长 dt 进来（正是之前爆掉的那一帧）。
    sw.set_pressed(false);
    sw.process_pointer_events(
        &[PointerInput {
            x: 113.0,
            y: 200.0,
            phase: PointerInputPhase::Up,
        }],
        1.0,
    );
    sw.advance(381.6);
    for (axis, s) in [("x", sw.knob_scale_x()), ("y", sw.knob_scale_y())] {
        assert!(s.is_finite(), "松手帧 scale_{axis} 非有限：{s}");
        assert!(
            (0.5..=1.6).contains(&s),
            "松手帧 scale_{axis}={s} 越界 —— 弹簧发散（把手会翻转放大）"
        );
    }

    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert!(!sw.is_animating(), "松手后动画必须收敛");
    assert_eq!(sw.knob_scale_x(), 1.0);
    assert_eq!(sw.knob_scale_y(), 1.0);
}

/// 动画相关的测试默认走液态玻璃（Solid 是默认材质，零动画）。
fn glass(key: &'static str, label: &'static str) -> ToggleSwitch {
    ToggleSwitch::new(key, label).with_material(zephyra_core::filter::Material::Glass)
}

// ───────────────────── 快入 tween（Direct） ─────────────────────

/// `cubic-bezier(0,0,0,1)` 的退化形状（x = t³，y = 3t² − 2t³），以 t = 直接 elapsed
/// 比例即可求 y —— 但本仓没有多项式求值，公式直接 `3·u^(2/3) − 2·u`。
/// 端点必须精确（diff「不抖」的根基），且单调、先快后慢。
#[test]
fn direct_ease_shape_is_monotonic_and_pinned_to_endpoints() {
    let samples: Vec<f32> = (0..=100)
        .map(|i| {
            let t = i as f32 / 100.0;
            3.0 * t.powf(2.0 / 3.0) - 2.0 * t
        })
        .collect();
    assert_eq!(samples[0], 0.0, "0% 必须精确为 0");
    assert_eq!(samples[100], 1.0, "100% 必须精确为 1");
    assert!(
        samples.windows(2).all(|w| w[1] >= w[0]),
        "快入曲线必须单调不减"
    );
    assert!(
        samples[50] > 0.8 && samples[50] < 1.0,
        "u=0.5 时应明显过半程（≈0.89，先快后慢），实际 {}",
        samples[50]
    );
    assert!(
        samples[25] > 0.6 && samples[25] < 0.9,
        "u=0.25 时应明显过半程（先快后慢），实际 {}",
        samples[25]
    );
}

/// 完整走完 250 ms 之后 fraction 必须**精确**落在目标上、动画熄火。
#[test]
fn fraction_tween_lands_exactly_on_the_target_and_stops() {
    let mut sw = glass("k", "ui.label");
    sw.toggle(); // fraction 0 → 1
    for _ in 0..20 {
        sw.advance(16.0);
    }
    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert_eq!(sw.fraction(), 1.0);
    assert!(!sw.is_animating(), "收敛后宿主必须能睡死");
}

/// 中途反转：toggle → 推进 ~133 ms（过半程）→ toggle back。
/// 位移必须从**当前**位置重启满 250 ms，不回弹、不瞬移。
#[test]
fn reversing_mid_tween_restarts_from_current_fraction() {
    let mut sw = glass("k", "ui.label");
    sw.toggle(); // 0 → 1
                 // 推进 ~128 ms（8 × 16 ms），fraction 应已经明显向 1 走。
    for _ in 0..8 {
        sw.advance(16.0);
    }
    let mid = sw.fraction();
    assert!(mid > 0.7 && mid < 0.99, "前提：tween 过半程，实际 {mid}");

    sw.toggle(); // 1 → 0
    let before_reverse = sw.fraction();
    assert_eq!(before_reverse, mid, "toggle 自身不搬动 fraction");

    sw.advance(16.0);
    let after_one = sw.fraction();
    assert!(
        after_one < before_reverse,
        "反转后第 1 帧 fraction 必须沿反方向走（{before_reverse} → {after_one}）"
    );
    assert!(after_one > 0.0, "反转后不允许瞬移回 0");

    // 完整走完 → 精确落 0。
    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert_eq!(sw.fraction(), 0.0);
    assert!(!sw.is_animating());
}

/// tween 的进行中判据不能被 dragging 掩盖：松手后
/// fraction 未到端点时动画必须被视为“进行中”（宿主才肯泵帧）。
#[test]
fn an_unfinished_tween_is_animating() {
    let mut sw = glass("k", "ui.label");
    sw.toggle();
    sw.advance(16.0);
    assert!(sw.is_animating(), "tween 刚起步就必须判定为动画中");
    for _ in 0..600 {
        sw.advance(16.0);
    }
    assert!(!sw.is_animating());
}

// ───────────────────── Solid 材质（默认）零动画 ─────────────────────

/// 默认（未指定材质）= Solid：toggle 后 fraction 必须**立刻**精确落位，
/// 没有任何动画进行中。
#[test]
fn solid_toggle_snaps_instantly_and_never_animates() {
    let mut sw = ToggleSwitch::new("k", "ui.label");
    assert_eq!(sw.material(), zephyra_core::filter::Material::Solid);
    sw.toggle();
    assert_eq!(sw.fraction(), 1.0, "Solid：fraction 必须立刻到位");
    assert!(!sw.is_animating(), "Solid：不存在动画中状态");
    assert_eq!(sw.knob_x(), 20.0);
    sw.advance(16.0);
    assert_eq!(sw.fraction(), 1.0);
    sw.toggle();
    assert_eq!(sw.fraction(), 0.0);
    assert!(!sw.is_animating());
}

/// Solid 下按压也瞬变：`set_pressed(true)` → press 立刻 = 1.0（无弹簧爬升）。
#[test]
fn solid_press_progress_is_instant() {
    let mut sw = ToggleSwitch::new("k", "ui.label");
    sw.set_pressed(true);
    sw.advance(16.0);
    assert_eq!(sw.press_progress(), 1.0);
    sw.set_pressed(false);
    sw.advance(16.0);
    assert_eq!(sw.press_progress(), 0.0);
    assert!(!sw.is_animating());
}

/// Solid 拖拽期间 fraction 仍由手势直接写，`advance` 不把它拉回端点
/// （手势不是动画；与玻璃模式同一保护）。
#[test]
fn solid_drag_fraction_is_not_snapped_back() {
    use zephyra_fx::{PointerInput, PointerInputPhase};

    let mut sw = ToggleSwitch::new("k", "ui.label");
    sw.set_pressed(true);
    sw.process_pointer_events(
        &[PointerInput {
            x: 100.0,
            y: 200.0,
            phase: PointerInputPhase::Down,
        }],
        1.0,
    );
    // 越过 slop 后的拖拽增量直接落到 fraction 上。
    sw.process_pointer_events(
        &[PointerInput {
            x: 112.0,
            y: 200.0,
            phase: PointerInputPhase::Move,
        }],
        1.0,
    );
    let mid = sw.fraction();
    assert!(
        mid > 0.0 && mid < 1.0,
        "拖拽后 fraction 应处于中间值，实际 {mid}"
    );
    sw.advance(16.0);
    assert_eq!(
        sw.fraction(),
        mid,
        "Solid：advance 不允许回拉拖拽中的 fraction"
    );
}

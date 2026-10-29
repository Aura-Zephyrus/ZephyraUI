//! 令牌名单的门禁。
//!
//! `ALL` 是门禁遍历的对象（`gallery/tests/gates.rs` 逐个对 `Theme::role()`），
//! 所以**漏一个名字 = 那个令牌永远不会被检查**，而失败模式是静默的：样式表里
//! 写了 `@surface_alt_translucent` 时颜色悄悄保持原值，看起来像「样式没生效」。
//!
//! 2026-10-28 实测踩过：`SURFACE_ALT_TRANSLUCENT` 常量定义了、`ALL` 里没有。
//! 下面 `core 的每个角色都出现在名单里` 那条就是钉住它的门禁。

use zephyra_core::theme::{Theme, ThemeVariant};

use super::ALL;

/// 深浅两套主题都要能查到：令牌是主题无关的语义名，只在一套主题里成立
/// 说明那份实现漏了分支。
#[test]
fn every_token_in_all_resolves_in_both_themes() {
    let dark = Theme::builtin(ThemeVariant::Dark);
    let light = Theme::builtin(ThemeVariant::Light);
    for t in ALL {
        assert!(
            dark.role(t).is_some(),
            "令牌 `{t}` 在深色主题里查不到 —— 它在样式表里会静默退回原值"
        );
        assert!(
            light.role(t).is_some(),
            "令牌 `{t}` 在浅色主题里查不到 —— 它在样式表里会静默退回原值"
        );
    }
}

#[test]
fn all_has_no_duplicates() {
    for (i, a) in ALL.iter().enumerate() {
        for b in &ALL[i + 1..] {
            assert_ne!(a, b, "令牌 `{a}` 在 ALL 里出现了两次");
        }
    }
}

/// core 的角色名单 → 本文件的名单，这个方向**没有自动门禁**（Rust 没有常量
/// 反射），所以手列在这里：core 新增角色时这里必须同改。
///
/// 手列的代价换来的是判据方向正确：判的是「core 有、名单没有」，而那正是
/// 2026-10-28 漏掉 `surface_alt_translucent` 的形状。
#[test]
fn every_core_role_appears_in_list() {
    use zephyra_core::theme::roles;

    const CORE_ROLES: &[&str] = &[
        roles::SURFACE,
        roles::SURFACE_ALT,
        roles::SURFACE_ALT_TRANSLUCENT,
        roles::CONTENT,
        roles::CONTENT_SECONDARY,
        roles::CONTENT_DISABLED,
        roles::STROKE,
        roles::CONTROL_FILL,
        roles::CONTROL_FILL_HOVER,
        roles::CONTROL_FILL_PRESSED,
        roles::CONTROL_SECONDARY,
        roles::CONTROL_TERTIARY,
        roles::CONTROL_STRONG,
        roles::CONTROL_BORDER,
        roles::CONTROL_BORDER_STRONG,
        roles::DIVIDER,
        roles::ACCENT_FILL,
        roles::ON_ACCENT_FILL,
        roles::FOCUS_RING,
        roles::FOCUS_INNER,
        roles::CARD,
        roles::CARD_SECONDARY,
        roles::CARD_BORDER,
        roles::LAYER,
        roles::SUCCESS,
        roles::CAUTION,
        roles::CRITICAL,
        roles::SUCCESS_BG,
        roles::CAUTION_BG,
        roles::CRITICAL_BG,
    ];

    let mut missing: Vec<&str> = CORE_ROLES
        .iter()
        .copied()
        .filter(|r| !ALL.contains(r))
        .collect();
    missing.sort_unstable();
    assert!(
        missing.is_empty(),
        "这些 core 角色不在 tokens::ALL 里，门禁会静默跳过它们：\n  {}",
        missing.join("\n  ")
    );
}

/// 镜像常量的**值**必须与 core 的角色名一致。
///
/// 值漂移在编译期就不可能（常量就是 core 常量的别名），这条测的是别名关系
/// 本身没被改写成字面量 —— 一旦有人写死 `"surface"`，core 改名时就不再报错。
#[test]
fn mirror_constants_still_point_to_core_roles() {
    use zephyra_core::theme::roles;

    const PAIRS: &[(&str, &str)] = &[
        (super::SURFACE, roles::SURFACE),
        (super::SURFACE_ALT, roles::SURFACE_ALT),
        (
            super::SURFACE_ALT_TRANSLUCENT,
            roles::SURFACE_ALT_TRANSLUCENT,
        ),
        (super::CONTENT, roles::CONTENT),
        (super::CONTENT_SECONDARY, roles::CONTENT_SECONDARY),
        (super::CONTENT_DISABLED, roles::CONTENT_DISABLED),
        (super::STROKE, roles::STROKE),
        (super::ACCENT_FILL, roles::ACCENT_FILL),
        (super::ON_ACCENT_FILL, roles::ON_ACCENT_FILL),
        (super::FOCUS_RING, roles::FOCUS_RING),
        (super::FOCUS_INNER, roles::FOCUS_INNER),
        (super::LAYER, roles::LAYER),
        (super::CRITICAL_BG, roles::CRITICAL_BG),
    ];

    for (mine, theirs) in PAIRS {
        assert_eq!(mine, theirs, "镜像常量与 core 的角色名不一致");
        assert!(
            ALL.contains(mine),
            "`{mine}` 定义了却不在 ALL 里 —— 门禁会静默跳过它"
        );
    }
}

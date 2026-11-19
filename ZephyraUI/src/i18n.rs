//! 控件层的 i18n 契约：**控件自有语义**的文案与「哪些参数是语义 key」。
//!
//! # 为什么要控件层自己有一份
//!
//! 「控件只发 key，文案表是应用的」这条纪律一直写在 `Cargo.toml` 的注释里，
//! 但**并没有兑现** —— 控件模板里的 `<text text="{label}">` 收的是应用**已经
//! 翻译好**的文案，控件层从头到尾没见过 key。
//!
//! 本模块把它兑现：控件模板的文案参数收**语义 key**，翻译发生在**控件内部**
//! （`tr()` → 宿主的文案表）。于是：
//!
//! - 换应用时只换 key 空间，控件模板一个字都不用改；
//! - 控件层**永远不发面向用户的文案** ⇒ 「控件层零裸文案」可以做成门禁；
//! - 控件**自身语义**的文案（复选框组的组头「全选」）归控件层，应用可覆盖。
//!
//! # 两层 key 命名空间
//!
//! | 层 | 命名空间 | 谁的文案 |
//! |---|---|---|
//! | 控件层 | `zephyra.*` | 控件**自身语义**（组头「全选」）—— [`builtin_messages`] 自带默认译文 |
//! | 应用层 | `ui.*` 等 | 应用自己的文案 —— 应用自己的表 |
//!
//! 两层用不同前缀 ⇒ 合并时不会撞名，撞了也能一眼看出是哪层的问题。
//!
//! # 应用怎么用
//!
//! ```ignore
//! // 应用建表时把控件层的默认文案并进来（应用条目优先 ⇒ 应用可覆盖控件的默认说法）。
//! let mut msgs = app_messages();
//! zephyra_ui::i18n::merge_builtin(&mut msgs);
//! ```
//!
//! 合并方向是**单向**的：控件层永远不覆盖应用已经写好的条目 —— 否则应用改一句
//! 说法就被控件的默认值吃掉了，而那正是「应用可覆盖」这条承诺的反面。

use zephyra_core::i18n::{Messages, Preferences};

/// 控件层 key 的命名空间前缀。
///
/// 门禁靠它区分「这是控件层的 key」与「这是应用的 key」——
/// 两者混在一起时，报错要能一眼看出是哪一层的问题。
pub const NAMESPACE: &str = "zephyra";

/// 复选框组**组头**的语义 key。
///
/// # 为什么组头的文案归控件层
///
/// 「全选」是**控件语义**而不是应用语义：组头是一颗三态复选框，点了它就是
/// 「全选 / 全部不选」—— 这个动作的含义由控件定义，不由应用定义。
/// 应用能改的是**名词**（「全选邮件」「全选文件」），而「全选」这个词本身
/// 属于控件。
///
/// 放在控件层还有个好处：**换应用时这一行不用改**，而它恰恰是最容易被
/// 漏翻的一行（组头不像子项那样每个应用都写一遍）。
pub const CHECKBOX_SELECT_ALL: &str = "zephyra.checkbox.select-all";

/// 控件层自有的全部 key（门禁与文档用）。
pub const BUILTIN_KEYS: &[&str] = &[CHECKBOX_SELECT_ALL];

/// 控件的**语义 key 参数**契约表。
///
/// # 这张表是门禁的真相来源
///
/// 值是「组件名 → 哪个实参收语义 key」。门禁靠它判断：
///
/// - 应用模板里 `<Button label_key="ui.ok"/>` 里的 `ui.ok` 必须在文案表里
///   （否则画面上会显示键名本身 —— 可见的降级，不是静默失败）；
/// - 反过来，`<Button label="确定"/>` 这种**裸文案**会被判为「静态文案没走
///   i18n 通道」⇒ 红。
///
/// # 为什么用 `label_key` 而不是让 `label` 直接收 key
///
/// `label` 还要接**动态文案**（「删除 3 项」这种每帧不同的文本）。若让 `label`
/// 独占 key 语义，动态文案就没地方放了 —— 那是真实的功能回退。
///
/// ⇒ 双通道：`label_key` 收语义 key（控件内部 `tr()`），`label` 收已翻译的
/// 文案。两者互斥，模板里 `label_key ? tr(label_key) : label` 择一。
pub const KEY_PARAMS: &[(&str, &str)] = &[
    ("Button", "label_key"),
    ("PillButton", "label_key"),
    ("ToggleButton", "label_key"),
    ("RoundButton", "label_key"),
    ("CheckBox", "label_key"),
    ("CheckBoxGroup", "label_key"),
    ("RadioButton", "label_key"),
    ("ToggleSwitch", "label_key"),
];

/// 这个实参是不是「语义 key」型（按 [`KEY_PARAMS`] 判定）。
pub fn is_key_param(component: &str, param: &str) -> bool {
    KEY_PARAMS
        .iter()
        .any(|(c, p)| *c == component && *p == param)
}

/// 控件层自带的默认文案（三语言）。
///
/// # 为什么自带而不是让应用全给
///
/// 控件的**自身语义**（组头「全选」）在每个应用里都是同一句话。让每个应用
/// 都抄一遍，抄漏的那个就会显示键名 —— 而这行恰恰是最容易漏的。
///
/// 自带默认值 ⇒ 应用**什么都不做**也是对的；应用想改说法时覆盖即可
/// （见 [`merge_builtin`]）。
pub fn builtin_messages() -> Messages {
    let mut m = Messages::new();
    for (locale, key, value) in BUILTIN_TEXT {
        m.insert(locale, key, value);
    }
    m
}

/// 控件层自带的文案（`(locale, key, value)` 三元组）。
///
/// 顺序按 locale 分组（与 gallery 的 `CATALOG` 同一习惯，便于肉眼比对）。
const BUILTIN_TEXT: &[(&str, &str, &str)] = &[
    // ---- en（默认语言）----
    ("en", CHECKBOX_SELECT_ALL, "Select all"),
    // ---- zh-Hans ----
    ("zh-Hans", CHECKBOX_SELECT_ALL, "全选"),
    // ---- ja ----
    ("ja", CHECKBOX_SELECT_ALL, "すべて選択"),
];

/// 把控件层的默认文案并进应用的表（**应用条目优先**）。
///
/// # 方向为什么是单向的
///
/// 控件层永远**不覆盖**应用已有的条目。反过来的话，应用改一句说法就被控件
/// 的默认值吃掉了 —— 那正好是「应用可覆盖控件默认说法」这条承诺的反面，
/// 而且症状是「改了没反应」，极难归因。
///
/// # 覆盖后仍然是三语言齐全的
///
/// 合并是**逐 (locale, key)** 判的：应用只覆盖**它自己写过的那门语言**时，
/// 另两门仍走控件层的默认 —— 不会出现「某个 key 只有一门语言有译文」而门禁
/// 报缺译（那说明应用确实该补，而不是控件该补）。
pub fn merge_builtin(app: &mut Messages) {
    for (locale, key, value) in BUILTIN_TEXT {
        if app.get(locale, key).is_none() {
            app.insert(locale, key, value);
        }
    }
}

/// 按偏好取一条文案；查不到回落**键名本身**（可见的降级）。
///
/// 与宿主的 `tr()` 同一套回退链（[`Messages::select`]）：先按 `prefs` 逐个
/// 语言试（精确 locale → language-only），再试 `fallback` 指定的默认语言，
/// 最后回落键名。
///
/// ⚠ `key` 是 `&'static str`：回落值要能存进 `&'static str`（控件层的
/// 文案与 key 全是编译期常量 —— 本仓的零分配纪律不允许运行期拼字符串）。
pub fn resolve(
    msgs: &Messages,
    prefs: &Preferences,
    fallback: &str,
    key: &'static str,
) -> &'static str {
    msgs.select(prefs, fallback, key).unwrap_or(key)
}

#[cfg(test)]
#[path = "../tests/unit/i18n.rs"]
mod tests;

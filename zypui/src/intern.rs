//! 动态字符串的 `&'static` 化去重。
//!
//! # 为什么需要它
//!
//! `Element` 的 key / 分类名只收 `&'static str`（元素树每帧重建，运行期字符串
//! 等于每帧泄漏）。表达式求值会产出动态串（`"button " + variant` 这类），
//! 这里把**同一内容**只泄漏一份并跨帧复用 —— 每帧成本是一次哈希查表。
//!
//! # 增长上界
//!
//! 表只随「出现过的不同串」增长。分类名/文案这类词的词汇表是有限集
//! （variant × 状态词的组合数），启动若干帧后不再新增。若某绑定把**无限集**
//! （时间戳、原始输入）灌进分类名/key，那就是应用的 bug —— 内存会线性涨，
//! 而这类 bug 在这里以最直白的方式现形。

use std::collections::HashMap;

/// 字符串去重表。一个 `.zypui` 工件一份，与它的生命周期一致。
#[derive(Default)]
pub struct Interner {
    map: HashMap<Box<str>, &'static str>,
}

impl Interner {
    /// 空表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 取 `s` 的唯一 `&'static` 副本：已有就复用，没有才泄漏。
    pub fn intern(&mut self, s: &str) -> &'static str {
        if let Some(hit) = self.map.get(s) {
            return hit;
        }
        let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
        self.map.insert(Box::from(leaked), leaked);
        leaked
    }

    /// 已见过的不同串数（测试与诊断用）。
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// 表是否为空。
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

//! 文本层：测量、排版、字形图集 —— 实现已搬到 `zephyra-vector`，此处按原路径再导出。
//!
//! # 分层
//!
//! 文本拆成两半，切分线是**字体文件从哪来**：
//!
//! ```text
//! core（零依赖，本层）                base（宿主）
//! ───────────────────                ─────────────────
//! GlyphSource  trait（唯一要宿主实现的）  该 trait 的实现
//! TextStack    排版 + 断行 + 图集 + 降级   └ ab_glyph 解析字体字节
//! GlyphAtlas   字形位图打包（shelf）     D3D11 纹理上传
//! ```
//!
//! core 侧**不**知道字体格式、不知道图集是 GPU 纹理、不知道纹理句柄长什么样。
//! 它只知道"给我一个能把字符变成位图的来源"。于是换一个字体栈（ab_glyph →
//! cosmic-text → CoreText）只需实现 [`GlyphSource`]，本文件一行不改。
//!
//! 宿主那一侧因此只剩两件事：把字体字节解析成 [`GlyphSource`]，以及把
//! [`GlyphAtlas::pixels`] 上传成纹理。
//!
//! # 为什么图集在 core 而不是 base
//!
//! 字形图集常被误认为"GPU 资源所以该在 base"。但它**不是** —— 它是一块 CPU 侧的
//! `Vec<u8>` 加一张"哪个字形在哪个位置"的表；GPU 只在最后一步 upload 一次。
//!
//! 把它放 base 的代价是：打包逻辑（shelf packer、边界检查、uv 计算）只能在 base
//! 里测试，而 base 的测试需要 GPU 或字体文件。放 core 则这些都能在纯 CPU 上断言 ——
//! 本文件末尾的断言正是这么写的。
//!
//! # 为什么 `Command::Text` 在 core 里产出
//!
//! [`crate::paint::Command::Text`] 要求 `Vec<GlyphQuad>`（带 uv）与图集句柄。
//! 宿主只能**消费** `DisplayList`，不能往里注入命令 —— 所以这个转换必须在 core
//! 降级时发生。core 有图集，于是能给出 uv；core 不碰纹理句柄，于是只用那个不透明
//! 的 [`crate::paint::TextureRef`]。
//!
//! # 已知限制
//!
//! **没有 GSUB/GPOS 整形。** 本层不做连字、双向文本与复杂脚本 —— 那些需要完整的
//! OpenType 整形栈。阿拉伯语、印地语会显示成逐字孤立字形。要支持就得换一个更强的
//! [`GlyphSource`]（且 [`GlyphSource::advance`] 需要返回整段而非单字形，以承载
//! 字素簇）。
//!
//! 行内断行按**字符边界**切，不做连字符与字素簇感知的断词。对 UI 标签够用。

pub use zephyra_vector::text::*;

/// `String` → `PropertyValue::Str` 的便捷转换。
///
/// 文本几乎总是运行期构造的（来自应用状态），所以这个转换比手写
/// `PropertyValue::Str(s.into())` 常见得多。
pub fn text_value(s: impl Into<std::borrow::Cow<'static, str>>) -> crate::property::PropertyValue {
    crate::property::PropertyValue::Str(s.into().into())
}

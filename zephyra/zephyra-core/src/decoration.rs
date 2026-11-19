//! 玻璃装饰层：高光、外投影、内阴影 —— **纯描述，零依赖，不碰任何系统 API**。
//!
//! # 它和 [`crate::filter`] 是两件事
//!
//! | | 描述什么 | 画在内容的哪一侧 |
//! |---|---|---|
//! | [`crate::filter`] | 这条命令**自己**要多经过什么处理（模糊 / 折射 / 投影） | 取决于滤镜（`redraw_original` 决定原图在合成前还是后） |
//! | 本模块 | 一块**玻璃**的装饰四件套：镜面高光、外投影、内阴影 | 固定：外投影在**内容之下**，高光与内阴影在**内容之上** |
//!
//! 层序是**契约**而不是实现细节，所以它必须写进数据里（一个 `FilterChain`
//! 表达不了"这层在内容之上"）。Android 原版（`AndroidLiquidGlass`，Apache-2.0）
//! 的实际绘制顺序就是这个：
//!
//! ```text
//! 外投影（最下）→ 内容 → 高光 → 内阴影（最上）
//! ```
//!
//! # 一比一复刻的参数
//!
//! 本模块里的常量与构造函数**逐字**取自 Kotlin 原版：
//!
//! | 装饰 | 原版构造 | 参数 |
//! |---|---|---|
//! | [`Highlight`] | `Highlight.Default` | `width=0.5dp`, `blurRadius=width/2`, `alpha=1`, `color=White@0.5`, `blend=Plus`, `angle=45°`, `falloff=1` |
//! | [`Highlight`] | `Highlight.Ambient` | `intensity=0.38` ⇒ `color=White@0.38`, `blend=SrcOver`, 无角度 / 衰减 |
//! | [`Highlight`] | `Highlight.Plain` | `color=White@0.38`, `blend=Plus`, **无着色器** |
//! | [`OuterShadow`] | `Shadow(radius=24dp, offset=4dp)` | `radius=24dp`, `offset=(0, radius/6)`, `color=Black@0.1` |
//! | [`InnerShadow`] | `InnerShadow(radius=24dp)` | `radius=24dp`, `offset=(0, radius)`, `color=Black@0.15` |
//!
//! Dp→px 按 `density = 1` 直接换算 —— 于是 `0.5dp` 就是 `0.5px`，与原版数值
//! 逐字节相同（不引入缩放因子）。
//!
//! # 为什么不把它们编成 [`crate::filter::Filter`] 的变体
//!
//! 三条装饰里只有外投影的语义与 [`crate::filter::Filter::DropShadow`] 接近，
//! 但**不完全相同**：原版的外投影要"挖掉形状内侧的余量"（玻璃是半透明的，
//! 内侧的影子会透出来，看起来像脏污）。把它硬塞进 `DropShadow` 会让那个滤镜
//! 的语义分叉，而现有调用方（普通卡片的投影）不该受影响。高光与内阴影更不
//! 是"滤镜"——它们靠形状遮罩与描边，不是对整张图的卷积。

pub use zephyra_vector::decoration::*;

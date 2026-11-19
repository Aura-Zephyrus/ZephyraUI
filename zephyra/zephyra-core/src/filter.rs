//! 效果契约：滤镜（filter）与混合模式（blend）——**纯描述，零依赖，不碰任何系统 API**。
//!
//! # 为什么效果在这里，而不在GPU 层
//!
//! Skia 把效果切在 [`include/effects`](https://skia.googlesource.com/skia/+/refs/heads/main/include/effects/)
//! ——与 `include/core` **并列**的独立层。构建依赖是单向的：`include/effects` 依赖
//! `//src/core`，反向零命中。所以它的切法是：
//!
//! > 效果的**描述**是纯数据（只用 `SkScalar` / `SkPath` 这类 core 词汇表），
//! > 效果的**执行**永远在 GPU 侧（`src/gpu/ganesh/ops/`）。
//!
//! 这条边界正是 [`crate::paint::DisplayList`] 契约成立的前提：效果一旦降级成
//! DisplayList 里的纯字段，换一个 base 实现（DXGI ↔ DirectComposition，或
//! Windows ↔ Wayland）产出的 DisplayList 必然逐字节相同 —— 因为效果从没
//! 参与过"画什么"的决策，它只是"画的东西要多经过一道什么处理"的声明。
//!
//! # 为什么**不**用 `PushLayer` / `PopLayer` 配对命令
//!
//! 这是本模块最容易被写错的地方，而DisplayList 的结构直接否掉了那条路：
//!
//! `DisplayList` 构造时即按 `(layer, z, order)` **稳定排序**（见
//! [`crate::paint`]）。配对命令靠"在有序流里前后夹住内容"生效，可排序会把
//! 中间内容插到别的 `layer` 上去 —— `PushLayer(Content)` 包住的命令可能被排到
//! `Popup` 之后，配对当场失效，且**没有任何报错**。
//!
//! Skia 也没有这个问题，因为它压根不用配对命令：滤镜挂在 `SkPaint` 上，离屏由
//! `SkCanvas::aboutToDraw` → `AutoLayerForImageFilter`（`SkCanvasPriv.h:129`）
//! 在**回放时按需自动开** —— 有 imageFilter 才 `addImageFilterLayer`
//! (`SkCanvasPriv.cpp:234`)。命令流里没有 save/restore 标记。
//!
//! 所以这里沿用同一思路：[`crate::paint::Command`] 各变体**自己**带
//! `filter`与 `blend` 字段，回放方看到非恒等的 filter 才去开离屏。
//!
//! # 离屏边界由谁算
//!
//! 由**滤镜自己**给出，而不是录进命令流。Skia 走
//! `paint.computeFastBounds(drawBounds)`（`SkCanvasPriv.cpp:308`），本质是滤镜
//! 贡献一份 outset。本模块用 [`Filter::outsets`] 做同一件事。
//!
//! ⚠ 那条 outset 与四边形 SDF 的抗锯齿余量（`zephyra_fx::quad::AA_BAND = 0.75`）
//! 是**两套独立依据**，不要混为一谈：
//!
//! | | 谁贡献 | 值 | 挡什么 |
//! |---|---|---|---|
//! | `AA_BAND` | 着色器 SDF | 固定 0.75px | 边缘一个像素的抗锯齿过渡带 |
//! | [`Filter::outsets`] | 滤镜半径 | `3σ`（blur）/ `offset + 3σ`（shadow） | 模糊/阴影溢出原矩形的那部分 |

pub use zephyra_vector::filter::*;

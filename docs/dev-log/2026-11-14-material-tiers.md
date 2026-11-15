# 2026-11-14 材质三档整合：纯色 / 磨砂 / 液态玻璃（真实折射）

> 现象：`Material` 曾有四档（`Solid` / `Frosted` / `GlassRefract` / `PixelOffset`），
> 其中 `GlassRefract` 与 `PixelOffset` 都画「玻璃样子」但原理不同（前者真折射、
> 后者形状位移），且 `zstylec` 里 `material: glass-refract` 与 `material: glass` 指向
> 同一件事的两个拼法。磨砂态用 `@surface_alt_translucent`（α=0.85） ⇒ 视觉上与
> 纯色几乎不可分，用户指着屏幕说「磨砂和纯色长一样」。需要整合成**恰好三档**：
> 纯色（无滤镜）/ 磨砂（`Filter::BackdropBlur`）/ 液态玻璃（`Filter::Refraction`，
> 真实斯涅尔折射）。

## 排查

1. **`Material` 枚举精简到三个变体**：`Solid` / `Frosted` / `Glass`。
   `Material::GlassRefract` 重命名为 `Material::Glass`，保留
   `Material::glass_refract()` 作为 `#[deprecated(since = "2026.11.0")]` 兼容别名。
   `GLASS_OFFSET_AMOUNT` / `GLASS_OFFSET_BEZEL` 删除（只用于旧的 `GlassRefract`）。
2. **`filter_with` 把 `Glass` 映射到 `Filter::Refraction`**，`Frosted` 映射到
   `Filter::BackdropBlur`，`Solid` 不带滤镜（identity）。`Filter::PixelOffset`
   保留在 core 里作为滤镜，但**不可通过 `material:` 到达**（`zstylec` 的 `material:`
   关键字只认 `glass` / `glass-refract` → `Material::glass()`）。
3. **磨砂看起来等于纯色的根因**：`control.button.frosted` 用了
   `@surface_alt_translucent`（α=0.85，近乎不透明）⇒ 磨砂与纯色视觉重合。
   新增主题角色 `frosted_glass_bg`（α=0.15，`roles::FROSTED_GLASS_BG`，住在
   `zephyra-core/src/theme.rs`），`control.button.frosted` 改用它；悬停/按下保留
   半透明填充，把反馈移到边框（边框变亮），而不是把填充改成不透明。
4. **Glass 的悬停/按下只亮边框**：填充保留玻璃本体（透明，只靠折射与描边显示），
   `hovered` / `pressed` 只把 `border-color` 调亮，不碰填充 ⇒ 玻璃质感不会被
   实色覆盖掉。
5. **参考页必须在真实照片背景上比材质**：gallery `MaterialBoard` / `cmp:*` 行
   放在 `bg-mountain` 照片上，容器本身用 `material: glass`，三档并排
   (`cmp:solid` / `cmp:frosted` / `cmp:glass`)，画面上一眼看出纯色遮住底图、
   磨砂模糊底图、玻璃折射底图。

## 排除的假设

- ❌ **「`material: glass-refract` 保留为独立变体」** → 与 `glass` 同一件事的两个
  拼法，语义重复 ⇒ 合并为 `Glass`，`glass-refract` 只留 `zstylec` 里的废弃别名。
- ❌ **「`Filter::PixelOffset` 也该能通过 `material:` 选」** → `PixelOffset` 是形状
  边位移（不采样背景），与「折射背景」的 `Glass` 是两套物理模型，混在
  `material:` 里会让设计师选错 ⇒ 留在 core 只作为滤镜，不进 `material:` 映射表。
- ❌ **「磨砂的悬停/按下改填充色给反馈」** → 填充改成不透明后磨砂又和纯色
  无法区分 ⇒ 保留半透明填充，反馈改在边框。
- ❌ **「新增一个专门的主题角色（如 `@frosted_glass`）比用 `@surface_alt_translucent`
  更好」** → `@surface_alt_translucent` α=0.85 太不透明，必须新角色
  `frosted_glass_bg`（α=0.15）才能让磨砂「透出背景」。

## 结论

`Material` 扎成三档：`Solid`（无滤镜）、`Frosted`（`BackdropBlur`，用
`@frosted_glass_bg` α=0.15，悬停/按下只亮边框）、`Glass`（`Refraction`，真实
斯涅尔折射，悬停/按下只亮边框）。`glass-refract` 只留 `zstylec` 废弃别名；
`Filter::PixelOffset` 留在 core 但不可通过 `material:` 选。参考页在
`bg-mountain` 真实照片背景上并排三档，容器本身用 `glass`。

## 门禁

| 门禁 | 钉住什么 |
|---|---|
| `zephyra-core` 核心单测 `filter_tests.rs`（`material` 相关断言） | `Material` 仅三变体；`filter_with` 把 `Glass`→`Refraction`、`Frosted`→`BackdropBlur`、`Solid`→identity |
| `zephyra-core` 核心单测 `tests.rs` | `Material::glass_refract()` 是 `#[deprecated]` 别名，与 `Material::Glass` 产出相同滤镜链 |
| `zstylec` 解析测试 | `material: glass` 与 `material: glass-refract` 都解析为 `Material::glass()`；`material: pixel-offset`（如曾有）被拒绝 |
| `zephyra-core` 主题角色（`theme.rs`） | 新增 `frosted_glass_bg`（α=0.15）与 `roles::FROSTED_GLASS_BG` |
| `gallery/tests/gates.rs` DISPLAY_ONLY 白名单 | `cmp:solid` / `cmp:frosted` / `cmp:glass` 三行在参考页，板 560×100px、背景 `bg-mountain`，容器 `material: glass` |
| `controls-button.zstyle` | `control.button.frosted` 用 `@frosted_glass_bg`；`control.button.glass` hover/pressed 只亮 `border-color` |

验收：`cargo fmt --all -- --check` 干净、`cargo clippy --workspace --all-targets
-- -D warnings` 0 警告、`cargo test --workspace` 全绿、`cargo doc --no-deps
--workspace` 无新增警告。

# 2026-10-30-button-materials

> 现象：`Button` 的背景只有纯色（`.standard`/`.accent`/`.subtle` 三变体，各自
> `background: @token`）。想接「纯色 / 模糊透明 / 液态玻璃」这类**材质**，仓库
> 却没有一条能选材质的通道：`.zstyle` 只能写颜色令牌，`filter` 又只能来自宿主
> 绑定（按名字、不按实例）。

## 排查

先把「材质到底能落在哪」摸清：

| 能力 | 现状 |
|---|---|
| `.zstyle` 写 `filter:` | **能**——`props_map` 有 `filter` 映射，`zstylec/value.rs` 有完整语法解析（`blur()` / `refraction()` / `drop-shadow()` / `none` / 链式）。玻璃面板当年不写只是**刻意**（为了参数可调），不是语法限制 |
| 「模糊**背后**画面」的滤镜 | **没有**——`Filter::Blur` 模糊元素**自己**（读 `g_tex`）；`Filter::Refraction` 采背景 `g_dst` 但只做**几何位移**、不模糊 |
| `filter` 能否内联到 `.zstyle` | 能，但它是**底层效果链**，不是高层「材质」概念 |

关键结论：`.zstyle` 完全能声明材质，缺的只是**一个「模糊背景」的滤镜**和**一个
高层的 `material` 属性**。用户拍板：真做背景模糊（动三后端），材质独立成一套
属性（不跟 `filter` 混用），`.zstyle` 只写**材质名一个词**，未定义默认纯色。

## 排除的假设

- **「材质做成 `Filter` 的一个变体，`.zstyle` 写 `filter: material(glass)`」**——
  这是「和 filter 混用」，用户明确否掉。材质是高层语义、filter 是底层效果链，
  两者正交：一个节点可以 `filter: drop-shadow(...)` + `material: glass`。
- **「模糊透明用 `Filter::Blur`」**——错。`Blur` 读 `g_tex`（元素自己），糊的是
  按钮内容不是背景。症状是「文字糊了、背景没糊」，与磨砂玻璃正好相反。
- **「模糊透明用折射+半透明近似」**——用户要真做。折射只位移不模糊，近似出不来
  「磨砂」。
- **「材质参数写进 `.zstyle`（`material: glass(24px, 20px, 0.3, 1.5)`）」**——
  用户要 `<Button material="glass"/>` 这种一个词。参数藏进 core 的默认值表，
  后续要加档位再单独做。

## 结论

**材质是一个独立属性 `MATERIAL` + 独立枚举 `filter::Material`，与 `filter` 正交。
材质在 fx 层展开成底层 `FilterChain`，三后端只看到展开后的滤镜、不知道 `material`
存在。**

| 层 | 改动 |
|---|---|
| **core** | `Filter` 加 `BackdropBlur { sigma }`（采 `g_dst` + 高斯卷积，不外扩、σ≤0 恒等）；新 `Material` 枚举（`Solid`/`Frosted`/`Glass`，参数藏默认值）；`props::MATERIAL`；`property::PropertyValue::Material`；`Node::filter()` 把材质展开的链**追加**到 `filter:` 链之后（Solid→空，filter-only 行为逐字节不变） |
| **zstylec** | `props_map` 加 `material`；`value.rs::enum_value` 加 `material:` 的 `solid`/`frosted`/`glass` 三词（**独立于** `filter` 的函数解析） |
| **fx** | `OffscreenPass` 加 `backdrop_blur: Option<BackdropBlur>`；`pass_for_chain` 把 `Filter::BackdropBlur` 当合成期参数（读 `g_dst`、`redraw_original=true`、与折射互斥后者覆盖前者）；`backdrop_blur_composite_quad`；`RenderReport::backdrop_blur_passes` |
| **D3D11 / D3D12** | `PSBackdropBlur`（HLSL，两份手抄同步）→ fxc 编译 `.ps.bin`；`PS_VARIANTS` 加一项；renderer 在合成趟按 `refraction`/`backdrop_blur`/普通三分支选管线，`g_dst` 副本条件合并两者 |
| **Vulkan** | `fs_backdrop_blur`（WGSL 手抄，与 HLSL 对应）；新管线；`SegmentPlan::Offscreen` 加 `backdrop_blur` 槽；`record_frame` 返回三元组 |
| **模板 + .zstyle** | Button 加 `material` 参数 → 发 `{material}` 类；`.zstyle` 定义 `Control.button.frosted`（半透明填充 + `material: frosted`）与 `.glass`（默认透明 + `material: glass`）；未声明 → 纯色 |
| **gallery** | basics 页加「材质」演示行（纯色 / 磨砂 / 玻璃），三语言文案 |

**关键决策**：

1. **材质与 `filter` 正交**：`material` 是独立属性，展开成滤镜链后**追加**到
   `filter:` 链，互不覆盖。`Material::Solid` 展开成空链 ⇒ 只写 `filter` 或都不写
   的节点，返回的链与之前逐字节相同。
2. **材质展开在 fx（跨后端共享层）**：三后端只看到展开后的 `FilterChain`，
   不知道 `material`。守住 AGENTS 边界④（三后端互不依赖）。
3. **`BackdropBlur` 与 `Refraction` 互斥、同读 `g_dst`**：两者都「替换背景那块
   区域」，一次只画一趟；链里后写者覆盖前者（与 `tint` 同一「就地替换」语义）。
4. **液态玻璃不写 `background: transparent`**：不写即默认透明（core 读不到
   background 时按 `rgba(0,0,0,0)`），显式写是噪声。

## 已知限制（本轮不做，记账）

- 材质参数（σ / 折射率…）是**固定默认值**，`.zstyle` 不能调浓淡。要调档位时加
  `material: frosted(2)` 一类语法 + `Material` 档位表。
- 磨砂玻璃的背景模糊是**单趟二维高斯**（截半径 12 + 步长抽样），不是 H/V 两趟
  乒乓 —— 因为读的是 `g_dst` 全屏副本、拿不到元素自身那条离屏 RT。磨砂到看不出
  细节即可，不追求逐像素精确。
- 只有 `Button` 接了材质；`ToolButton` / `PillButton` 等可照同一模式补。

## 门禁

- `cargo test -p zephyra-core --lib filter` —— 加 `backdrop_blur_has_no_outset_and_zero_sigma_is_identity`
  与 `material_expands_to_the_right_filter`（纯色→空链、磨砂→BackdropBlur、玻璃→Refraction）。
- `material_property_reaches_command` / `material_and_filter_coexist`（材质展开进
  `Command`，且与 filter 正交共存）。
- `zephyra-vulkan` 的 `fs_backdrop_blur_exists_and_samples_the_backdrop`（WGSL 入口
  存在 + 采样 `g_dst` + 高斯 + 1px 遮罩）。
- `zephyra-winit` 的 `shader_assets_are_in_sync`（所有 `.bin` 都比 `zephyra.hlsl` 新
  —— 改了 HLSL 必须重编**全部**变体，本轮 20 PS + VS × 两后端已重编）。
- 四条门禁 + gallery 整树门禁：fmt 干净；`cargo clippy --workspace --all-targets
  -- -D warnings` 0 警告；`cargo test --workspace` 1060 passed / 0 failed / 52 ignored；
  `cargo doc --no-deps --workspace` 无新增警告；gallery gates 41 全绿。
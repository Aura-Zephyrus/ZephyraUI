# 2026-10-24 Vulkan 后端补上折射（液态玻璃在 Linux 侧真正生效）

日期：2026-10-24。起因是用户一句话：「在 ZephyraUI 实现完整折射」——
背景是 Linux 侧的液态玻璃页（`gallery --route glass`）与「没设滤镜」无异。

---

## 一、现象

wgpu → 裸 Vulkan（ash）那一轮（`2026-10-linux-vulkan-raw.md`）之后，Linux 侧
的玻璃页三块玻璃**只有描边、没有弯折**：位移与色散都不存在。10-22 那条日志
的「未做」里记着「Vulkan：滤镜管线仍未接通，Linux 上这一页与没设滤镜无异」。

通读 `zephyra-vulkan/shaders/zephyra.wgsl`（当时 478 行）确认了根因不是
「没接线」而是**没有入口**：WGSL 里只有 `fs_main`、14 档混合变体、
`fs_blur_h/v`、`fs_composite` —— HLSL 那份有的 `PSRefraction` 在 WGSL 里
一个字节都没有。渲染器侧也没有对应的段计划分支。

## 二、排查：先确认缺的只有 Vulkan 两侧

| 查什么 | 结论 |
|---|---|
| fx 侧（`zephyra_fx::offscreen`） | **全都有**：`plan_segments` 产出 `pass.refraction`、`refraction_composite_quad` 按元素矩形建四边形、`redraw_original` 已置位 —— D3D12 一直在用，与后端无关 |
| core 侧（`filter.rs`） | **全都有**：斯涅尔位移的纯 Rust 镜像 + 四条契约门禁（IOR=1 位移为 0、厚度 0 恒等、位移 ≤ 厚度、离屏不外扩） |
| D3D12 渲染器 | 完整实现：`refresh_dst_copy` 取目标副本 → `PSRefraction` 合成 → 退化路径与 `refraction_passes` 计数 |
| WGSL | **没有折射入口** |
| Vulkan 渲染器 | **没有折射分支**：`record_offscreen` 第 3 步无条件画普通合成 |

所以移植面只有两个文件加一个宿主调用点，公式与契约全部照抄 D3D12 侧
（两份手抄、同一份契约，core 的纯 Rust 镜像钉着契约本身）。

## 三、排除的假设

- **「fx 侧要加东西」** —— 否。`OffscreenPass::refraction` 与
  `refraction_composite_quad` 在 fx 里躺了整整两轮，D3D12 已在用。在 fx 侧
  「补实现」只会得到第二份真相。
- **「折射可以塞进 `blurs` 乒乓链」** —— 否。模糊趟读 `g_tex`（离屏 RT，装的是
  元素自己），折射必须读 `g_dst`（全屏目标副本，装的是元素背后的屏幕）。
  `offscreen.rs` 的 `OffscreenPass::refraction` 文档写着这条，症状是「玻璃
  边缘把元素自己的轮廓糊开」。
- **「复用内容趟里 PS 变体那次快照」** —— 否。PS 变体在 RT 通道里走
  `snapshot_rt`（把 RT 内容按屏幕坐标拷进 `dst_rt`），那张图是**元素自己**；
  折射要的是 MSAA 里**已经画在屏幕上**的画面，必须走 `snapshot_dst`
  （resolve MSAA → `dst_rt`），且时机必须在合成之前。
- **「加一个顶点字段传屏幕尺寸」** —— 否。HLSL 用 `ddx/ddy` 推 `px2uv`，
  理由写在那里的注释里（`Vertex` 52 → 56 字节要同步改两个后端的布局）。
  WGSL 的对应物是 `dpdx/dpdy`，同样不占字段。
- **「顺手把内容趟也省了（纯折射链不采 RT）」** —— 否。D3D12 同样白画那一趟
  （10-22 日志「未做」记着），删它要动渲染器状态机（`absorb` 记账、
  `unimplemented_filters` 加减都在那一段），风险大于收益。本轮保持 parity。

## 四、结论：改了三处

1. **`zephyra.wgsl` 新增 `fs_refraction`**（+ `saturate` / `saturate2` /
   `glass_surface_height` / `sd_normal` 四个辅助），与 HLSL `PSRefraction`
   逐项对应：1px 遮罩、bezel 只按半边长夹、有界楔形偏折
   `δ = h·strength·sin(Δ)/cos(Θr)`、R/B 反向色散、预乘输出。
   文件头的「对应关系」从四条加到五条（新增折射那条）。
2. **`renderer.rs`**：
   - 新建 `fs_refraction` 管线（与 composite 同采样数、同通道、`src_over_blend`）；
   - `SegmentPlan::Offscreen` 增加 `element`（元素内容矩形）/ `corner_radius` /
     `refraction`（折射四边形的顶点偏移）三个字段 —— 折射四边形与合成四边形
     **字段约定不同**，两份都在 `prepare` 建好，录制时挑；
   - `record_offscreen` 第 3 步：折射段先 `ensure_dst_rt` + `snapshot_dst`
     （在通道外，resolve 是 transfer 类操作）取全屏背景副本，再用折射管线画；
     拿不到副本时退化画普通合成并计 `unimplemented`（与 D3D12 同语义）；
   - `record_frame` 返回 `(unimplemented, refractions)` —— 后者是
     `RenderReport::refraction_passes` 的记录来源，宿主 trace 打印。
3. **`host.rs`**：调用点解构新返回值，`ZEPHYRA_TRACE=1` 时打印折射趟数。

## 五、门禁

| 门禁 | 钉住什么 |
|---|---|
| `fs_refraction_entry_exists` | 入口存在（漏了它要到运行期建管线才炸） |
| `refraction_mask_is_a_one_pixel_band` | 遮罩 `edge = 1.0`（宽遮罩把最强折射乘成 0，实测差 6 倍） |
| `bezel_is_not_clamped_by_the_corner_radius` | bezel 只按半边长夹（按半径夹 ⇒ 倒角参数上 3/4 档空档） |
| `displacement_is_the_bounded_wedge_form` | 有界楔形式在、薄棱镜近似不在（旧式实测位移 491px vs 厚度 28px） |
| `dispersion_offsets_red_and_blue_in_opposite_directions` | 色散 R/B 反向（只拉一个通道 ⇒ 色散永不生效） |

五条全部**复现过失败再还原**（把 bezel 改回按半径夹，门禁当场红）。加上
build.rs 的 naga 构建期校验（WGSL 语法/类型错误在 `cargo build` 就炸）。

## 六、验收

- `cargo fmt -p zephyra-vulkan -- --check` 干净
- `cargo clippy -p zephyra-vulkan --all-targets -- -D warnings` —— 0 告警
- `cargo test -p zephyra-vulkan` —— 5 passed / 0 failed
- `cargo check -p zephyra-vulkan --all-targets` 通过（Windows 上可编译）

## 七、未做（诚实记账）

- **真 GPU 验证没做**：本机 WSL 被安全策略挡掉（`wsl.exe` 在程序黑名单里），
  Linux 侧的 gallery 一步都没跑起来。交付前必须在 WSL2 上实测：
  `cargo run -p zephyra-ui --bin gallery --no-default-features --features linux-vk -- --route glass`，
  判据两条 —— `ZEPHYRA_TRACE=1` 打「折射趟 3」，以及截图里三块玻璃的边缘
  弯折与色散可见（对照 10-22 日志 v8 截图的观感）。
- **单采样（`ZEPHYRA_MSAA=0`）下折射不画**：离屏链整体在单采样下不拆段
  （没有 MSAA 做 resolve 起点），折射与模糊同一条既有语义。
- **D3D11 的折射像素门禁**仍欠（与 D3D12 共用同一份 HLSL，内容会重复）。
- **液态玻璃的四层装饰**（镜面高光 / 内阴影 / 内缘亮线 / 玻璃外投影）仍未做，
  与前三轮一样一层没碰。

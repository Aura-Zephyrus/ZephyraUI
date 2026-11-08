# 2026-11-07 适配器标识与 wgpu 24 P0 前置（`AdapterIdentity`）

> 现象：`docs/backend-extension-interface.md`（v1.18，§13/#2349、§17.4#1、P0-4、S5）要求在 Phase 1 前验证 `wgpu = "=24.0.0"` 能否提取 `AdapterIdentity`（Windows LUID / Vulkan UUID 等）并记录结论；但当前仓库工作区**没有任何 `wgpu` 依赖**，无法在不引入新依赖/不接线的前提下做真实 API 探测。

## 排查

在工作区根目录做只读核对（2026-11-07，本机）：

```text
# Cargo.toml 中未发现实际 wgpu 依赖（仅注释提及）
Select-String -Path <workspace>/**/Cargo.toml -Pattern '"wgpu"|wgpu =' -SimpleMatch
# 结果：无匹配依赖行；命中均为注释（“不用 wgpu”“wgpu 版已删除”“零 wgpu”）

# 现有后端形态
zephyra-base/zephyra-vulkan/Cargo.toml  -> ash 0.37（裸 Vulkan），naga 构建期编译 WGSL→SPIR-V
zephyra-base/zephyra-winit/Cargo.toml   -> windows crate / D3D11 路线（见 crate 注释）
zephyra-base/zephyra-winit-d3d12/      -> D3D12 路线（独立 crate）
zephyra-base/zephyra-render-ext/Cargo.toml -> 契约层，已移除 zephyra-core/fx/bytemuck，注释“零平台依赖、零 wgpu”
zephyra-base/zephyra-ext/Cargo.toml    -> 仅依赖 zephyra-render-ext（独立 [workspace] 开发态，未入根 members）
```

设计文档要求与仓库现状的关系：

| 文档要求 | 仓库现状（本次核对） | 影响 |
|---|---|---|
| `wgpu = "=24.0.0"` 锁定（§13） | 无 `wgpu` 依赖；`zephyra-vulkan` 为 ash/raw Vulkan | P0 无法按“从 wgpu 24 Adapter 提取”做实测 |
| `AdapterIdentity::from_wgpu_adapter` 仅作探测方向（P0-4）/ 核心不得依赖 wgpu（S5） | `zephyra-render-ext::types::AdapterIdentity` 已为纯数据；`ext/render-ext` 无 wgpu | **契约侧已满足 S5/P0-4 数据形态** |
| 记录 `docs/dev-log/` 结论（§17.4#1） | 本文件补齐记录 | 满足“结论落 dev-log” |

## 排除的假设

- **假设：仓库里仍有 wgpu 24，可直接调用 `wgpu::Adapter`/`AdapterInfo` 做提取探测。** 排除：全工作区 Cargo.toml 依赖搜索无 `wgpu`；Linux Vulkan 路线明确为 ash（`zephyra-vulkan`），且历史记录显示 wgpu 版路径已删除/废弃（见 `examples/Cargo.toml`、`Cargo.toml` 注释、`2026-10-linux-vulkan-raw.md`）。
- **假设：`zephyra-render-ext`/`zephyra-ext` 需要依赖 wgpu 才能定义适配器标识。** 排除：S5 明确禁止；当前实现使用 `AdapterIdentity { vendor_id, device_id, luid: Option<u64>, device_uuid/driver_uuid: Option<[u8;16]>, name }` 纯值，转换责任在应用/后端接线层（feature-gated hal 或平台枚举对齐）。
- **假设：P0 必须在引入 wgpu 并改动后端后才能记录。** 排除：P0 的“结论+门禁”可以先记录**不可验证的前提条件与决策边界**，避免后续在无 wgpu 的仓库形态下误实现 `from_wgpu_adapter` 或误加依赖。
- **假设：Windows LUID 可作为跨平台通用标识。** 排除：P0-4 已裁决 Linux 无 LUID；`AdapterIdentity` 必须同时承载 `vendor/device` + 可选 LUID/UUID。

## 结论

1. **ext 契约层 P0-4/S5 已落地**：`AdapterIdentity` 纯数据、`zephyra-render-ext`/`zephyra-ext` 无 wgpu 依赖（本轮开发验证：`zephyra-render-ext` 16 tests、`zephyra-ext` 12 tests 通过，均在 ext 范围内）。
2. **wgpu 24 提取探测在当前工作区不可执行**：仓库无 `wgpu` 依赖，且 Vulkan 后端为 ash/raw Vulkan；按文档字面进行 `wgpu "=24.0.0"` `AdapterInfo`/hal 探测会要求新增依赖并触及后端/应用层，属于“接线/选型变更”，不应在 ext 未接线阶段隐式完成。
3. **后续接线决策（必须写死其一）**：
   - 若应用层未来引入 wgpu：在应用/feature-gated crate 提供 `wgpu::Adapter -> AdapterIdentity` 转换；核心/ext 仅收值。
   - 若继续 ash/D3D11/D3D12 直连：由各后端从原生 API（DXGI adapter LUID、Vulkan physical device UUID/properties）构造 `AdapterIdentity`；不引入 wgpu。
4. **`zephyra-ext` 当前为隔离开发态**：独立 `[workspace]`、未加入根 `members`（避免影响他人开发）；接线时需恢复成员/依赖继承并按根工作区门禁验证。

## 门禁

- 本条为 P0 记录门禁（设计流程）：
  - [x] 只读核对 `wgpu` 依赖/后端形态（grep Cargo.toml + 关键 crate 清单）。
  - [x] `AdapterIdentity` 存在且 `ext/render-ext` 无 wgpu 依赖（代码事实；对应测试已通过）。
  - [ ] **待后续接线阶段**：选定“wgpu 转换”或“原生 API 构造”路径后，补充可复现探测命令/输出（含平台、驱动/adapter 样本）并在实现中固定转换函数位置；若采用 `wgpu "=24.0.0"`，同时满足 §13 锁定与 §12#2 的 `cargo metadata` 校验。
- ext 代码门禁（本轮，scoped）：

```text
cargo fmt -p zephyra-render-ext --manifest-path Cargo.toml
cargo test -p zephyra-render-ext --manifest-path Cargo.toml --lib   # 16 passed
cargo fmt --manifest-path zephyra/zephyra-base/zephyra-ext/Cargo.toml
cargo test --manifest-path zephyra/zephyra-base/zephyra-ext/Cargo.toml --lib  # 12 passed
```

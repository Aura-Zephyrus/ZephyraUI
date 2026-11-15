# 滚动、裁剪与懒渲染能力补全计划

> 盘点范围：滚动容器、视口裁剪、overflow 样式、scissor 实现、窗口可见性感知。

## 现状

| 能力 | 现状 |
|---|---|
| 滚动容器 (`ScrollView`) | 不存在 |
| 视口裁剪 (`Clip/Mask`) | `PushClip`/`PopClip` 定义存在，但**零生产者** |
| `overflow: scroll/auto/hidden` | 样式表无此属性 |
| 滚动条 | 不存在 |
| 内容超出窗口 | 直接截断，无法滚动查看 |
| 窗口最小化/遮挡/屏幕外 | `frame()` 仍会被调用，内容有变时甚至持续 present |
| `should_draw_frame` 判据 | 仅检查设备死/交换链失败，不检查窗口可见性 |

## 计划（按优先级）

### P0
- [ ] `Node` 新增 `clip_rect: Option<Rect>` 字段
- [ ] `Clip.Rect` `PropertyKey` 加入 `zephyra-core/src/property.rs`（`affects_layout = false`）
- [ ] `zstylec/src/props_map.rs` 加 `clip-rect` → `Clip.Rect` 映射
- [ ] `zephyra-core/src/paint.rs` 的 `render_rec` 激活 `PushClip`/`PopClip` 产出，维护裁剪栈
- [ ] 三个后端 renderer 实现 scissor 设置/清除（D3D11 `RSSetScissorRects`、D3D12 命令列表、Vulkan `vkCmdSetScissor`）
- [ ] 各后端 `HostWindow` 新增 `window_visible` 字段，处理 `Occluded`/`Minimized` 窗口事件
- [ ] 各后端 `frame_inner`/`frame` 开头按可见性守恒：不可见时直接返回 `empty_frame()`，不进布局、不画、不 present
- [ ] 窗口从不可见 → 可见时 `cache.invalidate()` 强制下一帧重绘

### P1
- [ ] `Node` 新增 `scroll_offset: Point` 字段
- [ ] `Scroll.OffsetX/Y`、`Scroll.ContentWidth/Height` `PropertyKey` 加入
- [ ] `zephyra-core/src/layout.rs`：滚动容器子内容按内容尺寸测量，容器按视口尺寸测量
- [ ] `ZephyraUI/src/scroll_view.rs`：新增 `ScrollView` 控件（拖动/滚轮滚动、钳位）
- [ ] 三个后端：命令按 `-scroll_offset` 平移，命中测试反向平移
- [ ] 三个后端 `should_run_frame` 行为保持一致

### P2
- [ ] `zstylec` / `zypui`：`overflow: visible/hidden/scroll/auto` 解析与应用
- [ ] `ZephyraUI/src/scroll_bar.rs`：新增 `ScrollBar` 控件，绑定到 `ScrollView.scroll_offset`

## 验收标准

- [ ] `cargo fmt --all -- --check` 干净
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 0 警告
- [ ] `cargo test --workspace` 全绿
- [ ] 裁剪单测：嵌套 clip rect 产出正确有序的 `PushClip`/`PopClip` 对
- [ ] 滚动单测：scroll offset 超出范围时自动钳位
- [ ] `--dump-cpu` 验证：带 `overflow: scroll` 的页面产出 scissor 命令与平移顶点
- [ ] 窗口最小化：`frame()` 不再被执行，CPU/GPU 占用归零
- [ ] 窗口恢复可见：首帧完整重绘，无空白
- [ ] 窗口被完全遮挡：不持续 present

## 明确不做

- 跨帧局部重绘（damage 跨帧）：flip model 下后缓冲 present 后内容未定义，跨帧拼接不可行
- 窗口大小为 0 时的特殊处理：已由 `size.width.max(1)` 处理
- 多显示器上的窗口可见性：`Occluded` 已涵盖完全遮挡，部分遮挡继续正常渲染

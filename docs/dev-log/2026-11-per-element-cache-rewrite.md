# per-element cache rewrite（zypui + core 接口收口）

## 现象

原有缓存/重建路径以整页/整帧代价为主：zypui 每帧实例化整棵页面树，core `FrameCache` 决定 `Unchanged/Updated`，但 build 侧无法按元素/组件边界复用。目标是**融合 zypui，缓存对应每个元素，而不是窗口**，并保持单一渲染决策点。

## 排查 / 实施

- `Element` 子节点改为 `Vec<Arc<Element>>`（共享句柄），兼容 `with_children(Vec<Element>)` 与 `child_elements()`；`Node::from_element`/`diff` 适配。
- `zypui::Host` 增加可选 `state_revision()` / `binding_revision()`（默认 `None`，零迁移）；Gallery 实现 `state_revision()` 并在输入/点击时 bump `zypui_rev`，使 `uses_tr` 组件可在提供 revision 的宿主上安全参与缓存。
- `UiRuntime` + `ElementCacheStore` + `DepMask/DepSnapshot`：加载期计算静态依赖掩码（不改 `.zypui.bin`），运行期按 `(parent_cache_id, site_id, Key/Index/Single)` + 依赖快照缓存 `Use` 边界输出。`DepSnapshot` 保留 `Val`/状态比较（不压裸 hash），`RenderSource/Val::Raw` 沿用现有 `==` 语义。
- 结构化 site scope：`mix_site(parent,index,tag)`；`If` then/else 固定分支 scope；`For` 迭代位置化 scope（P1 不伪造跨重排稳定 item 身份）。
- 守卫（保守正确）：slot 回填、动态 `class_extra`、`dynamic_interaction`、`uses_tr` 且宿主无 `state_revision()`、`render_source` 属性依赖不独立缓存。
- `zephyra-style::SheetStyler` 增加 `(NodeTag,class names)` 匹配缓存（selector 语义不变）。
- `zephyra_core::frame` 增加 `RenderFrame = FrameCache` 过渡别名；后续 P4 clean-break。

## 排除的假设

- **“只缓存 key 就够”**：排除。重复 key、无 key `Use`、`For` 位置项会串身份；必须包含 `parent + site + variant`。
- **“Host revision 可作为唯一正确性来源”**：排除。revision 仅加速；快照仍保留值比较，宿主漏 bump 时 debug/oracle 路径可发现。
- **“把 `Element` 子树改成 `Arc` 会自动让 core paint/diff 零成本复用”**：排除。P1 只改 build/cache 与表示；core diff/style/layout/paint 算法仍按现有语义执行，P2b/P2c 需另行证明。
- **“`For` 无 key 时可在 P1 伪造稳定身份”**：排除。会造成错位复用；P1 允许位置化单元，P3 再加 track-by/key。

## P0 基线（dev，1400×800，2026-11）

命令：

```powershell
cargo run -p zephyra-gallery --bin gallery -- --dump-cpu --route reference
cargo run -p zephyra-gallery --bin gallery -- --dump-cpu --route slider
cargo run -p zephyra-gallery --bin gallery -- --bench --alloc-probe
```

结果（接入 UiRuntime/P2a/P2c/P3/RenderFrame 主名后的当前状态）：

| 场景 | 结果 |
|---|---|
| `--dump-cpu --route reference` | 四边形 444 / 顶点 1776（字形 404），滤镜 4，矢量 path 21 / 三角形 527，缺失纹理 0，内容像素 318965/1120000，输出 `gallery-cpu.bmp` |
| `--dump-cpu --route slider` | 四边形 199 / 顶点 796（字形 181），滤镜 1，矢量 path 15 / 三角形 454，缺失纹理 0，内容像素 269899/1120000 |
| `--bench --alloc-probe`（120 帧，丢弃 12 帧，最终） | 合计中位 505 µs / 最坏 755 µs；view 141 µs；diff 0 µs；layout 0 µs；paint 0 µs；expand 183 µs；四边形 213 / 顶点 852；分配分布显示大块/中块计数下降（相对早期基线 447 次 / 79440 字节中位） |

注：`gallery --list-routes` 当前合法路由为 `guide,state,styling,layout,theme,testing,reference,materials,slider`；计划草案中的 `buttons` 需映射到实际路由后做精确对照。

## 结论 + 门禁

- **正确性优先**：缓存命中必须在依赖快照/守卫成立时才复用；Gallery `gates`、`hover_layout_tests`、`dropdown_tests`、`zypui` 等价性测试是最低门禁。
- **单一渲染决策点不变**：`FrameCache/RenderFrame::frame()` 仍是 `Unchanged/Updated` 的唯一来源；zypui cache 只影响 build 输入，不在宿主/后端引入第二套“该不该画”判断。
- **后续门禁**：P2b 需 `DisplayList == oracle`（debug/test）；P2c 需证明子树句柄/快照不变时 diff 下探跳过不破坏语义；P3 修改 `.zypui.bin` 时需再生成 gallery artifacts 并跑 gates/bench/dump-cpu。

## 完成状态（收口）

```text
cargo test -p zephyra-core --lib                                   434 passed
cargo test -p zephyra-style                                        7 passed
cargo test -p zypui                                                17 passed
cargo test -p zephyra-gallery --test hover_layout_tests            3 passed
cargo test -p zephyra-gallery --test dropdown_tests               14 passed
cargo test -p zephyra-gallery --test gates                         58 passed
cargo fmt --all -- --check                                         clean（stable wrap_comments 警告除外）
cargo clippy -p zephyra-core -p zephyra-style -p zypui -p zephyra-gallery --lib -- -D warnings  clean
```

## 完成状态（收口）

- **P1 深化**：UiRuntime 增加 CacheStats/cache_stats()、invalidations、set_oracle_check()；DepSnapshot 增加 host_scope（修复共享 runtime 下跨 Gallery 实例缓存串扰，触发 gates 失败后已修复）；ElementCacheStore 增加 len/invalidate。Gallery 提供 cache_scope + state_revision()，uses_tr 在有 revision 的宿主上可缓存。
- **P2b（保守完成）**：RenderFrame 增加 debug paint_oracle，可对 Updated 帧做 DisplayList == oracle 全量重绘比较。frame 级 Unchanged 已跳过 paint/present；DisplayList 全局 (layer,z,order) 排序+交错命令使 per-subtree 区间复用架构侵入大，当前保持全量 repaint 正确路径 + oracle gate。
- **P2c**：Node.source: Option<Arc<Element>>；diff 在 Arc::ptr_eq 且 key/tag 匹配时跳过子树下探，保留既有 dirty/布局需求传播。
- **P3**：ForNode.track_key 贯通 IR/zypuic 解析（key/	rack-by）/encode/decode（.zypui.bin 由 build 重生成）；instantiate 支持 keyed/positionized For item 缓存。fx/render-extension 无需额外 artifact 迁移（构建期重生成）。
- **P4**：RenderFrame 主名 + FrameCache 兼容别名；Ui::build 保留为 oracle/兼容（生产热路径 Gallery 已用 UiRuntime）；dev-log/计划更新。

## 最终验证

`	ext
cargo test -p zephyra-core --lib                                   434 passed
cargo test -p zephyra-style                                        7 passed
cargo test -p zypui --lib                                          19 passed
cargo test -p zypuic --lib                                         42 passed
cargo test -p zephyra-gallery --test hover_layout_tests            3 passed
cargo test -p zephyra-gallery --test dropdown_tests               14 passed
cargo test -p zephyra-gallery --test gates                         58 passed
cargo fmt --all -- --check                                         clean（stable wrap_comments 警告除外）
cargo clippy -p zephyra-core -p zephyra-style -p zypui -p zephyra-gallery --lib --tests -- -D warnings  clean
cargo run -p zephyra-gallery --bin gallery -- --bench --alloc-probe  合计中位 505 µs, view 141 µs, expand 183 µs (1400×800, 120 帧, dev)
`

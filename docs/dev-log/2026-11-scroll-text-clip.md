# 滚动容器裁掉了底色却没裁文字（视口下方飘着一行行没有底色的字）

> 2026-11-15 · Gallery（D3D12 后端，1400×800 物理像素，scale 1.25）· 现象来自用户截图
> 排查手段：`zephyra bridge`（调试桥）+ `shot` 像素读回，**没有靠肉眼猜**

## 现象

滚动容器（`overflow: scroll`）里内容溢出视口时：

- 溢出条目的**底色盒子**被裁掉了（看不见）—— 这部分是对的；
- 溢出条目的**文字**却画到了视口**外面**，在页面背景上飘着一行行没有底色的字。

截图上看着像「裁剪只对矩形生效」。截图本身看不出**哪一类图元**漏了，所以第一反应
（本仓已经犯过一次的）会是「裁剪没生效」或「`overflow` 没解析」。

## 排查

### 1. 先用桥把「属性链」整条读回来

```text
$ zephyra bridge "tree" | Select-String "scroll:demo|item:"
  0.1.1.1.0.1.1.0.1.0.0    Box scroll:demo 436,194 320x240 scroll-view
  0.1.1.1.0.1.1.0.1.0.0.0  Box content 436,194 320x568 scroll-view-content
  0.1.1.1.0.1.1.0.1.0.0.0.0 Box item:0 436,194 320x64 scroll-item
  …（item:1..item:7 在 266 / 338 / … / 698）

$ zephyra bridge "props 0.1.1.1.0.1.1.0.1.0.0"
  Overflow = Scroll (styled)
  Scroll.OffsetX = 0
  Scroll.OffsetY = 0
```

⇒ **样式表那侧是通的**：`overflow: scroll` 解析成了 `Overflow::Scroll`，裁剪区就是
视口本身（320×240），内容高 568 ⇒ 最大可滚 328。

### 2. 交互链也通

```text
$ zephyra bridge "scroll 500 300 0 120" "props 0.1.1.1.0.1.1.0.1.0.0"
  Scroll.OffsetY = 328        ← 滚轮 → 模型 → 写回节点，三段都在
```

拖动也通（`pointer_down` → `hover` → `pointer_up`）：偏移从 0 变到 36。
（36 而不是 46：宿主把指针坐标除以 scale=1.25，50 物理像素 ⇒ 40 DIP，再扣 4 DIP
slop。**布局在物理像素、控件模型在 DIP** 这件事见
`docs/developer/responsive-and-touch.md` §5，是已知待办，不是本条。）

### 3. 像素证据：底色没了，文字还在

`zephyra bridge "shot --repaint --out x.png --rect 430,400,180,120 --scale 4`
读回 180×120 那一块（视口下沿 y=434 在裁剪区第 34px 处）：

- `#4` 的盒子在视口下沿被**整齐切断** ⇒ 裁剪**确实生效**了；
- 切口下方是**页面背景**（没有 `#4` 的字）⇒ 视口里的字也没漏；
- 再往下 50px，`#5` 的**字**出现了，而它**没有底色盒子** ⇒ 底色被裁了，文字没裁。

⇒ 漏的不是「裁剪区算错了」，而是「**某类图元根本没读裁剪栈**」。

### 4. 根因

`zephyra-fx/src/quad.rs` 的 `expand_range_translated_into` 里，四条图元分支对
`clip`（裁剪栈顶）的处理**不一致**：

| 分支 | 处理 |
|---|---|
| `Command::Quad` | `intersect(rect, clip)` ⇒ 盒子的底色被裁 ✓ |
| `Command::Image` | `intersect(dst, clip)` ✓ |
| `Command::Path` | Sutherland–Hodgman 逐三角形裁 ✓ |
| `Command::Text` | **整段字形直接进顶点流，一个字都没问过 `clip`** ✗ |

裁剪是 **CPU 侧的几何求交**（不是 GPU scissor）：三个后端都走同一个
`expand_range_translated_into`，所以这条漏在**所有后端**上，且 CPU 参考图
（`--dump-cpu`）与 GPU 画面**一致地错** —— 这也是为什么「两边对照」查不出它。

## 排除的假设

- **「`overflow` 没解析 / 样式表没加载」**：排除。`props` 显示 `Overflow = Scroll (styled)`，
  且 `controls-scroll.zstyle` 在 `ZephyraUI/src/styles.rs` 的加载表里（`scroll.zstyle: 8 rules`）。
- **「`PushClip` 根本没发出来」**：排除。裁剪区实测就是视口 `(436,194,320,240)`，
  且 `#4` 在视口下沿被整齐切断 —— 裁剪**确实在跑**。
- **「D3D12 后端没实现 scissor」**：排除（本条一开始就是这么猜的）。本仓的裁剪**不是**
  GPU scissor，而是 fx 层把几何裁掉；D3D11 / D3D12 / Vulkan 共用同一条展开路径。
  真正的漏是**展开层少了一个分支**。
- **「滚动偏移没写回节点 / 画面没跟上」**：排除。`Scroll.OffsetY` 随滚轮与拖动变化，
  截图里内容也确实移动了。
- **「命中测试没算滚动偏移」**：排除。真实输入走 core 的命中测试（`visual::enter`
  按 `-scroll_offset` 平移）。⚠ 但桥的 `hit` 命令是**布局矩形探针**
  （`bridge.rs::hit_at` 直接用 `arranged_rect()`），**不算滚动偏移也不算裁剪** ——
  拿它判断「点到的是哪个条目」会得到与真实命中不同的答案。排查时别把它当真相。
- **「只是旧构建 / 缓存」**：排除。修之前的当前构建稳定复现，修之后消失（同一台机器、
  同一份样式表、同一条 CLI 命令）。

## 结论 + 门禁

**修法**（`zephyra-fx/src/quad.rs`，`Command::Text` 分支）：字形四边形与裁剪区求交，
**uv 按比例跟着切**。

uv 必须一起切：纹理映射在两轴上都是线性的（见 `Vertex::quad`），所以按比例裁 uv
是**精确**的；只缩几何不缩 uv ⇒ 纹理被拉伸 ⇒ 字形变形。

**门禁**（`zephyra-fx/tests/unit/tests/quad_tests.rs`，三条新用例）：

- `text_fully_outside_the_clip_produces_nothing` —— 完全在外的字形**一个字都不产出**
  （**改之前是红的**：整段文字照画）。
- `text_crossing_the_clip_is_cut_with_proportional_uv` —— 跨界的字形被切到交集，
  且 uv 按比例裁。
- `text_fully_inside_the_clip_is_unchanged` —— 完全在内的字形几何与 uv 一字不改
  （防「裁剪把字形切变形」）。

**实测**（D3D12，1400×800 物理像素，scale 1.25）：

```text
cargo fmt --all -- --check                              clean
cargo clippy --workspace --all-targets -- -D warnings  0 警告
cargo test --workspace                                   1538 passed / 0 failed / 73 ignored
cargo doc --no-deps --workspace                          退出 0（39 条警告，全是既有的，无新增）

zephyra bridge "shot --repaint --out x.png"（视口下沿 430,400,180,120 @4×）
  修复前：#5 的文字在视口外、底色盒子没了
  修复后：视口外什么都没有；#1..#4 在视口内、#4 在下沿被切
```

**顺带发现**（本条排查里撞到的，与本 bug 无关，单独记账）：

1. `gallery/src/bin/main.rs` **没有 D3D11 分支** —— `Cargo.toml` 里 `d3d11` 这个
   feature 存在，但 bin 里只有 `#[cfg(all(windows, feature = "d3d12"))]` 与
   `#[cfg(all(not(windows), feature = "linux-vk"))]`。于是
   `--no-default-features --features d3d11` 编得出来，跑起来只会打印
   「需要一个后端特性才能开窗口」。`skills/zephyra-mcp/SKILL.md` 里写的
   「`--no-default-features --features d3d11` 切 D3D11」与实现对不上。
2. 桥的 `hit` 命令不算滚动偏移与裁剪（见上面「排除的假设」），当探针用可以，
   当真相不行。

# 2026-11-01 材质专区 + 液态玻璃合并，背景换成真实照片

> 现象：新增「材质专区」并与「液态玻璃」合并成一页（路由 `glass` → `materials`），
> 页面中央是一张真实山景照片当背景板，板上浮着三块材质样板（纯色 / 磨砂 / 液态玻璃）
> 与三块可调参数的玻璃块。**照片把压在它上面的「纯色」样板连同样板里的文字标签
> 一起盖掉了** —— 画面上那一块只剩照片，纯色样板「消失」。

## 需求与口径

| 问题 | 结论 |
|---|---|
| 「材质的专区」 | 新增三块材质样板：纯色 / 磨砂 / 液态玻璃 |
| 「和液态玻璃合并」 | 两者合成**一页**（路由改名 `glass` → `materials`），参数调节器保留在同一页 |
| 背景换成图片 | 用户指定 `C:\Users\Levy\Downloads\kinkate-mountain-landscape-2031539_1920.jpg`；**照片当玻璃舞台背景**（用户在三选一里明确选了这一项），不是整页壁纸 |
| 照片从哪来 | 编译期 `include_bytes!` 内嵌 JPEG，运行时解成 RGBA8 注册进宿主（与字体同一待遇 —— 运行时从磁盘找的话 cwd 一变照片就没了，而症状「背景全黑」与「纹理没注册上」无法区分） |

落地：`gallery/assets/mountain.jpg`（裁切 1556×1020 → 1464×960，宽高比 1.525 与照片板
732×480 精确一致，无非等比拉伸）、`gallery/src/bin/main.rs`（JPEG 解码 + `register_texture`）、
`gallery/frontend/shell.zypui`（`PageMaterials` / `MaterialBoard` / `MaterialSwatches` /
`GlassPanes`）、`gallery/frontend/pages.zstyle`（`.material-board` 带 `image-texture`）、
`gallery/frontend/app/{mod.rs,pages.rs}`（路由与 i18n）、`gallery/tests/gates.rs`（门禁）。

## 排查

现象看上去像「这卡片没画出来」，而 DisplayList 里明明有它 —— 先确认命令在不在。

| 步骤 | 结果 |
|---|---|
| MCP 桥 dump UI 树 | `swatch:solid` 在（398,348 160×120）、`chip:solid` 在（461,396 34×24）、`Text sw:solid` 在 —— **节点都在** |
| `get_props` 看展开属性 | `Background = #F3F7FBD9`、`Material = Frosted`（磨砂那块）—— **属性都在** |
| 临时测试打印 DisplayList | `Quad fill=#E6EDF7D9 stroke=#FFFFFFB3` + `chip` 的 `Quad fill=#0B1220B3` + `Text glyphs=5` —— **合成命令都在** |
| CPU 参考栅格出图 | 标签牌画出来了 —— 但那张图**没有照片**（CPU 侧不采样纹理，`missing_textures=1`），所以「照片盖住卡片」这条在 CPU 图上看不出来 |

⇒ 命令在、属性在、CPU 路也能画 ⇒ 问题不在 core 的 paint 层，也不在 zstyle。

## 排除的假设

### 1. 「纯色底色太浅，压不住雪地」

把 `.material-solid` 的底色改成**纯红不透明** `#ff0000ff`，重编重截 —— 那块**依然**是照片，
一点红都没有。⇒ 不是颜色/对比度问题。

### 2. 「照片纹理没注册上」

首帧日志 `offscreen=5 refraction=4 unimplemented_filters=0`，无 missing texture；照片本身
在画面上清清楚楚。⇒ 不是纹理。

### 3. 「材质滤镜把子树吃掉了」

磨砂与玻璃两块样板的**标签牌都在**（同一个 `photo-chip` 类、同一份属性），只有 `material-solid`
那一支整棵子树没画。若是滤镜的问题，`frosted` / `glass` 应当同样受害。⇒ 不是滤镜。

### 4. 「zstylec 把整份 pages.zstyle 丢了」

排查过程中**真的撞到过一次** zstylec 的坑：用 `// image-texture: …` 注释掉一行来做实验，
结果 zstylec **不认 `//` 注释**，整份表被丢弃（`pages.zstyle has 1 error(s) -- skipping it`），
页面退回默认排版 —— 那一轮实验因此完全无效。**教训**：`zstylec` 的注释语法是 `/* … */`，
不是 `//`；而**一份表被丢弃的后果不是「那条规则被跳过」而是整份表都不生效**。
但这条与本问题无关：恢复正确 zstyle 后照片正常出现、纯色样板仍然消失。

### 5. 「CPU 与 GPU 不一致，GPU 画错了」

CPU 参考图里标签牌在 —— 但那张图**没有照片**，所以它证明不了什么。把 CPU 侧也查一遍后
发现：CPU 参考栅格与三后端吃的是**同一份** `zephyra_fx::quad` 展开结果，两者都按
「实心 → 图像 → 多边形」的顺序画。⇒ **不是某一端画错，是展开层本身排错了序**。

## 结论

`zephyra-fx/src/quad.rs` 的展开层把顶点分三段存（`solid` / `images` / `paths`），
`merge_runs` 把三份段内记录**按类别**拼成一份 `DrawRun` 列表：

```rust
add(&tally.solid, 0);
add(&tally.images, solid_len);
add(&tally.paths, solid_len + images_len);
```

渲染器是**逐 `run` 回放**的（`runs` 的先后 = 绘制顺序），于是绘制顺序变成了
**图元类型分组**而不是 DisplayList 顺序 ⇒ 任何 `Command::Image` 都排在同一段里
**所有**实心四边形之后绘制。

照片是板的 `image-texture`，它在 DisplayList 里排在材质样板**之前**，但因为它是图像，
实际绘制时被推到了所有实心四边形**之后** ⇒ 照片盖住纯色样板与它的标签。

⚠ 三处「恰好对上了」让这个缺陷活到今天：① demo 与 gallery 的其它页面**一张图像都没有**，
于是「类型分组」与「DisplayList 顺序」完全一致；② 既有的 `runs` 门禁用的 DisplayList
也是同序排列的（实心→图像→多边形），断言「按 `first_vertex` 升序首尾相接」因而恒绿；
③ D3D12 渲染器里那段「为什么按 `DrawRun` 走而不是按图元类型分三段」的注释**恰好说对了
目标、却没发现目标没达成** —— `runs` 的顺序对了，但 `runs` 本身已经是分好组的。

## 修复

不动顶点流的三段布局（重排它要连带改三后端的索引定位），只**把 run 的先后改回
DisplayList 顺序**：

- `RunTally` 由三份（`solid` / `images` / `paths`）合成**一份**，按 DisplayList 顺序记；
- 图像的顶点与 run **在命令遍历里就地产生**（分批规则不变：只合并相邻的同纹理同混合段）——
  原来它们被推进临时队列、遍历结束后才统一分批，于是所有图像的 run 都排在实心之后；
- `merge_runs` 按记录顺序输出，基址仍按类别加（实心在流头、图像在中、多边形在尾）；
- 段能否合并的判据从「同类别里的上一条」改成「**紧邻的上一条**」——
  「实心 → 图像 → 实心」里两个实心**不能**并（并了就等于把后一个提到图像之前画）。

`DrawRun` 的公开结构与三后端的用法**一字未改**：三后端的索引定位
（`run.first_vertex >= path_first`）是**逐段独立**的，与段的先后无关。

CPU 参考栅格（`cpu_raster::draw_batch`）同步改成**逐 `run` 画** —— 它此前按
`solid` / `images` / `paths` 三段顺序画，是同一个缺陷的另一半；参考图与实测图若顺序不同，
参考图就不是参考了。

## 门禁

| 门禁 | 位置 | 钉住什么 |
|---|---|---|
| `an_image_does_not_cover_later_solids` | `zephyra-fx/src/tests/quad_tests.rs` | 图像在前、实心在后时，`runs` 的先后必须与之一致 |
| `a_solid_between_images_keeps_its_own_run` | 同上 | 两个实心之间夹着图像时**不能**并段 |
| `two_images_with_a_solid_between_keep_two_runs` | 同上 | 两张同纹理图像之间夹着实心时**不能**并段 |
| `a_solid_drawn_after_an_image_covers_it` | `zephyra-fx/src/tests/cpu_raster_tests.rs` | **像素级**：蓝图像 + 后画的红色实心，CPU 画布上必须是红色 |
| `stress_mixed_commands_keep_displaylist_order` | `zephyra-fx/src/tests/quad_stress_tests.rs` | 900 条**交错**命令：段序列必须等于命令序列的游程编码 |

既有的 `runs_spliced_rebuild_the_unsplit_stream` 与压力测试的「runs 铺满顶点流」断言
从「按 `runs` 的顺序首尾相接」改成「**按 `first_vertex` 排序后**首尾相接」——
段的先后是 DisplayList 的顺序，`first_vertex` 是它在合并流里的位置，两者的序不必一致。

**变异验证**：把 `merge_runs` 改回按类别分组（`sort_by_key`），上面三条展开层门禁与
压力门禁当场全红；改回来全绿。

## 顺带记下

- **照片裁切也是设计**：初版裁的是全高（1647×1080），样板行正压在最亮的雪地上，
  「纯色」与「磨砂」的差别被压平。改成裁掉大块天空（1556×1020+182+58）后，
  样板行落在**岩石峰体**上（磨砂糊化、玻璃掰弯都看得出来），下半是石阶与花海
  （折射与色散最明显的地方）。
- **`zstylec` 不认 `//` 注释**（只认 `/* … */`），而一份表解析失败的代价是**整份表被丢弃**，
  页面静默退回默认排版 —— 与「多值 padding」同一个坑。

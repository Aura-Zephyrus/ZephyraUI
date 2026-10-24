# 画廊两处「画面不更新」：陈旧文字成形缓存 + MSAA 折射读错源

> 现象一：`gallery` 液态玻璃页点 `Thickness` 的 `−`/`+`，**数字不动**。要等一次 resize
> 或换页之后，才一次性跳到最终值。
>
> 现象二：从「状态」页点导航进「液态玻璃」页，三块玻璃**里面**浮着上一页的控件残影
> （`lay-only (core boundary)` 文字、两条滑块、开关的圆钮），而玻璃**外面**一切都是新的。

两个缺陷互相独立，只是同一轮报出来的。第二个在排查过程中还牵出一个更贵的坑：
**「点导航页面不换」看着像输入坏了，实际是渲染循环被一条非法命令冻死了。**

---

## ★ 结论（先看这段）

### 缺陷一：`TEXT` 是纯绘制属性，而字形缓存按「旧文字」存的

`TEXT` 被有意钉成 `affects_layout = false`（`layout_classification_tests.rs`：
「内容变了不等于盒子要重排」）。于是改文字既不清 `measure_cache`、也不重跑布局 ——
这本意是对的。但 `paint::text_command` 在字形缓存未命中时，读的是
`Node::cached_shaped()`（**上一次布局**存下的 `ShapedText`，还是旧文字），
拿它算完字形之后，又把**新文字**写进 `glyph_quads` —— 缓存被投毒：
旧字形一直留在屏幕上，直到 `Constraints` 变化（resize / 换页）把整条链冲掉。

修法：`glyph_quads` 未命中时**无条件按当前文字重新成形**。稳态快路径本来就有
字形缓存（按 文字 / 样式 / max_width / 图集版本 做键）兜着，所以没有每帧重新成形的代价。
`MeasureResult.shaped` 与 `Node::cached_shaped()` 随之成了只写不读的死字段，一并删掉。

### 缺陷二：折射副本取的是**后缓冲**，而那上面的内容是**上一帧**的

MSAA 开着时本帧画进的是**多重采样纹理**，后缓冲要等帧末 `Swapchain::resolve_msaa`
之后才有内容。而 `refresh_dst_copy`（折射的 `g_dst`、PS 变体混合的 `dst` 都靠它）
拷的是后缓冲 —— 于是折射采样到的是**上一帧解析后的画面**：换页那一帧，上一页的控件
正好还留在那张图里，玻璃就把它们折出来了。

对照实验一次定案：`ZEPHYRA_MSAA=0`（此时绘制目标**就是**后缓冲，两源合一）走同一条
导航，玻璃里**一块残影都没有**。

修法：宿主把**本帧真正的绘制目标**传进去（`chain.msaa_tex()` 优先，否则后缓冲），
`refresh_dst_copy` 按源资源的 `SampleDesc.Count` 分岔：

- 多重采样 ⇒ `ResolveSubresource`（逐样本求平均，与帧末解析同语义）；
  拷贝类调用一律要求源目标**采样数相同**，4× → 1× 是非法的；
- 单采样 ⇒ `CopyTextureRegion` + 源框（只取**内容区**）。

### 顺带挖出来的第三个坑：解析目标的状态必须是 `RESOLVE_DEST`

第一版修法用的是 `RENDER_TARGET`（照抄 `swapchain.rs` 的 `resolve_msaa`，那条一直是好的）。
结果玻璃页第一次渲染就录进一条**非法命令**：

```text
[zephyra-d3d12] 提交失败: Close 命令列表失败: 参数错误 (0x80070057)
[zephyra-d3d12] 开帧失败: Reset 命令列表失败: 参数错误 (0x80070057)    ← 此后每帧
```

`RENDER_TARGET` 当解析目标**只对交换链后缓冲成立**。换成普通提交式纹理后，
`ResolveSubresource` 照样能被记录进命令列表 —— 错要到 `Close()` 才现形，于是整帧
提交失败、`Reset` 也失败，画面**永久冻结**在最后一帧。外部症状是「点导航页面不换、
点控件没反应」，与
[`2026-10-gpu-device-reset-dead-clicks.md`](2026-10-gpu-device-reset-dead-clicks.md)
那一族**一模一样**，会把人往输入/路由方向带（本轮就被带偏了大半程）。

正解：解析目标转 `RESOLVE_DEST`。

---

## 现象与排查

### 先确认「截图坐标 = 点击坐标」

两个缺陷都只能靠真 GPU 出图判，所以先得让「看着像什么」与「点到的是哪儿」可信：

- 截图与点击都以 `GetWindowRect`（**外框**）为原点，两边同一套坐标；
  进程先 `SetProcessDpiAwarenessContext(-4)`，否则拿到的是虚拟化坐标，
  1:1 比对从第一步就不成立；
- 截图是 1418×847（外框），去掉 DWM 边框/标题栏后正好是应用自报的 1400×800 ——
  **截图是物理像素**，没有第二次重采样；
- 按像素扫侧栏得到的行心（115 / 162 / 206 / …）与 `shell.rs` 的几何公式
  （`TITLEBAR_H 44` + `padding 12` + `46·i` + `20`）**逐项对得上** —— 坐标不是猜的。

### 缺陷一：数字为什么非得等一次 resize

`InputState` 的那一路（点击 → `glass.nudge` → 改 `TEXT` 属性）一直是好的：
`ZEPHYRA_TRACE` 下 `quads` 数确实变了。变的是**字形**那一层：

| 层 | 键是什么 | 文字变了会不会失效 |
|---|---|---|
| `measure_cache` | `Constraints` | **不会**（`TEXT` 不参与布局） |
| `glyph_quads` | 文字 + 样式 + max_width + 图集版本 | 应该会 —— 但当时存进去的是错的东西 |

于是「数字不动」的判据就变成一句话：**`quads` 数变了吗？**
变了 ⇒ 输入与布局都正常，问题在成形；没变 ⇒ 再往上游查。这次是前者。

### 缺陷二：为什么只有玻璃里面脏

因为只有玻璃那三块会去**读目标**。玻璃外面的控件是直接画进目标的，永远是最新内容；
玻璃要先把「背后的东西」拷一份来采样，拷错了只影响它自己那三块 —— 这个分布
（脏的恰好是三个圆角矩形内部、且内部是**上一页**的控件）已经把范围指到了
「折射的 backdrop 副本」，不需要再去怀疑绘制顺序或裁剪。

### 那条「冻结」是怎么被揪出来的

先观察到：导航点击**有时生效、有时不生效**，而 hover 高亮**一直生效**、
状态页的复选框点击**一直生效**。这三个事实放一起，指向「不是输入坏了」。
决定性的一步是拿回码说话 —— 把 stderr 落到文件后看到上面那两行 `0x80070057`。
再进一步：用临时开关整段跳过 `refresh_dst_copy`，同一条导航**立刻正常**，
于是范围锁死在那一个函数里。之后就是二分：先换回 5 参 `ResolveSubresource`
（仍是错状态）→ 依然失败；把目标状态改成 `RESOLVE_DEST` → 通。

---

## 排除的假设

| 曾经的怀疑 | 排除方式 |
|---|---|
| DComp 1.25× 错位 / DPI 虚拟化让截图与点击错开 | 截图是物理像素（1418×847 外框 vs 应用自报 1400×800），且侧栏行心与 `shell.rs` 几何公式逐项吻合；点击与截图共用同一 `GetWindowRect` 原点 |
| 窗口位置漂移导致点歪 | `click.ps1` 每次都重读 `GetWindowRect`；另加一条「蓝色强调条落在哪一行」的像素探针，选中行始终可判 |
| 导航项把点击路由给了子节点（`label` / `icon`），所以 key 不是路由 id | 同一构造的「样式」行点一下就能跳转；且本缺陷修好之后所有行都能跳 |
| 输入路由 / `resolve_route` 认不出 `glass` | `resolve_route("glass")` 的 id 就在 `ROUTES` 里；更关键的是**跳过 `refresh_dst_copy` 后同一次点击就能跳转** —— 输入那一侧全程是好的 |
| `ResolveSubresourceRegion`（带源矩形）这个 API 本机不支持 | 它确实失败，但换回 5 参 `ResolveSubresource` 配**同一个错状态**也照样失败 —— 说明问题在**状态**，不在 API |
| `CopyResource` 只是「某些驱动只警告就继续」（D3D11 后端那条注释） | D3D12 这条路上它**不静默**：命令录进去了、`Close()` 才炸、整帧提交失败、画面冻结。而且残影的成因根本不是采样数不匹配，是**源取错了资源**（后缓冲 vs MSAA 纹理） |
| 折射的几何 / 遮罩 / 混合算错了 | `ZEPHYRA_MSAA=0` 一次对照即排除：同样的几何、同样的混合，一块残影都没有 |

---

## 现在的行为

### `refresh_dst_copy` 的两条岔路

```text
源 = 本帧绘制目标（MSAA 开 ⇒ 多重采样纹理；关 ⇒ 后缓冲）
│
├─ SampleDesc.Count > 1  ⇒ 转 RESOLVE_DEST → ResolveSubresource → 转 PIXEL_SHADER_RESOURCE
└─ SampleDesc.Count == 1 ⇒ 源转 COPY_SOURCE、目标转 COPY_DEST → CopyTextureRegion(内容区) → 各自还原
```

采样数与尺寸从**资源本体**读（`ID3D12Resource::GetDesc`），不是从调用方的约定 ——
同一个函数要同时伺候三种源：多重采样纹理、后缓冲、离屏 RT（PS 变体混合读自己）。

副本自身带一个 `state` 字段，屏障的 `StateBefore` 用它。原先那份代码每帧都假定
「副本还在 `COPY_DEST`」，而它第一帧之后一直在 `PIXEL_SHADER_RESOURCE` ——
`StateBefore` 说谎在调试层是报错，在没有调试层的机器上则会让下一次拷贝把
「还挂在 SRV 表里的资源」当成可写副本。

### 已知边界：交换链有余量时放弃本次折射

`Swapchain::resize` 按 25% 余量增长容量，所以**窗口变大之后**容量会大于内容尺寸，
MSAA 纹理（按容量建）也就大于按内容尺寸建的副本。`ResolveSubresource` 解析的是
**整个子资源**、要求源目标逐位同尺寸，于是这一档**返回 `None`** ⇒ 调用方退回
`SrcOver`（玻璃还在，只是这次不折射）。

这是有意的保守：宁可少个效果，也不要录进一条非法命令 —— 非法命令的代价是
**整个应用冻死**，不是画错一块玻璃。旧代码在这一档上是
`CopyResource(内容尺寸 ← 容量尺寸)`，本身就是非法调用（同样会冻死）。

往下要补的是「按区解析」：`ResolveSubresourceRegion` 在
`ID3D12GraphicsCommandList1` 上、带源 `RECT` 与目标偏移；或者把 MSAA 目标改成
内容尺寸、在帧末解析时按区落到容量尺寸的后缓冲上。两条都要先在真 GPU 上验过
`Close()` 不报错才算数。

---

## 门禁

| 测试 | 钉住什么 |
|---|---|
| `zephyra-core::tests::text_pipeline::changing_text_content_reshapes_on_the_next_render` | 改 `TEXT` **不重排**也要重新成形：先画 `"48"`（2 个 quad），只改文字再画 `"4"`，断言 **1 个** quad。旧代码会吐 2 个陈旧 quad |
| `zephyra-winit-d3d12::renderer::tests::dst_copy_must_be_a_single_sample_render_target` | 副本的三条创建期不变量：单采样、带 `ALLOW_RENDER_TARGET`、格式与源一致 |
| `zephyra-winit-d3d12::renderer::tests::resolve_destination_must_be_resolve_dest_not_render_target` | 解析目标的状态是 `RESOLVE_DEST` 而**不是** `RENDER_TARGET` —— 防止有人照着 `swapchain.rs` 那条后缓冲路「简化」回去 |
| `zephyra-winit-d3d12::swapchain::tests::msaa_sample_count_is_four` 等 | MSAA 目标与采样数的一致性（已有，未改） |

**诚实记账**：后两条是**创建期不变量 + 常量锚**，不是行为测试 —— 「录进非法命令、
`Close()` 才炸」这一类只能端到端才看得见，而本仓的真 GPU 探针
（`tests/d3d12_draw.rs` 等 21 条）都在 `#[ignore]` 里、且**没有覆盖折射的
backdrop 拷贝**。所以缺陷二这一轮的**行为**判据是：

- 导航 state → glass，出图确认三块玻璃内只剩被折射的棋盘格（无上一页控件）；
- `ZEPHYRA_MSAA=0` 走同一条导航，同样干净；
- `Thickness` 步进器 48 → 44 → 40 **逐帧**生效（不需要 resize / 换页）。

这几条**没有**自动化门禁，是已知盲区，不假装它被钉住了。

---

## 方法论教训

**「点了没反应」先看日志里有没有回码，别先看输入。**

这一轮前大半程都在怀疑点击坐标、DPI、导航 key，全是白工 —— 回码一直摆在那里：
`Close 命令列表失败: 参数错误 (0x80070057)`。判据只要一句：**stderr 里有没有
D3D12 的错误行**。有 ⇒ 是渲染循环死了；没有 ⇒ 再往输入查。

与
[`2026-10-gpu-device-reset-dead-clicks.md`](2026-10-gpu-device-reset-dead-clicks.md)
是同一族的第二种成因：那一篇是**设备死了**（`DEVICE_RESET`），这一篇是
**设备活着但每一帧都录错了**（`Close()` 回 `E_INVALIDARG`）。两者的画面表现
一模一样 —— 冻结在最后一帧，标题栏还能拖。

第二条教训小一点但很实用：**状态机的 `StateBefore` 必须记真实状态。**
「每帧都假定资源还在初始状态」这类写法在能跑的机器上一直跑得通，
只是因为它把一次真转换谎报成了空转换；等到哪天中间多了另一条路径，它才变成错。

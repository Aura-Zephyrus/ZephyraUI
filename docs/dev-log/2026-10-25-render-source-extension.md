# 2026-10-25-render-source-extension

> 需求：做一个拓展接口，允许外部渲染代码接入 base/core。
> 对应 2026-10-25 core 结构审计盘出的第一个缺口——DisplayList 命令闭集
> （Quad/Image/Text/Path，宿主无法注入自绘内容：视频帧、GPU 图表、嵌人的
> 另一套渲染器）。

## 设计：核心只搬运，宿主解释

与 `TextureRef`（纹理句柄）同一哲学落到命令层：

```
Element(.with(RENDER_SOURCE, PropertyValue::custom(src)))
  → render_rec 发 Command::Custom { bounds, source, filter, blend }
  → plan_segments 切出 Segment::Custom { index }（保序，见下）
  → 后端查钩子注册表（按 RenderSource::kind）→ 调用宿主代码
```

三个契约决定：

1. **`RenderSource { kind: Cow<'static, str>, payload: Arc<dyn Any + Send + Sync> }`**。
   `kind` 是跨 crate 边界的身份（图标名/纹理名同一惯例）；载荷 `Any` 让宿主
   拿回自己的类型；`Arc` 让 `PartialEq` 用**指针相等**判「没变」——diff
   不为此抖动，稳态零重建的前提是宿主跨帧共享同一 `Arc`。
2. **保序**：`Command::Custom` 单独成 `Segment::Custom`（fx `plan_segments`），
   钩子在被调用的位置执行，不提前不拖后。段数不变量更新为
   `2×滤镜 + 2×自绘源 + 1`（压力测试同步扩展，随机列表生成器加了 Custom）。
3. **降级不崩**：未注册钩子 → 跳过 + `unhandled_custom` 计数（"这块区域
   没画"，与查不到纹理同款）；注册并执行 → `custom_passes` 正面计数
   （与 `offscreen_passes` 同理，正面/负面两个计数才分得清「没人用」与「全被跳」）。

### v1 边界（刻意不做）

- **离屏不支持自绘**：带非恒等滤镜的 Custom 照常进离屏段，但几何展开层
  画不出宿主内容（计数 `unhandled_custom`）。钩子自理外观，v1 无离屏。
- **混合状态自理**：进钩子时混合状态是上一段遗留值，钩子要什么自己设
  （D3D12 本来就烤在 PSO 里）；下一段 `draw_range` 的「每段自给自足」
  结构保证钩子污染的状态不会泄漏。
- **CPU 参考路径只计数**：fx `expand` 的 Custom 臂只 +1。
- **Vulkan v1 不支持**：记录/提交两段式，钩子需要提交段的活命令缓冲，
  要给 SegmentPlan 加回调型段——归入真机验证轨道（本机无 WSL）之后再做，
  先诚实计数。

## 实现

| 层 | 内容 |
|---|---|
| core | `paint::RenderSource` + `Command::Custom` + 四个命令助手（content_rect / filter / corner_radius / blend）+ `props::RENDER_SOURCE` + `Node::render_source` + `render_rec` 发射（节点自身外观之后、子节点之前） |
| fx | `Segment::Custom` + `plan_segments` 路由 + `expand` 计数臂 + `RenderReport::{unhandled_custom, custom_passes}` |
| D3D11 | `RenderHook` trait + `HookCtx`（dc/device/rtv/size）+ `Renderer::register_hook` + 段回放臂（查表→调用/计数） |
| D3D12 | 同构（HookCtx 携带记录中的命令列表 + RTV 句柄）；与 D3D11 的 trait 刻意不抽公共 crate——两边上下文毫无共性，统一只会造类型擦除层 |
| Vulkan | 计数 + 文档注明 v1 边界 |

`RENDER_SOURCE` 键默认值是空串占位而非 `Custom`：属性键默认值必须 const
可构造，而 `Custom` 要分配；读侧把非 `Custom` 的值一律读成「无」，占位
没有语义。

## 验收

- core 341（+4：命令搬运 / 零发射 / 判等语义 / 助手语义）；
- fx 125（压力测试扩展：随机 Custom 命令、段数不变量含自绘源段、
  分段回放守恒律覆盖新段型、absorb 幺元新字段）；
- **D3D11 端到端（验收测试）**：注册钩子用 `ClearView` 涂红 → 读回像素
  **恰好等于 bounds 面积**（32×64 个纯红，ClearView 无 AA 可以钉到像素）；
  未注册 → 全黑 + `unhandled_custom == 1`。这条把整条链钉死：
  DisplayList → 分段 → 查表 → 钩子执行 → 像素落位。
- **D3D12 端到端（同日补齐，`8be7344`）**：与 D3D11 逐条对应。D3D12 没有
  `ClearView`，钩子改走 `ClearRenderTargetView` 的 `pRects`——同款逐像素
  确定原语（不走混合、不受 scissor/视口影响）。判定语义多一层：钩子拿到的
  是**记录中**的命令列表，它的命令必须混进同一帧随 `submit` 生效——顺序错
  （被后续清屏盖掉）会在像素判据里现形。18 过 / 3 ignore（折射真机套件）。
- 全 workspace 测试 0 失败；clippy 干净（顺手清了碰过的测试文件里
  4 条既有 lint）。

## 踩坑记录

- 枚举变体写成一半就编译（漏 `Custom {` 头）——两次低级语法错，都是
  「文档注释比代码先写完」导致的眼花。
- 静态初始化不能调 `Arc` 分配（E0015）→ 占位默认值方案。
- windows crate 的 `ClearView` 矩形参数是 `Win32::Foundation::RECT`
  （不是 `D3D11_RECT`）且只收 3 参。

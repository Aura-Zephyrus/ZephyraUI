# 布局与定位规范（layout-spec）

> 对应计划：`docs/plan/zephyra-box-layout-positioning-plan.md`（D1–D19）。  
> 本文是实现与评审依据；与计划 §0/§1 冲突时，以“先改计划再改实现”为原则。  
> 状态：D1–D19 已冻结；R2/R3/R4 核心实现与迁移已落地（2026-11-08），实测与门禁见 `docs/dev-log/2026-11-layout-refactor-box.md`。

## 1. 坐标、盒模型、模式

- 单位 DIP；窗口/根客户端区原点 `(0,0)`，X 向右、Y 向下。
- `arranged_rect`：**border box**，根绝对坐标；`layout/arrange` 的唯一几何输出。
- 内容区：`arranged_rect.inset(padding)`。
- `width/height`：border box；`border-width` 当前仅绘制，不参与布局（R1 冻结）。
- `anchor`：border box 9 点，`(ax,ay)∈{0,0.5,1}`；默认 `top-left`。
- `position: flow|parent|root`（默认 `flow`）。`static/absolute/fixed` 报错并给迁移映射（`static→flow`，`absolute→root + anchor: center`，`fixed→root`）。
- `parent` 参照父内容区左上角；`root` 参照根/窗口客户端区 `(0,0)`。
- `left/top` 是**锚点坐标**（参考系内 anchor 点的位置），不是左上角偏移（除非 `anchor: top-left`）。
- 缺轴：参照系原点；**取消 static-position 回落**；`anchor` 仅作用于显式坐标轴（D6/D9）。

## 2. 三套几何（不得混用）

| 名称 | 含义 | 用途 |
|---|---|---|
| 布局矩形 | `arranged_rect`（border box），不含 translate/scale | layout、`Node::rect`/`find_rect`、`center()`（布局中心） |
| 视觉矩形 `visual_rect` | 布局矩形经祖先 translate + 本节点 scale 后的**实际绘制边界**，并集文字排版矩形；不含滤镜/装饰外扩 | paint 合成参考、hit、`HitNode.rect` |
| 脏区 `damage_bounds` | `visual_rect` ∪ 滤镜/装饰外扩 | `FrameCache` damage/脏区 |

`visual_rect(root, key, ctx)` 为自由函数（root-only 语义，避免任意子树误用）。

## 3. VisualState / root 边界

遍历状态仅维护累计 `translate: Point`（`opacity` 单独沿祖先链相乘，保持现有 paint 语义；`filter/blend` 不继承）。`collapsed` 跳过。

```text
enter(node, inherited_translate):
  base = if node.position == Root { ZERO } else { inherited_translate }
  own  = base + node.translate
  visual = actual_visual_bounds(node, own, node.scale)  // union text bounds
  recurse children with own
```

`scale` self-only（不沿父链）；`translate` 沿父链累加；`root` 清零祖先累计 translate，自身 translate 仍生效。

## 4. 文字视觉 bounds（R1 阻断，已冻结）

唯一来源与 fallback（paint/frame/hit 共用 `Node::text_visual_bounds(...)`）：

1. 若节点 glyph cache 有效（文本/样式/max_width 指纹 + atlas revision 匹配），取 glyph dst 并集 + offset（局部坐标），转换到窗口视觉空间时与 `visual_rect` 同规则。
2. 否则若 `ctx.engine` 可用，调用 `shape()` 得到文本块尺寸/行框，按当前 `text_max_width`/样式生成保守轴对齐 bounds（不要求 glyph 级精度），缓存为 `text_bounds` 并用同一失效指纹。
3. 否则（无 engine、paint 前且无缓存）回退为 `Rect::ZERO` 对文字贡献（不扩大），但**不得**让已有布局 rect 丢失；实现需避免“可见文字点不到/脏区漏刷”的已知路径，R2 补针对性测试（含 engine 可用场景）。

实现注意：`glyph_quads` 当前仅 paint 填充；`shape()` 结果不进 `MeasureResult`。R2 可引入 `text_bounds`/复用 glyph cache 指纹，禁止 hit 直接假设 paint 已运行。

## 5. 有效层 `effective_layer`（R1 阻断，已冻结）

- 语义：弹层子树的排序/命中层 = 最近祖先显式 `layer` 或本节点 `layer` 的**有效层**。只在弹层根写 `layer: popup` 时，子节点必须与根同属 `popup`（paint 排序 + hit 候选排序一致）。
- 实现选型：**paint/hit 共用遍历时传递的 `effective_layer` 上下文**（最近祖先有效层）。不改为 `inheritable_layout`；若后续调整 `LAYER` 属性继承标志，必须保持 `affects_layout=false` 且与 paint/hit 同源。当前 property 继承未全局接线（`resolve(..., None)`），故不依赖属性继承。
- `SortKey.layer` / hit 候选层均使用 `effective_layer`。

## 6. 命中（D13/D14）

- 候选：遍历全部 `!collapsed` 节点，`visual_rect.contains(point)`（视觉矩形，不含滤镜外扩）的节点进入候选。
- 选择：`(effective_layer, z, preorder)` 最大者。`z` v1a/v1b 恒 0；v1c 映射 `SortKey.z`。
- `preorder`：**节点级** paint rank（文档序先序：节点自身在子节点之前），**不复用** `SortKey.order`（命令级）。R2 实现可在单次遍历中计数或预计算；O(n) 最坏路径需有 bench 记录。
- `path`：`root → 最上层命中节点` 的完整祖先链；`HitNode.inside` 表示该节点自身视觉矩形是否包含点。
- `target()`：路径中最后一个 `interactive && inside`。
- 删除 `HitTestRequest.ignore_above`（R1/R2 迁移）。
- `HitNode.rect` = 视觉矩形；`Node::rect/find_rect` 仍是布局矩形。

## 7. measure / LayoutEnv（D15）

```text
pub struct LayoutEnv { pub viewport: Size }

layout(node, bounds, ctx):
  env = LayoutEnv { viewport: bounds.size() }
  measure(node, constraints(&bounds), &env, ctx)
  arrange(node, bounds, ctx)
```

- measure 缓存键包含 `env.viewport`（至少）。
- 手动项按轴约束：显式尺寸轴 `max=INF`（v1b 再夹 min/max）；auto 轴 `max = viewport`（`root`）或父内容区（`parent`）；`min=0`。
- `parent/root` 项自身测量但不计入父 flow 尺寸、不推进游标。
- 根节点自身 `position/left/top/anchor` 忽略/诊断；根铺满 `bounds`。
- 迁移策略：**直接改公开签名并全仓同步**，避免两套 viewport 语义。

## 8. 诊断（D10）与 zstyle/zypui（R3）

- `flow + 显式手动属性`：同规则块 error/warning 按 D10 矩阵；显式判定用 `PropertyStore::lookup`。运行时只 debug 一次。
- 裸标签小写 `box/text/image/control`；组件 PascalCase；`<container>`/未知裸标签报错并给迁移提示。
- 布局属性 `.zstyle`-only。

## 9. 快速参考（R4）

### 9.1 居中首选布局，不要首选手动定位

**文档首推**：优先用 `main-align` / `cross-align` / `justify-content` / `align-items` 等布局能力把内容居中或对齐。手动 `position` 只用于明确的脱流叠放（浮层、角标、相对父/根的固定几何），不要用 `left/top + anchor` 代替普通流式居中。

### 9.2 `position` 模式对照

| 模式 | 参与父布局 | 参照系 | `left/top` 含义 | 祖先 `translate` | 典型用途 |
|---|---:|---|---|---|---|
| `flow`（默认） | 是（Stack/Flex） | 由父布局决定 | **不应显式设置**；同规则块显式设置按 D10 诊断 | 正常累计 | 普通文档流子项 |
| `parent` | 否（脱流） | 直接父**内容区**左上角 | 锚点在父内容区参考系中的坐标 | 正常累计 | 角标、相对容器定位 |
| `root` | 否（脱流） | 窗口/根客户端区 `(0,0)` | 锚点在根参考系中的坐标 | 进入时清零祖先累计；自身 `translate` 仍生效 | 全局浮层、相对窗口定位 |

旧关键字（编译期报错并给迁移提示）：

| 旧写法 | 迁移 |
|---|---|
| `position: static` | `position: flow` |
| `position: absolute` | `position: root; anchor: center`（按旧“窗口+中心”语义） |
| `position: fixed` | `position: root` |

v1b 计划项（**尚未实现**）：`right/bottom`、`min/max/margin`、拉伸、`pin`（如 `pin: center` / `pin: fill` / 四边+margins）。不要在 v1a 代码/样式中假设这些已生效。

### 9.3 盒模型与 `anchor` 参考系（ASCII）

```text
父节点 (position: parent 的参照 = 父 content box 左上角)
┌──────────────────────────────────────────────┐  ← parent border box
│ padding                                      │
│  ┌────────────────────────────────────────┐  │  ← parent content box (origin for parent)
│  │  (Px,Py) = content origin             │  │
│  │                                        │  │
│  │     child arranged_rect = border box   │  │
│  │     ┌───────────────┐                  │  │
│  │     │ TL  TC   TR   │  ← anchor 点在 border box 上
│  │     │ CL   C   CR   │     (ax,ay)∈{0,.5,1}
│  │     │ BL  BC   BR   │
│  │     └───────────────┘                  │  │
│  └────────────────────────────────────────┘  │
└──────────────────────────────────────────────┘

求解（某轴有显式坐标时）: rect_origin = P - a * size
  P   = 参考系中的锚点坐标（来自 left/top）
  a   = anchor 分量（top-left => 0, center => 0.5, bottom-right => 1）
缺轴: 放在参考系原点，且该轴不应用 anchor 分量（D6/D9）
```

### 9.4 诊断示例（zstylec）

`position: static|absolute|fixed` 会报错并给出可直接替换的建议；`flow + 显式手动属性` 按 D10 分级。示例（实际文案以当前 `zstylec` 为准）：

```text
app.zstyle:N:1: error: position: absolute is no longer supported.
  position: absolute;
  ^^^^^^^^^^^^^^^^^^
  help: use `position: root; anchor: center;` (maps old window+center behavior)
```

```text
app.zstyle:N:1: error: position: static is no longer supported.
  help: use `position: flow;`
```

## 10. R2/R3 实施顺序（历史记录）

1. core：`LayoutEnv` + measure 缓存键 + `Position` 语义/数据结构准备（保持迁移可编译）。
2. core：`visual_rect`/`damage_bounds`/`VisualState` helper，paint/frame/hit 共用；文字 bounds fallback。
3. core：hit 重写（候选全局、`effective_layer`、节点级 preorder、完整 path+inside、删 `ignore_above`）。
4. core/props/zstylec：`position flow|parent|root`、`anchor`、`left/top` 锚点坐标、`layer` 解析（v1a 若弹层需要）。
5. zypui/zypuic/仓内：小写词表与 `<box>` 迁移（单独提交）。
6. UIA/gallery/tests/docs：视觉矩形、旧契约改写、dev-log 结论+门禁。
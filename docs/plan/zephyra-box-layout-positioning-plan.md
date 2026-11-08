# Zephyra 布局引擎全量重构 + `<box>`/定位规范（主计划）

> **状态**：Plan 模式产出（仅计划）。开始改代码前请切换出 Plan 模式。  
> **阶段前提**：框架仍在开发期，暂无外部应用 → **clean-break**（不做外部兼容/长期 deprecation；仅仓内迁移与门禁）。  
> **归属**：`zephyra-core` 布局子系统为核心；向上联动 `zypui`/`zypuic`、`zstylec`、`.zstyle`/`.zypui`、`gallery`/`ZephyraUI`/示例/测试。`<box>` 原语属于 zypui 体系，不放 `ZephyraUI` 控件层。  
> **动机**：现有布局/定位实现混乱、bug 多；通过**架构级重构**在统一规范下重建 `measure/arrange/缓存/节点模型` 与定位语义。  
> **边界**：遵守 `AGENTS.md`（`FrameCache` 仍是帧内容唯一决策点；core/fx `forbid(unsafe_code)`；外观不进源码；属性改动联动 `props.rs` + `zstylec` 映射；`--workspace` 门禁；后端分离，本特性不改 shader/宿主语义）。

> **文档分层**：§0 / §1 是**规范层**（R1 要冻结成可照着实现的规格）；§2 是**路线图层**（R0–R4 任务）；§3 是属性检查表；§4 只保留未冻结项。后续修改规范只改 §0/§1，任务进度只改 §2，避免两处措辞漂移。

## 0. 已确认决策（D1–D19 已冻结）

- **全量重构布局引擎架构**（`measure/arrange/缓存/节点模型`），不是仅补丁 box。
- QML 对齐心智：父子坐标系、布局接管 vs 手动几何、边缘锚点明确；不引入 CSS “absolute 向上查找 positioned 祖先”的隐式组合。
- **D1 定位模式**：`position: flow | parent | root`（默认 `flow`）。
  - `flow`：参与父布局（Stack/Flex），位置由父布局决定；显式设置 `left/top/right/bottom/pin` 应诊断。
  - `parent`：脱流，手动定位，参照**直接父内容区**左上角。
  - `root`：脱流，手动定位，参照**根/窗口客户端区**左上角。
  - `static` / `absolute` / `fixed` **全部报错，不设别名**；诊断给迁移映射。
- **D2 视觉变换 = 布局惰性**：`translate` 不改变 `arranged_rect`/measure/arrange，但影响 **paint + hit + damage**；`scale` 保持 self-only，也影响 paint/hit/damage。**“视觉矩形”取实际绘制边界**，不是简单地把 border box 乘 scale；文字不缩放时见 D17。
- **D3 命名分域**：`anchor` 只表示自身 9 点对齐点；v1b 的约束能力统一用 `pin`，禁止 `anchors.*`。
- **D4 能力分期**：
  - **v1a = 手动摆放**：核心引擎重构 + `<box>` + 小写词表 + `position: flow|parent|root` + `left/top` + `anchor` + 视觉/命中契约。
  - **v1b = 手动约束求解**（唯一阶段）：`right/bottom`、`min/max`、`margin`、拉伸、`pin`。
  - **v1c（可选，非布局）**：`z`/绘制顺序、根 overlay/portal 等合成能力；不再产生第二套布局规则表。
- **D5 词表**：裸元素与类型选择器**统一小写**：`box` / `text` / `image` / `control`；`.zstyle` 写 `box.window`，`.zypui` 写 `<box>`；组件名仍 PascalCase。一次性移除 `<container>` 与 `Container.` 等旧大写类型名，不保留别名。
  - **未知裸标签必须报错**（例如旧 `<container>`、拼错的 `<boxx>`），不能落入 `NodeTag::Control` 兜底。
  - 内部 `NodeTag::Container` 在 R3 单独提交改名 `NodeTag::Box`；不再保留“内部名字可暂留”的长期例外。
- **D6 原点回落**：手动模式未给坐标的轴 = 参照系原点（`parent` → 父内容区原点，`root` → `(0,0)`）；**取消 static-position 回落**；Stack/Flex 不再有两套脱流回落路径；`anchor` 只作用于有显式坐标的轴。
- **D7 中心**：`Node::center()` = `arranged_rect` 的几何中心；删除无调用的 `Node::find_center()`；清掉“left/top 指中心”的旧文档。
- **D8 布局几何 vs 视觉几何**：
  - `Node::rect()` / `find_rect()` = **布局矩形**（窗口绝对坐标），不随 translate/scale 改变；
  - `HitNode.rect` = **实际视觉矩形**（布局矩形 + 祖先 translate + 本节点 scale；不包含滤镜/装饰外扩）；
  - `find_visual_rect()` 只允许在 **layout root** 上调用，或做成 `visual_rect(root, key)` 自由函数；不能对任意子树调用后假装拿到了窗口视觉位置；
  - `center()` 仍是布局矩形中心；视觉中心另开 API，若需要；
  - UIA 的屏幕矩形使用视觉矩形，不使用 `arranged_rect`。
- **D9 anchor 按轴生效**：
  - 参照系原点是左上角；`left/top` 是**锚点坐标**（默认 `anchor: top-left` 时才等于左上角偏移）；中心语义只能由 `anchor` 显式表达。
  - X 轴有显式 `left`（v1b 后含 `right`/`pin`）才使用 `anchor.ax`；Y 轴同理；缺轴回落参照原点并忽略该轴 anchor 分量。
  - `anchor: center` 单独出现且两轴都没有显式坐标 ⇒ no-op（v1a 给 warning）。
  - v1b：`right/bottom` 是**末端边缘约束**，该轴忽略 anchor；`left+right` / `top+bottom` 拉伸时忽略该轴 anchor；与 anchor 冲突给诊断。
  - 显式尺寸优先于 anchor 的摆放分量；anchor **不参与 measure**（方案 A）。
- **D10 flow 诊断边界（矩阵）**：

  | 情况 | 级别 | 处理 |
  |---|---|---|
  | 同一规则块 `position: flow` + 显式 `left/top/right/bottom/pin` | error | 这是明确的手动定位误用 |
  | 同一规则块 `position: flow` + 显式非默认 `anchor` | warning | no-op，提示需要 `parent/root` |
  | 只写手动属性、同块未写 `position` | warning | 可能由另一条规则提供 position |
  | 跨规则级联成 `flow + 手动属性` | 运行时忽略 + debug 一次 | 编译期无法可靠判断 |
  | `anchor: center` 单独出现、两轴都无坐标 | warning | no-op |

  - “是否显式设置”一律用 `PropertyStore::lookup`（Local/Styled）判断，不能用 `value()` 的默认值；`is_locally_set` 只看 local，不够。
  - 运行时提示只在样式应用/变更时发一次，不能每帧 `layout` 打日志。
- **D11 root 视觉边界与测量**：
  - `arranged_rect` 始终是根绝对坐标；`parent` 的 `left/top` 用父内容区原点，`root` 用 `(0,0)`。
  - 进入 `position: root` 节点时清零**祖先累计 translate**；自身 translate 仍生效；`opacity` 仍沿祖先链相乘；滤镜/混合模式不继承；不自动脱离裁剪/合成组。
  - 视觉遍历只维护 `VisualState { translate }`，不需要额外的“坐标空间已重锚”布尔量；`collapsed` 节点直接跳过。
  - 新增布局环境 `LayoutEnv { viewport: Size }`（精确形态见 D15）：`root` 项用视口约束测量，`parent` 项用父内容约束；根节点自身的 `position` 忽略/诊断。
  - 根节点自身（`layout::layout` 的 `bounds`）永远铺满窗口；根节点上的 `left/top/anchor` 不生效。
- **D12 小写词表**：见 D5；`zstylec` 的 `TAG_NAMES`、`zypuic` 的裸标签白名单、诊断候选、测试夹具全部同步；`is_pascal()` 从此只用于组件名。旧大写类型名给“改成小写”的迁移提示，不静默匹配。
- **D13 命中顺序**：
  - hit 按 **paint 顺序的节点级等价序** `(effective_layer, z, preorder)` 取最上层；不是“子节点逆序 + 首个命中”。
  - `effective_layer` 是节点的**有效合成层**：D18 要求 paint/hit 使用同一来源；R1 必须冻结“只读本节点 vs 最近祖先有效层/继承”的语义（见下）。
  - `preorder` 是**节点级** paint rank（与 `paint` 的“节点自身命令先于子节点”一致），**不是** `SortKey.order`（`SortKey.order` 是命令级序号；单节点可发多条命令，不能直接复用）。
  - `z` 在 v1a/v1b 恒为 0；v1c 的 `z` 直接映射 `paint::SortKey.z`（i32），不另造栈上下文；接入时同步改 `props` 与 paint 的 5 处 push。
  - `HitTestRequest.ignore_above` 当前无任何调用方读取；**R1 直接删除**，v1c 做 portal 时再按明确语义加回。
- **D14 命中路径与目标**：
  - `HitResult.path` = **root → 最上层命中节点的完整祖先链**，包含矩形不包含点的祖先；`HitNode.inside` 记录该节点自己的视觉矩形是否包含点。
  - `target()` = 路径中最后一个 `interactive && inside` 的节点；祖先即使可交互，只要 `inside == false` 就不能被当作点击目标（避免“点在子元素上却命中外部父容器”）。
  - 候选选择：遍历全部 `!collapsed` 节点，取 `visual_rect.contains(point)` 的候选，按 `(effective_layer, z, preorder)` 取最大；不提前返回。实现形态由 R1 冻结（推荐单次遍历更新最佳候选并保留其祖先路径，保持零额外分配可控）。
  - 路径深度仍受 `MAX_DEPTH` 限制；截断标志保留。
- **D15 LayoutEnv 与测量约束**：
  - 精确形态：

    ```text
    pub struct LayoutEnv { pub viewport: Size }

    layout(node, bounds, ctx):
        env = LayoutEnv { viewport: bounds.size() }
        measure(node, constraints, env, ctx)
        arrange(node, bounds, ctx)
    ```

  - `measure` 的缓存键必须包含 `LayoutEnv`（至少 `viewport`）；否则固定宽度父容器里的 `root` 子项在窗口 resize 后可能不重测。
  - 手动项按轴测量：
    - 显式 `width/height` 的轴：`max = INF`，允许超出父/视口（`min/max` 到 v1b 再夹逼）；
    - `auto` 的轴：`max = viewport`（`root`）或父内容区尺寸（`parent`）；
    - `min = 0`。
  - `text-max-width` 仍是文本换行的唯一显式入口；v1a 不引入百分比。
  - `parent/root` 项自身仍要测量（arrange 需要缓存尺寸），但不计入父 flow 尺寸，不推进游标。
  - 根节点自身的 `position`、`left/top/anchor` 忽略并给诊断；根节点永远铺满 `bounds`。
  - 公开 `measure` 的签名是否保留兼容包装，R1 决定；倾向直接改成带 `LayoutEnv` 的版本，仓内测试一起迁移，避免出现两套 viewport 语义。
- **D16 盒模型**：

  | 项 | 定义 |
  |---|---|
  | `arranged_rect` | border box |
  | 内容区 | `arranged_rect.inset(padding)` |
  | border width | 只影响绘制，不参与当前布局 |
  | `width/height` | border box 尺寸 |
  | `anchor` 锚点 | border box 上的 9 点 |
  | `position: parent` 原点 | 父节点内容区左上角 |
  | `position: root` 原点 | 窗口客户端区左上角 `(0,0)` |
  | v1b `margin` | border box 外侧 |

  - `anchor` 的 border box 与 `parent` 的 content box 是两件事；文档必须同时写清，避免 1px 边框导致的“看起来偏了”。
- **D17 文字与 scale**：
  - `paint::text_command` 当前只接收 `offset`，不接收 `NodeDraw`；文字**不随 `scale-x/y` 缩放**。
  - v1a 的 `visual_rect` 取实际绘制边界：`transform(border box)` **并集** 文字排版矩形（字形四边形并集 + offset）；`damage_bounds` 再并集滤镜/装饰外扩。命中与脏区都不能假设“文字一定在缩放盒内”。
  - **R1 阻断（实现前必须冻结）**：文字 bounds 的唯一来源、失效时机、以及 `ctx.engine` 不可用 / paint 前命中 / 文本变更但未重排时的 fallback。`glyph_quads` 当前只在绘制趟填充、`measure` 的 `shape()` 结果不进 `MeasureResult`，不能把“读 glyph cache”当默认成立的前提；R1 需定义 paint/frame/hit 共用的取数路径，并补测试（文字超出缩放 border box 仍可命中、仍触发 damage）。
  - v1c 若要让文字真正随 scale 缩放，再改 `Command::Text` + `zephyra-fx` + 三后端；不混进 v1a 的布局重构。
  - `scale` 仍 self-only：父节点 scale 不影响子节点（当前 `paint` 语义）。
- **D18 `layer` 与 `z`**：
  - `LAYER` 是节点级合成层：`background | content | overlay | popup`，默认 `content`；`affects_layout = false`（排序属性，不能误改成 `inheritable_layout`）。
  - **R1 阻断（实现前必须冻结）**：弹层子树的**有效层**语义。当前 `LAYER` 不可继承且 paint/hit 各自按节点读取，若只在弹层根写 `layer: popup`，子节点会掉回 `content`，导致子命令/子命中层排序错误。R1 必须二选一并写死：① 让 `LAYER` 具备“有效层”继承语义（若改属性构造器，必须保持 `affects_layout = false`，不能用 `inheritable_layout`）；② paint/hit 共用“最近祖先有效层”上下文。无论哪种，paint 与 hit 必须使用同一来源。
  - `.zstyle` 必须能解析 `layer: popup`（当前 `props_map` 有映射但 `zstylec::value::enum_value` 没有 `Layer` 分支）；v1a 的弹层迁移若需要跨层覆盖，就把解析放进 v1a/R3，不要留到 v1c。
  - `z` 是 v1c 的层内序，映射 `SortKey.z`（i32，默认 0）；`layer` 与 `z` 不是同一个东西。
  - `position: root` 只改坐标参照，不改变树序/层序；跨层弹层显式 `layer: popup`（或 v1c `z`）。
- **D19 `.zypui` 与 `.zstyle` 的分工**：
  - 布局属性是 `.zstyle`-only；`.zypui` 不提供内联 `position/left/anchor/right/min/max/margin/pin`。
  - `zypui::ir::BARE_ATTRS` 只保留内容/动态外观（`translate/scale` 等）与 `fill:` 键的 `width/height`。
  - §3 表里这些布局项的 `zypui attr` 一律写 `—`；若未来要做内联布局，另立决策并补 instantiate/BARE_ATTRS/文档。
- clean-break（仅仓内）：旧 `absolute` 的“窗口+中心”语义直接废止；迁移映射固定为 `static → flow`、`absolute → root + anchor: center`、`fixed → root`。`dd-morph`/弹层等几何重算并 `--dump-cpu` 对照。

> 仓内实测（2026-11，`git grep`）：`.zstyle` 中**没有**实际 `position/left/top` 规则（只有注释与测试），因此模式/回落 clean-break 的迁移成本极低。  
> 小写迁移实测：`.zstyle` 里 `Container.` 278 行/18 文件、`Text.` 101 行/19 文件、`Image.` 2 行/1 文件、`Control.` 174 行/15 文件（类型选择器合计约 552 行/20 文件）；`.zypui` 里 `<container>` 178 行/3 文件（`<text>/<image>/<control>` 本来就是小写）；Rust 内联样式/测试/文档中的类型选择器约 53 行/15 文件。另需同步 `zypuic` 的 `tests.rs`/`fmt_tests.rs`/`style_gate_tests.rs` 中的 `<container>` 字面量。

## 1. 目标架构与规范（R1 冻结，作为实现真相）

### 1.1 坐标、模式、锚点

| 项 | 规则 |
|---|---|
| 单位/原点 | DIP；参照系左上角 `(0,0)` |
| `position: flow`（默认） | 参与父 Stack/Flex；位置由父布局决定；显式手动定位属性按 D10 诊断 |
| `position: parent` | 脱流；参考原点是**直接父内容区**左上角；`left/top` 是该参考系中**锚点的坐标**；父动子动；祖先 `translate` 正常累加 |
| `position: root` | 脱流；参考原点是**根/窗口客户端区**左上角；`left/top` 是该参考系中**锚点的坐标**；进入该节点时清零祖先累计 `translate`；自身 `translate` 仍生效；`opacity` 仍相乘，滤镜/混合模式不继承；不自动脱离裁剪/合成组 |
| `anchor` | 属性名 `anchor`；默认 `top-left`；9 点 `top-left,top-center,top-right,center-left,center,center-right,bottom-left,bottom-center,bottom-right`；border box；`(ax,ay) ∈ {0,0.5,1}`；**只作用于有显式坐标的轴** |
| 禁止 | 把 `left/top` 本身解释成中心点；中心必须由 `anchor` 显式表达；`static/absolute/fixed` 关键字 |

**`(ax,ay)` 冻结**：`TL(0,0) TC(0.5,0) TR(1,0) CL(0,0.5) C(0.5,0.5) CR(1,0.5) BL(0,1) BC(0.5,1) BR(1,1)`。

**坐标轴独立**：

- 某轴有显式坐标 ⇒ `rect = P - a*size`，其中 `P` 是参考系里的**锚点坐标**；
- 某轴无显式坐标 ⇒ 该轴放在参考系原点，**不启用该轴 anchor**（不会产生 `-w/2` 的半截越界）；
- 示例：`position: parent; left: 100; anchor: center` ⇒ x 以中心对齐 100，y 在父内容区顶部；
- 示例：`position: parent; top: 50; anchor: center` ⇒ y 以中心对齐 50，x 在父内容区左侧；
- 示例：`position: parent; anchor: center`（无 left/top）⇒ 完全 no-op，并按 D10 给 warning。

**`left/top` 的读法**：不是“元素左边缘的偏移”，而是“元素 anchor 点在参考系里的坐标”。默认 `anchor: top-left` 时才等价于左上角偏移。这是本规范最容易误读的一条，文档必须配图。

### 1.2 求解阶段（方案 A，确定性）

1. **Measure**：得最终尺寸 `w,h`（显式 → 布局/约束 → 内容 → min/max → v1b 拉伸，按优先级表）。**`anchor` 不参与 measure**。
2. **Arrange/Resolve**：用最终 `w,h` + `anchor` + 参照点 `P` 计算 `rect`；递归 arrange 子树。`parent/root` 项自身 measure，不计入父 flow 尺寸。
3. **Root 参照**：`root` 项的 `P = (0,0) + left/top`；v1b 的 `right/bottom` 使用 `LayoutEnv.viewport` 的宽高。根节点自身（`layout::layout` 的 `bounds`）永远铺满，`position/left/top/anchor` 忽略/诊断。
4. **手动项测量**：按 D15 的按轴约束；显式尺寸不被视口/父内容区夹逼，`auto` 轴才受参照尺寸限制。
5. 未定义组合**默认诊断报错**（规则表冻结前不静默）。

### 1.3 回落、布局接管、视觉变换与命中契约

- **原点回落（D6）**：
  - `parent`：未给坐标的轴 = 父内容区原点；
  - `root`：未给坐标的轴 = 根/窗口 `(0,0)`；
  - 不再读取“正常流里本会到的位置”；Stack/Flex 的脱流项共用同一规则。
- **布局接管**：
  - `flow` 子项由父 Stack/Flex 管理；`parent/root` 子项不参与父 flow、不推进游标、不计入父内容尺寸；
  - `collapsed` 继续不占位、不绘制、不命中；
  - `parent/root` 项在父 measure 里仍要测量（arrange 需要缓存尺寸），只是不计数；
  - `flow` 子项上显式设置手动定位属性按 D10 诊断；未定义组合默认诊断。
- **视觉变换（D2/D11/D16/D17）**：
  - `arranged_rect` 始终是根绝对坐标；`translate`、`scale` 都不改变它；
  - 视觉遍历状态只有累计 `translate`；`opacity` 单独沿祖先链相乘：

    ```text
    enter(node, inherited_translate):
        if node.collapsed:
            return
        base = if node.position == Root { ZERO } else { inherited_translate }
        own  = base + node.translate
        visual_rect = actual_visual_bounds(node, own, node.scale)
        for child in children:
            enter(child, own)
    ```

  - `actual_visual_bounds` = `transform(border box)` 并集文字排版矩形（D17）；不含滤镜/装饰外扩；
  - `transform` 与现有 `paint::NodeDraw` 同一公式：绕**未平移**布局盒中心缩放，再加 `own` 偏移；
  - `translate` 沿祖先链累加；`scale-x/y` 保持 self-only（不沿父链）；`root` 节点是 translate 边界；
  - **`visual_rect` 与 `damage_bounds` 分开**：paint/hit 用 `visual_rect`；frame 脏区用 `damage_bounds = visual_rect + filter/decor outsets`。hit 不能把投影/模糊外扩当成命中区域；
  - `paint`、`frame`、`hit` 三处共用同一 transform/actual bounds 实现；`HitNode.rect` 返回 `visual_rect`。
- **命中顺序与路径（D13/D14）**：
  - 候选 = 全部 `!collapsed` 且 `visual_rect` 包含点的节点；
  - 取 `(layer, z, preorder)` 最大的候选；不提前返回，不能只做“子节点逆序”；
  - `path = root → 候选` 的完整祖先链；`HitNode.inside` 标记各节点自身是否包含点；
  - `target()` 只从 `interactive && inside` 中取最深者；
  - `HitNode.rect` 是视觉矩形；`Node::rect()`/`find_rect()` 仍是布局矩形；
  - `ignore_above` 在 R1 删除；不得保留未读字段；
  - 性能：首次分配后零分配复用 `HitResult`；O(n) 最坏情况要进 bench，若超出预算再缓存节点级 paint rank，不能退回错误的提前返回。
- **冲突**：v1b `left+right`/显式 `width`/非兼容 `anchor`、v1b `pin` 与手动坐标冲突，按下表处理；表格定稿前默认 error。

### 1.4 布局能力分期

- **v1a**：核心引擎重构 + `<box>` + 小写词表 + `position: flow|parent|root` + `left/top` + `anchor` + `translate/scale` 命中契约 + 视觉/命中契约（D8/D11/D13/D14/D15/D16/D17）+ 核心迁移（dropdown/morph/弹层）。
- **v1b**：**唯一的手动约束求解阶段**：`right/bottom`、`min/max`、`margin`、`left+right/top+bottom` 拉伸、`pin` 子集（`center`、`fill`、四边+margins）。只发布一张优先级/冲突表。
- **v1c（可选，非布局）**：`z`/绘制顺序、根 overlay/portal、其他合成能力；不再产生第二套布局规则表。`z` 映射 `SortKey.z`；文字真正随 scale 缩放若要做也放这里。

### 1.5 不变量（护栏）

- `FrameCache` 为帧内容唯一决策点。命中/绘制/布局共享同一套几何契约。
- **布局几何**：`Node::rect()` / `find_rect()` / `center()`；**视觉几何**：`HitNode.rect` / `find_visual_rect()` / `actual_visual_bounds()`；**脏区几何**：`damage_bounds()`。三者不混用。
- 缓存与 `Dirty`/`affects_layout` 一致；`measure` 缓存键包含 `LayoutEnv.viewport`；静止态不重测；resize/热路径不回退。
- 布局 props `affects_layout` 正确；`translate`/`scale` **layout-inert**，但 `translate` 影响 paint+hit+damage，`scale` 影响自身 paint+hit+damage；文字例外按 D17。
- `position: root` 的视觉位置必须等于根坐标（祖先 translate 清零）；`opacity` 仍相乘，滤镜/混合模式不继承。
- hit 顺序 = `(layer,z,preorder)` 的节点级 paint 等价序；`path` 包含完整祖先链；`ignore_above` 不存在于 v1a API。
- 裸元素/类型选择器统一小写；组件名 PascalCase；未知裸标签报错；内部 `NodeTag::Box` 在 R3 完成改名。
- 布局属性 `.zstyle`-only；`.zypui` 不提供内联布局属性。
- `Node::center()` = `arranged_rect` 的几何中心，不等于定位锚点；不提供会把两者混用的 API。

## 2. 重构主线（R0–R4）

### R0 — 特征化与基线（**先做，不动架构**）

- [ ] 布局特征测试：Stack/Flex/spacing/align/padding/尺寸/collapsed/文本/现有 absolute/缓存命中失效/resize；补齐缺口。
- [ ] **现存缺陷特征化**：
  - Stack 与 Flex 脱流缺轴回落不一致；
  - `left/top` 中心语义 + `find_center` 旧文档；
  - `translate/scale` 命中与 paint 不一致；
  - hit 顺序与 paint 的 `(effective_layer,z,preorder)` 不一致；`ignore_above` 从未读取（定义 `hit.rs`；构造点含 `input/router.rs` 与若干测试，R0 列全量）；
  - root/视口测量约束缺失；measure 缓存键不含 viewport；
  - 文字不随 scale 缩放，但脏区按缩放盒算；文字 bounds 在 hit/paint 前可能取不到（`ctx.engine`/`glyph_quads` 时序）；
  - `layer: popup` 无法从 `.zstyle` 解析；`LAYER` 不可继承导致弹层子树有效层掉回 `content` 的风险（R1 冻结）；
  - `zypuic` 对旧 `<container>` 仍接受，且未知裸标签诊断需在小写化后改为“报错 + 迁移提示”（当前 `build.rs` 对白名单外裸名已报错，`build_bare` 的 `_` 分支是死分支）；
  - UIA 屏幕矩形使用 `arranged_rect`。
- [ ] **测试卫生**：**不要提交“断言当前错误行为”的绿色测试**。旧行为写入 dev-log；对要改变的行为，提交 `#[ignore = "R2: ..."]` 的**目标行为**测试。R2 必须删除 ignore 或删除过期测试，并在 DoD 记一条。
- [ ] **旧契约盘点**（这些会直接变红，必须列入改写清单）：
  - `zephyra-core/tests/paint_transform_tests.rs`（命中按布局盒的断言）；
  - `gallery/tests/gates.rs::hit_test_agrees_with_the_painted_rects`（用 `n.rect()` 取中心）；
  - `zephyra-core/src/props.rs`、`zypui/src/ir.rs`、`paint.rs` 中“translate/scale 不改命中”的注释；
  - `layout_classification_tests.rs` 中依赖这些注释文字的断言；
  - `zephyra-uia/rules.rs::screen_rect` 的 `arranged_rect`；
  - `zephyra-core/src/input/router.rs`、`zephyra-core/src/input_tests.rs`、`zephyra-core/tests/paint_transform_tests.rs`、`gallery/tests/gates.rs` 中对 `HitTestRequest.ignore_above`/`HitResult.path`/`target()` 旧语义的假设；
  - `zypuic` 的 `tests.rs`/`fmt_tests.rs`/`style_gate_tests.rs` 中 `<container>`/大写标签夹具。
- [ ] 视觉/几何基线：关键 gallery 路由 `--dump-cpu`；记录**实际可运行**的命令、feature、分辨率、节点数、路由 id（以 R0 实测为准，写入 dev-log）；建立“预期变化（新定位语义）vs 回归”判定。
- [ ] 依赖与痛点盘点：`layout.rs`/`node.rs`/props/`frame`/`paint`/`hit`/`style`/`context`；整理已知 bug 列表并映射到 R1/R2。
- [ ] 小写词表盘点：`.zstyle` 的 `Container/Text/Image/Control` 选择器、`.zypui` 的 `<container>`、`zstylec::TAG_NAMES`、`zypuic::build_nodes`/`build_bare`、`zypuic` 测试/fmt 夹具、Rust 内联样式与文档。
- [ ] dev-log 立项（范围、排除的假设、基线）。

### R1 — 架构与规则设计（冻结后再大改）

- [ ] **规范层产物**：一页可执行的 `layout-spec`（坐标/盒模型/root 状态表/命中算法/测量约束/诊断矩阵/v1b 优先级），作为 R2 的唯一依据。
- [ ] 新布局核心契约：树+样式+约束+上下文 → 测量/放置结果；缓存条目与失效；方案 A；`Position::{Flow, Parent, Root}`；Stack 快路径/Flex/手动+约束算法边界。
- [ ] `Node` 模型调整方案（`measure_cache/arranged_rect/dirty/glyph_quads` 等语义不回归）；删除 `find_center`，明确 `center` = 几何中心；新增 `find_visual_rect` 的 API 边界（root-only 或自由函数）。
- [ ] **几何契约**：`VisualState { translate }`、`enter()` 状态表、`actual_visual_bounds` vs `damage_bounds`、文字例外、scale self-only、root translate 边界；paint/frame/hit 共用；文字 bounds 的唯一来源/失效/fallback 写死（无 engine、paint 前、文本变更未重排时仍可给出安全结果）。
- [ ] **有效层契约（R1 阻断）**：冻结 D18 的弹层子树有效层语义；paint 与 hit 使用同一 `effective_layer` 来源；若采用属性继承，必须保持 `affects_layout = false`（不能用 `inheritable_layout`）。
- [ ] **命中契约**：`(effective_layer,z,preorder)` 候选选择、完整祖先 `path`、`inside`、`target`、`ignore_above` 删除；`preorder` 为节点级 paint rank（不复用命令级 `SortKey.order`）；`filter/decor` 不进入 hit；O(n) 性能预算与 bench 判据。
- [ ] **测量契约**：`LayoutEnv { viewport }` 的精确签名；measure 缓存键；手动项按轴约束；根节点自身 position 忽略；固定父容器 + root 子项的 resize 测试。R1 决定并记录 `measure` 公开签名迁移策略（倾向直接改签名、全仓同步迁移，避免两套 viewport 语义）。
- [ ] **盒模型契约**：D16 表；border box/content box/anchor/margin 的关系；文档配图。
- [ ] **冻结规则表**：§1.3 的原点回落、布局接管、`translate`/命中、`position: root` 边界、v1b 的 `left+right`/拉伸/`min/max/margin/pin/anchor` 优先级与冲突诊断。
- [ ] **anchor 按轴表**：9 点 × 单轴/双轴/缺轴；`anchor:center` 单独出现 = no-op（warning）；v1b `right/bottom` 末端边缘忽略 anchor，`left+right` 拉伸忽略 anchor。
- [ ] **诊断契约**：D10 矩阵；`zstylec` 同块 `flow + 手动属性` 报错/警告分级；运行时忽略 + debug 一次；`PropertyStore::lookup` 作为“显式设置”判据。
- [ ] **小写词表**：`zstylec` 的 `TAG_NAMES` 改为 `box/text/image/control`；`zypuic` 裸标签白名单改为小写；移除 `<container>` 并给迁移提示；`build_nodes` 的裸标签分支先于 `is_pascal`；未知裸标签报错（`_` 分支显式错误/`unreachable!`，不落入 `Control`）；组件名仍 PascalCase。
- [ ] **layer/属性表**：`layer` 解析与 §3 行；有效层契约按 R1 决议落地；`z` 只在 v1c；`.zypui` 布局属性 = `—`；评审 zstylec/zypui 属性与 IR 影响。
- [ ] **v1b 优先原则**（可延后到 v1b 前冻结细节）：双端约束 + 显式尺寸 = over-constrained error；`min > max` = error；`pin` 与手动坐标冲突 = error；不做静默“start 边赢”。
- [ ] zstylec/zypui 属性与 IR 影响评审：`flow|parent|root`、`anchor`、`pin`、小写选择器；为 R3 准备。

### R2 — 实现（可对拍、可回滚）

- [ ] 新引擎实现（**生产路径只有一个**）：Stack → Flex → collapsed/文本 → `flow/parent/root` + `left/top` + `anchor` + 原点回落 → 迁移用例。
- [ ] **不保留生产双引擎/运行时开关**。旧实现只放在 test-only 的 legacy 模块做差分对拍，R3 删除；避免两套 viewport/measure 语义长期并存。
- [ ] **D2/D8/D11/D13/D14/D15/D16/D17/D18 修正**：`translate/scale` 进入 hit 与 `HitNode.rect`；`actual_visual_bounds` 并集文字（含 fallback 路径）；`root` translate 边界；paint/frame/hit 三处同规则；`effective_layer` 单一来源；hit 按 `(effective_layer,z,preorder)` 取候选；`preorder` 使用节点级 paint rank；完整祖先 path + inside；`ignore_above` 删除；补本节点/祖先 translate、scale、文字、Layered Popup（含子树有效层）用例。
- [ ] 新旧几何对拍（预期差异白名单）；缓存/dirty 行为对齐特征测试；旧 absolute/static fallback 用例替换为原点回落期望。
- [ ] 性能守护：`--bench/--bench-resize`/trace/alloc-probe 对照；未用手动定位/约束时走快路径；hit 的 O(n) 与零分配复用单独记录。
- [ ] core 单测覆盖 `anchor` 9 点、尺寸耦合、按轴生效、模式、原点回落缺轴、translate/scale 命中、文字视觉并集、root 边界、hit 顺序/path/inside、`find_rect` vs `find_visual_rect`、固定父容器 + root 子项 resize。
- [ ] R2 结束前清理所有 `#[ignore = "R2: ..."]`：目标已达到的删 ignore，过期的删测试。

### R3 — 映射、迁移、切换

- [ ] **core props**（`props.rs`/`property`/`style`）：`Position::{Flow, Parent, Root}`、`ANCHOR`、`left/top` 锚点坐标语义；`affects_layout`；移除旧 `Static/Absolute` 与中心点落位；删除/重命名 `is_absolute`。
- [ ] **zstylec**：`props_map.rs`/`value.rs`/诊断全链路；解析 `flow|parent|root`，拒绝 `static/absolute/fixed` 并给迁移映射；`anchor`；`pin`（v1b）；`layer`（v1a/R3 需要时）；`TAG_NAMES` 改为小写 `box/text/image/control`。
- [ ] **zypui/zypuic**：`<box>` 唯一标签（`<container>` 报错并提示 `<box>`）；裸标签白名单小写；未知裸标签报错，不再落入 `Control`；属性白名单/BARE_ATTRS 通路审计；布局属性保持 `.zstyle`-only；IR/instantiate/eval/encode/decode **dev-release 同构**；`cargo test -p zypuic -p zypui`。
- [ ] **D5/D12 机械迁移**：
  - `.zstyle`：`Container.` 278 行/18 文件、`Text.` 101 行/19 文件、`Image.` 2 行/1 文件、`Control.` 174 行/15 文件（合计约 552 行/20 文件）→ 小写 `box/text/image/control`；
  - `.zypui`：`<container>` 178 行/3 文件 → `<box>`；`<text>/<image>/<control>` 已小写；
  - Rust 内联样式/测试断言/文档中的类型选择器约 53 行/15 文件；
  - `zypuic` 的 `tests.rs`/`fmt_tests.rs`/`style_gate_tests.rs` 中 `<container>` 字面量；
  - 最终删除旧大写关键字/别名；`NodeTag::Container → NodeTag::Box` 单独提交改名。
- [ ] **仓内迁移（clean-break）**：`git grep -n "position: absolute\|LEFT\|TOP\|find_center\|dd-morph\|Container\.\|Text\.\|Image\.\|Control\." -- *.rs *.zstyle *.zypui` 全量清理；`dd-morph-*` 重算；root 弹层/dropdown；`--dump-cpu` 对照。
- [ ] **D7 清理**：删除 `find_center`；重写 `find_rect`/`center` 文档；审计 `center()` / `find_rect()` 调用点，确保没有“把中心写回 left/top”的隐式假设。
- [ ] **UIA**：`screen_rect` 改用视觉矩形；UIA hit test 与画面/命中共用同一几何；补测试。
- [ ] **旧契约文档/测试改写**：R0 清单中的 `paint_transform_tests`、gallery gates、`props.rs`/`zypui ir.rs`/`paint.rs` 注释、`layout_classification_tests`。
- [ ] **root + layer 检查**：`position: root` 只改坐标参照，不改变树序/层序；跨层级弹层显式使用 `layer: popup`（或 v1c `z`），并在迁移清单中逐项确认。
- [ ] 切换默认引擎，删除旧窗口+中心路径/死代码。

### R4 — 验收与文档

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`（并点名 `-p zypuic -p zypui -p zstylec`；UIA 相关 crate 一起）
- [ ] `cargo doc --no-deps --workspace` 不新增 warning
- [ ] `docs/developer/` 定位/布局规范定稿；`docs/dev-log/YYYY-MM-layout-refactor-box.md`（现象/排查/**排除的假设**/结论+门禁，含实测数字+条件）。
- [ ] 诊断含修复示例；布局居中（`main-align/cross-align`）为文档首推；`position` 模式/`pin` 的对照表进文档；小写词表写进样式/UI 规范；盒模型/anchor 参考系配图。
- [ ] 全仓不再有 `#[ignore = "R2: ..."]`；旧契约测试已改写，没有“错误行为即正确”的遗留断言。

## 3. 全链路属性清单（实现检查表）

| 能力 | core prop | zstyle | zypui attr | `affects_layout` | 默认 | 阶段 |
|---|---|---|---|---:|---|---|
| 定位模式 | `POSITION` → `Flow/Parent/Root` | `position` | —（style-only） | 是 | `flow` | v1a |
| `left/top`（锚点坐标） | `LEFT/TOP` 重定义 | `left/top` | — | 是 | 缺轴=参照原点 | v1a |
| `anchor`（自身 9 点） | `ANCHOR` | `anchor` | — | 是 | `top-left` | v1a |
| `translate-*` | `TRANSLATE_X/Y` | `translate-x/y` | 同名 | 否（layout-inert） | 0 | v1a |
| `scale-*` | `SCALE_X/Y` | `scale-x/y` | 同名 | 否（layout-inert） | 1 | v1a |
| `layer` | `LAYER` | `layer` | — | 否（只影响排序） | `content` | v1a/R3（弹层需要时） |
| `right/bottom` | 新增 | 同名 | — | 是 | 未设 | v1b |
| `min/max/margin` | 新增/对齐 | 同名 | — | 是 | 0/未设 | v1b |
| `pin`（center/fill/edges） | 新增 | `pin` | — | 是 | 无 | v1b |
| `z`/绘制顺序 | 新增 | `z` | — | 否 | 0 | v1c（可选） |

**每项必做**：props 定义 + `affects_layout` + `zstylec` `props_map`+`value`+诊断 + `zypuic` 白名单/构建（仅允许的项）+ `zypui` IR/实例化/eval/编解码 + 文档 + 测试。

**`.zypui` 边界**：布局属性是 `.zstyle`-only；`BARE_ATTRS` 只保留内容/动态外观与 `fill:` 键的 `width/height`。若未来要内联布局，另立决策，不在本计划里偷偷扩表。

## 4. 开放决策

### 已冻结（D1–D19）

- [x] **D1**：`position: flow | parent | root`；`static/absolute/fixed` 报错、无别名；`root` 为 translate 视觉边界（祖先 translate 清零；opacity 仍相乘，滤镜/混合模式不继承）；真正 overlay 逃逸走根 overlay/portal。
- [x] **D2**：命中跟随视觉；`translate` layout-inert 但影响 paint+hit+damage；`scale` self-only 按 paint 规则；paint/frame/hit 共用 helper；视觉矩形取实际绘制边界（D17）。
- [x] **D3**：`anchor` 只做自身对齐点；约束命名域为 `pin`；禁止 `anchors.*`。
- [x] **D4**：v1a = 手动摆放；v1b = 唯一手动约束求解阶段（right/bottom + min/max + margin + stretch + pin center/fill）；v1c 只留 z/绘制顺序/portal 等非布局能力。
- [x] **D5**：裸元素/类型选择器统一小写（`box/text/image/control`）；移除 `<container>` 与 `Container.`；不保留长期别名；未知裸标签报错；`NodeTag::Container` 在 R3 改名 `Box`。
- [x] **D6**：手动模式缺轴 = 参照原点（原点回落）；取消 static-position 回落；anchor 仅作用于有显式坐标的轴；Stack/Flex 统一。
- [x] **D7**：`center()` = 几何中心；删除 `find_center`；清理旧文档与隐含中心假设。
- [x] **D8**：`Node::rect`/`find_rect` = 布局矩形；`HitNode.rect` = 视觉矩形；`find_visual_rect` root-only/自由函数；`center` = 布局中心；UIA 用视觉矩形。
- [x] **D9**：`left/top` = 锚点坐标；anchor 按轴生效；缺轴忽略该轴 anchor；v1b `right/bottom` 末端边缘忽略 anchor，拉伸忽略 anchor。
- [x] **D10**：flow 诊断矩阵；同规则块 error/warning 分级；跨规则运行时忽略 + debug 一次；用 `lookup` 判断显式设置。
- [x] **D11**：root 视觉边界状态表；`arranged_rect` 绝对坐标；`LayoutEnv { viewport }`（D15）；根节点 `position/left/top/anchor` 忽略/诊断。
- [x] **D12**：小写词表落地到 `zstylec`/`zypuic`/测试/文档；未知标签报错；组件名仍 PascalCase。
- [x] **D13**：hit 按 `(layer,z,preorder)` 取最上层；`ignore_above` 删除；v1c `z` 映射 `SortKey.z`。
- [x] **D14**：`path` = 完整祖先链；`inside`；`target` = 最深 `interactive && inside`；候选全局取最大，不提前返回。
- [x] **D15**：`LayoutEnv { viewport }`；measure 缓存键含 viewport；手动项按轴约束（显式 max=INF；auto max=参考尺寸）；root 项测量但不计数；根节点铺满。
- [x] **D16**：border box/content box/anchor/margin 盒模型表；border width 当前 paint-only。
- [x] **D17**：文字不随 scale 缩放；v1a 视觉矩形/脏区并集文字排版矩形；v1c 再考虑真正缩放文字。
- [x] **D18**：`layer` 解析与默认值；`layer` ≠ `z`；`root` 不改层序；弹层显式 `layer: popup`。
- [x] **D19**：布局属性 `.zstyle`-only；`.zypui` 不提供内联布局属性；表里 `zypui attr = —`。

### 仍需在 R1 冻结（v1a 阻断项）

- [ ] **有效层契约（R1 阻断）**：冻结 D18 弹层子树的 `effective_layer`。二选一：① 使 `LAYER` 具备有效层继承语义（若改属性构造器，保持 `affects_layout = false`，**不能**用 `inheritable_layout`）；② paint/hit 共用“最近祖先有效层”上下文。paint 与 hit 必须同源；补“只在弹层根设 `layer: popup`，子节点仍正确排序/命中”的测试。
- [ ] **文字视觉 bounds（R1 阻断）**：定义唯一来源、失效时机、无 `ctx.engine` / paint 前命中 / 文本变更未重排时的 fallback；paint/frame/hit 共用。不得预设“直接读 `GlyphQuadCache` 一定成立”。
- [ ] `find_visual_rect` 的精确 API：倾向 `visual_rect(root, key)` 自由函数（避免任意子树调用误用）；是否同时提供视觉中心。
- [ ] `measure` 公开签名迁移策略：倾向直接改为带 `LayoutEnv` 的版本并全仓同步迁移，避免两套 viewport 语义；若保留包装必须标记过渡并在 R3 删除。
- [ ] hit 的 O(n) 性能预算与 bench 判据；节点级 `preorder`/paint rank 的缓存位置（不能复用命令级 `SortKey.order`）。
- [ ] `layer` 是否在 v1a 就进 `.zstyle` 解析：取决于 R0 对 dropdown/morph/弹层是否需要跨层覆盖的审计结论（与有效层契约分开冻结）。
- [ ] v1b 完整优先级/冲突表（`left+right`/`top+bottom` 拉伸、`width/height`、`min/max`、`margin`、`pin`、`anchor`）——在 v1b 前冻结，不阻塞 v1a。
- [ ] `pin` 的精确语法（`pin: center` / `pin: fill` / 四边+margins）与 zypui 表达。
- [ ] `position: root` 与未来裁剪/滚动/合成组的关系（是否引入根 overlay/portal；v1c 范围）。
- [ ] `z`/绘制顺序进入 v1c 的精确语义，及其对 hit 顺序的影响（D13/D14 已定原则）。

## 5. 风险与缓解

- **大爆炸回归** → R0 特征化+对拍+分期切换；旧 absolute/static fallback 用例先记录后替换。
- **热路径回退** → 快路径守护+bench/alloc-probe/trace 对照；hit 全局候选 O(n) 单独测。
- **measure 缓存过期** → 缓存键含 `LayoutEnv.viewport`；固定父容器 + root 子项 resize 专项测试。
- **盒模型混淆** → D16 表 + 配图 + border/padding/anchor 参数化测试。
- **文字视觉边界漏算** → D17 并集 + scaled text 测试；damage 宁可多刷不可少刷。
- **hit/paint 顺序不一致** → D13/D14 全局候选 + 完整 path + Popup 先声明用例；`ignore_above` 已删；`preorder` 用节点级 paint rank，不复用 `SortKey.order`。
- **弹层有效层错误** → R1 冻结 `effective_layer` 单一来源；paint/hit 共用；`LAYER` 若走继承语义必须保持 `affects_layout = false`（不得 `inheritable_layout`）；补弹层子树排序/命中测试。
- **layer 解析缺失** → §3 行 + `zstylec` `Layer` 分支 + `layer: popup` 解析测试（按 R0 弹层审计结论决定是否纳入 v1a）。
- **小写迁移破坏旧标签** → `zypuic` 移除 `<container>` 白名单并给迁移提示；未知裸标签报错；`_` 分支显式错误；fmt/门禁夹具同步。
- **`anchor` 尺寸耦合/组合爆炸** → 方案 A + 按轴生效 + 规则表冻结 + 参数化测试。
- **root 测量受父布局影响** → `LayoutEnv` + 手动项按轴约束 + 固定父容器用例。
- **特征化测试变地雷** → 只提交 ignored 目标测试；R2/R4 DoD 清理所有 `R2:` ignore；旧行为进 dev-log。
- **旧契约测试/文档反咬** → R0 清单逐项改写；R4 检查 `paint_transform_tests`、gallery gates、props/zypui 注释、UIA。
- **小写词表机械迁移 diff 噪音** → R3 单独提交；先改词表与夹具，再改布局实现；不与引擎实现混在一个 commit。

---

*主线：R0 特征化 → R1 冻结规范/架构 → R2 实现对拍 → R3 映射+仓内迁移+切换 → R4 验收。已冻结 D1–D19：`flow|parent|root`、命中跟随视觉、`anchor`/`pin` 分域、v1a/v1b/v1c 分期、小写词表、手动缺轴原点回落、布局/视觉/脏区三套几何、root 状态表+viewport、flow 诊断矩阵、hit 全局候选+完整 path、盒模型、文字视觉并集、layer 与 z 分域、布局属性 style-only。开始代码实现前切换出 Plan 模式。*

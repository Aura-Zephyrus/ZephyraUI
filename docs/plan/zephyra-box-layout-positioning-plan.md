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

- [x] 布局特征测试：Stack/Flex/spacing/align/padding/尺寸/collapsed/文本/现有 absolute/缓存命中失效/resize；补齐缺口（见 `docs/dev-log/2026-11-layout-refactor-box.md` 与 core/layout 测试）。
- [x] **现存缺陷特征化**（记录于 dev-log；R2/R3 已按 D1–D19 修正）：
  - Stack 与 Flex 脱流缺轴回落不一致；
  - `left/top` 中心语义 + `find_center` 旧文档；
  - `translate/scale` 命中与 paint 不一致；
  - hit 顺序与 paint 的 `(effective_layer,z,preorder)` 不一致；`ignore_above` 从未读取；
  - root/视口测量约束缺失；measure 缓存键不含 viewport；
  - 文字视觉 bounds 时序/fallback 风险（R1 冻结，R2 实现）；
  - `layer: popup` 解析/有效层传播风险（R1 冻结，R2 实现）；
  - `zypuic` 旧 `<container>` / 未知裸标签诊断迁移；
  - UIA 屏幕矩形使用 `arranged_rect`。
- [x] **测试卫生**：旧行为写入 dev-log；目标行为测试按 `#[ignore = "R2: ..."]` 落地后在 R2 清理。R4 复核：`git grep -n '#\[ignore = "R2'` 无输出。
- [x] **旧契约盘点**（已在 R2/R3 改写/迁移）：`paint_transform_tests`、gallery gates、props/zypui/paint 注释与断言、`layout_classification_tests` 相关、UIA `screen_rect`、`ignore_above`/hit 语义、`zypuic` `<container>` 夹具。
- [x] 视觉/几何基线：gallery `--dump-cpu` 路由基线记录于 dev-log（2026-11-08，Windows，条件见 dev-log 表）；后续对拍以“预期语义变化 vs 回归”判定。
- [x] 依赖与痛点盘点：`layout`/`node`/props/`frame`/`paint`/`hit`/`style`/`context` 映射到 R1/R2，见 dev-log。
- [x] 小写词表盘点：`Container/Text/Image/Control`、`<container>`、`TAG_NAMES`、`zypuic` 裸标签/夹具、Rust 内联/文档（计划与 dev-log 记录规模）。
- [x] dev-log 立项：`docs/dev-log/2026-11-layout-refactor-box.md`。

### R1 — 架构与规则设计（已冻结）

- [x] **规范层产物**：`docs/developer/layout-spec.md`（坐标/盒模型/root 状态表/命中算法/测量约束/诊断/参考表），作为实现依据。
- [x] 新布局核心契约：树+样式+约束+上下文 → 测量/放置；`LayoutEnv`/缓存失效；方案 A；`Position::{Flow, Parent, Root}`；Stack/Flex/手动路径边界（R2 落地）。
- [x] `Node` 模型/几何 API：保留 `arranged_rect` 语义；删除 `find_center`；`visual_rect`/`visual::text_visual_bounds`/`damage_bounds` 与共享 `VisualState`；`HitNode.rect`=视觉矩形（R2）。
- [x] **几何契约**：`VisualState`/`enter()`、视觉矩形并集文字 bounds（D17 fallback）、`damage_bounds`、scale self-only、root translate 边界；paint/frame/hit 共用（`zephyra_core::visual`，R2）。
- [x] **有效层契约（R1 阻断，已冻结并实现）**：paint/hit 共用遍历时 `effective_layer` 上下文（最近祖先有效层）；`affects_layout=false` 不变；D18 弹层子树用例覆盖（R2）。
- [x] **命中契约**：`(effective_layer,z,preorder)` 候选、完整祖先 `path`+`inside`、`target`、`ignore_above` 删除；节点级 preorder；filter/decor 不进入 hit（R2）。
- [x] **测量契约**：`LayoutEnv { viewport }`、缓存键含 viewport、手动项按轴约束、root 项/根节点规则；全仓签名同步（R2）。
- [x] **盒模型契约**：border box/content box/anchor 关系写入 layout-spec（含 ASCII 图）。
- [x] **冻结规则表**：原点回落、布局接管、translate/命中、root 边界；v1b 冲突表细节在 v1b 前冻结（不阻塞 v1a）。
- [x] **anchor 按轴表**：按轴生效/缺轴原点/no-op warning；v1b right/bottom/stretch 与 anchor 的细则留 v1b 冻结。
- [x] **诊断契约**：D10 矩阵落地到 zstylec/运行时路径（按 lookup/显式设置）；旧 `static/absolute/fixed` 给迁移提示。
- [x] **小写词表**：`box/text/image/control`、移除 `<container>` 并迁移提示、未知裸标签报错、组件 PascalCase（R3）。
- [x] **layer/属性表**：`layer` 解析与 §3；有效层按 R1 决议；`z` 仅 v1c；布局属性 `.zstyle`-only（R3）。
- [x] **v1b 优先原则**：已记录为 v1b 前冻结项（双端+显式尺寸/min>max/pin 冲突倾向 error）；v1a 不实现 v1b 约束求解。
- [x] zstylec/zypui 属性与 IR 影响评审完成并落地（R3）。

### R2 — 实现（已完成）

- [x] 新引擎实现（生产路径唯一）：Stack/Flex/collapsed/文本 + `flow/parent/root` + `left/top` + `anchor` + 原点回落（见 `zephyra-core` layout/visual/hit/paint/frame）。
- [x] 无生产双引擎/运行时开关；迁移按 clean-break 完成。
- [x] **D2/D8/D11/D13/D14/D15/D16/D17/D18**：translate/scale 进入 hit 与视觉矩形；文字 bounds fallback；root translate 边界；paint/frame/hit 同规则；`effective_layer` 单一来源；hit `(effective_layer,z,preorder)`；完整 path+inside；`ignore_above` 删除；相关用例落地（见 dev-log）。
- [x] 几何/行为按新语义对拍（预期差异即 D1–D19）；缓存/dirty 与 `LayoutEnv.viewport` 对齐。
- [x] 性能守护按计划方向推进；bench/trace 细化按后续需要补充（v1a 功能门禁优先）。
- [x] core 单测覆盖主要 v1a 场景（anchor/轴/模式/回落/translate-scale 命中/root 边界/hit path/visual 几何等）；`cargo test -p zephyra-core --lib` 434 passed（2026-11-08 记录）。
- [x] `#[ignore = "R2: ..."]` 全仓清零（2026-11 复核）。

### R3 — 映射、迁移、切换（已完成）

- [x] **core props**：`Position::{Flow, Parent, Root}`、`ANCHOR`、`left/top` 锚点坐标语义、`affects_layout`；移除旧模式/中心落位语义；`find_center` 删除。
- [x] **zstylec**：`flow|parent|root` 解析，拒绝 `static/absolute/fixed` 并给迁移映射；`anchor`/`layer`；小写 `TAG_NAMES`；诊断/格式化/门禁同步。
- [x] **zypui/zypuic**：`<box>`、裸标签小写、未知裸标签/`<container>` 迁移提示、布局属性 `.zstyle`-only、IR/编解码同构；`cargo test -p zypuic -p zypui` 通过。
- [x] **D5/D12 机械迁移**：仓内 `.zstyle`/`.zypui`/相关 Rust 夹具迁移完成；`NodeTag::Box` 词汇落地；门禁：全仓 `.zstyle` 零诊断、`.zypui` 无 `<container>`。
- [x] **仓内迁移（clean-break）**：旧模式/旧标签在有效源码与样式中清理；保留的旧词仅出现在诊断文案、历史注释/迁移记录等有意位置（如 zstylec 错误信息、dev-log）。
- [x] **D7 清理**：`find_center` 删除；`center`/`rect` 语义按布局几何；视觉几何走 `visual`/hit 路径。
- [x] **UIA**：`rules::build(root, win, ctx)` 使用视觉几何；相关调用点/测试更新。
- [x] **旧契约测试/注释改写**：R0 清单项已处理（含 `paint_transform_tests`、gates、UIA）。
- [x] **root + layer 检查**：`position: root` 仅改坐标参照/translate 边界；层序由 `effective_layer`/显式 `layer` 处理（D18）。
- [x] 默认引擎/语义切换完成；旧窗口+中心路径移除。

### R4 — 验收与文档

- [x] `cargo fmt --all -- --check`（2026-11，exit 0；stable 的 `wrap_comments` 警告为环境噪音）。
- [x] `cargo clippy --workspace --all-targets -- -D warnings`（2026-11 本轮复跑至 `Finished dev profile`；增量关键 crate `zephyra-core/gallery/ui/zstylec/zypuic/zypui/render-ext/ext` 复核 exit 0）。
- [x] `cargo test --workspace --no-fail-fast`（2026-11 本轮复跑；`test result:` 汇总无失败，`0 failed` 全程；含 `zypuic`/`zypui`/`zstylec`/UIA 相关 crate；GPU/诊断类测试按既有约定 `#[ignore]`）。
- [x] `cargo doc --no-deps --workspace`（2026-11 本轮 exit 0；既有私有链接等 warning，与基线一致，无新增 Rust 代码引入的回归）。
- [x] `docs/developer/layout-spec.md` 定稿/补全；`docs/dev-log/2026-11-layout-refactor-box.md` 含现象/排查/排除的假设/结论+门禁与实测条件。
- [x] 诊断含修复示例；布局居中（`main-align/cross-align`）首推；`position` 模式对照表（`pin` 标注 v1b）进文档；小写词表写进样式/UI 规范；盒模型/anchor ASCII 参考图（见 `layout-spec.md`、`04-stylesheets.md`、`07-declarative-ui.md`）。
- [x] 全仓不再有 `#[ignore = "R2: ..."]`；旧契约测试已改写。

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

### 仍需在 R1 冻结 / 后续阶段（状态）

- [x] **有效层契约（v1a 阻断，已冻结+实现）**：采用 paint/hit 共用的“最近祖先有效层”遍历上下文；`LAYER` 保持 `affects_layout=false`；弹层子树排序/命中用例已补（R2）。
- [x] **文字视觉 bounds（v1a 阻断，已冻结+实现）**：`visual::text_visual_bounds` + glyph cache / engine fallback 路径（见 layout-spec §4、R2 实现）。
- [x] `find_visual_rect` / 视觉 API：核心实现为 `zephyra_core::visual::visual_rect` 等共享 helper；UIA/hit 使用视觉几何。若需对外暴露更窄的 root-only API，作为后续 API 整理。
- [x] `measure` 公开签名/`LayoutEnv`：已全仓同步迁移，避免两套 viewport 语义（R2）。
- [x] hit O(n)/节点级 preorder：实现采用节点级 preorder 与全局候选选择；进一步 bench 记录可在后续性能专项补齐。
- [x] `layer` 在 v1a 的 `.zstyle` 支持：已按弹层/迁移需要纳入解析与有效层处理（R2/R3）。
- [ ] v1b 完整优先级/冲突表（`left+right`/`top+bottom` 拉伸、`width/height`、`min/max`、`margin`、`pin`、`anchor`）——v1b 前冻结，不阻塞 v1a。
- [ ] `pin` 的精确语法（`pin: center` / `pin: fill` / 四边+margins）与 zypui 表达——v1b。
- [ ] `position: root` 与未来裁剪/滚动/合成组/overlay-portal 的关系——v1c/后续。
- [ ] `z`/绘制顺序进入 v1c 的精确语义——v1c（D13/D14 已定原则）。

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

*主线：R0 特征化 → R1 冻结规范/架构 → R2 实现对拍 → R3 映射+仓内迁移+切换 → R4 验收。D1–D19 已冻结并在 v1a 落地；R4 四门禁于 2026-11 复跑通过（fmt/clippy/doc/workspace tests 无失败，忽略项为既有 GPU/诊断探针）。v1b（`right/bottom`/`min/max`/`margin`/拉伸/`pin`）与 v1c（`z`/portal/overlay 逃逸等）按计划分期推进。*

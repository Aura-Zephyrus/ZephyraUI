# .zypui 结构迁移：控件层 + gallery 外壳 + demo 全量落地

> 现象：三处 Rust 视图代码（`ZephyraUI/ui/controls.zypui` 的对照树、
> gallery 的外壳与 17 个页面、`examples/demo.rs` 的六个视图函数）仍在用
> Rust 构造器手写结构树——「结构的真相只有一份」没有兑现。

设计定稿（XML/HTML 风格、编译为二进制运行时零解析、全展开成裸元素树 +
组件模板、状态表达式只在实例化期求值、外观全在 `.zstyle`）此前已拍板。
本文只记**落地时踩到的缝**与验收方式。语言规范见
[`docs/developer/07-declarative-ui.md`](../developer/07-declarative-ui.md)。

## 排查

三段式推进，每段都以「树全等 parity」验收后才删旧代码：

1. **语言层补洞**：`Val::Raw(PropertyValue)` 变体（结构值透传）、`filter` /
   `blend` 两个 bare attr（`Val::Raw` 专用通道，绑定给错类型按「没设」降级）、
   `Host::tr` 签名收紧为 `-> &'static str`（与零分配纪律对齐）。
2. **gallery 全量**：`ui/shell.zypui`（~600 行：外壳 + 17 页组件 + 页分派）
   + `app/data.rs`（绑定数据面，全 `&'static` 表）+ `impl zypui::Host`（~70
   绑定名）；迁移期 parity 测试对 9 路由 × 输入态 × 22 项扰动逐树全等。
3. **demo**：`examples/ui/demo.zypui`（组件 FxSwatch + 单页）+ build.rs +
   `ui.rs`/`data.rs` + Host 实现；parity 四种 hover/press 组合全等后删参照树。

## 踩到的缝（每条都真红过一次）

- **组件数值/布尔实参必须带 `{}`**。`attr_to_expr` 无 `{` 即整体字符串
  字面量：`value01="0.65"` 的 `as_f32` 失败、宽度根本没设。正确写法
  `value01="{0.65}"`。字符串实参裸写才是对的。
- **未传实参求值为假**。依赖「缺省 = 空串」语义的参数（`value` / `label`）
  必须显式传空串，否则 `value != ''` 恒真（TextField 多出 clear 键）。
- **类序即匹配序**。`Control.caption-btn.close.hovered` 要求 close 在
  hovered 之前——词序错只有样式规则门禁与 parity 特定快照才暴露。
- **`for` 循环内 `hover`/`press` 基准不是实例 key**。内置词基准只在组件
  实例（Use）边界更新；循环项必须显式 `hover(item.key)`，否则所有项共享
  同一个（错误的）基准。
- **`<for>` 是属性式语法**：`<for var="b" list="@buttons">`，不是
  `<for b in @buttons>`。后者被 XML 解析器当成「属性 `b` 缺 `=`」，报错
  位置也完全对不上真实意图。
- **表达式逻辑算子是 `&&` / `||`，不是 `and` / `or`**。`!press(k) and
  hover(k)` 报「表达式后有多余内容」——报错列指向整段词，真相是 `and`
  不是算子。XML 侧 `&&` 原样通过（只解 `&amp;`）。
- **属性插入顺序不是语义**。demo parity 首轮唯一的 diff 是 crop 图两个
  属性的先后（模板 texture 先、Rust 构造器 src 先）——`PropertyStore`
  是按名字取用的映射，对照时按键名排序，不烤死书写顺序。
- **结构值只有 `Val::Raw` 一条路**。表达式语言没有 `FilterChain` /
  `BlendMode` 字面量；宿主绑定给整个值，`Raw` 在其它上下文按「假」降级。
- **build-dependencies 段必须放 Cargo.toml 末尾**。插进 `[dependencies]`
  段中间会把后续依赖截进 build-deps，症状是 bin 里 winit 等全部未解析。
- **tr 门禁要扫双形态**：Rust 侧 `g.tr("key")` 与 `.zypui` 侧 `tr('key')`；
  `.zypui` 的 XML 块注释要**整体剥离**再逐行扫（续行不以 `<!--` 开头，
  否则扫进注释里的键）。

## 排除的假设

- 「组件实例化时才展开，可以用文本级宏替换省掉逐帧求值」——否。实参是
  表达式、逐帧变值，加载期替换需要两份表示 + 克隆整棵树，省的只是名字
  查表；一次遍历同时完成展开与求值才是对的形状。
- 「`.zypui` 写不了自定义属性键（NOTE / Z_INDEX），需要加属性通道」——否。
  键的真相在宿主手里（`PropertyKey` 按地址身份），模板本来就不该写应用
  键；Rust 侧 view() 补挂两个 `with` 即可，外观门禁不涉及它们。
- 「demo 的集成测试（demo_interaction）要跟着改」——否。它自建本地视图，
  与 demo 树零耦合，迁移后原样绿；动的是 bin 内单测里三 条 `button()`
  级断言（改为按 key 从实例化树上找节点）。

## 结论

三处结构真相各归一份 `.zypui`；Rust 侧只剩状态、输入转发、事件分派与
绑定数据面。语言层最终形状：11 个 bare attr（含 `filter` / `blend`）、
`Val::Raw` 结构值通道、单臂三元、属性式 `for`。

## 门禁

- `zypui` 12 / `zypuic` 19 测；`zephyra-foundation` bin 单测 6 条
  （三 条整树门禁 + 三 条按钮状态断言）与 `demo_interaction` 7 条全绿。
- demo 迁移期 parity（`.zypui` 树 ≡ 旧 Rust 参照树，四种 hover/press
  组合逐节点全等）全绿后**已下线**——它是验收工具，不是长期门禁。
- 长期钉住的是既有三条：整树零外观、分类名全覆盖、元素可达；加上
  `gallery/tests/gates.rs` 的全部门禁（tr 双形态扫描在内）。
- 端到端冒烟：`cargo run -p zephyra-foundation --bin demo -- --dump-cpu`
  （141 quads / 字形 141 / 缺失纹理 0；`Command::Path 0` 是图标名不在
  内置图标集的既有现状，与迁移无关）。

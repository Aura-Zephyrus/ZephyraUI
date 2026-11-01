# 开发日志

给**内部**的。每条记一个问题：现象 → 排查 → **排除过的假设** → 结论 → 钉住它的门禁。

用户文档在 [`../developer/`](../developer/README.md)。这里的「排除过的假设」在那边
写不出来，而那恰恰是最值钱的部分。

## 新的一条怎么写

```markdown
# YYYY-MM-DD-<slug>

> 现象：<一句可复现的描述>

## 排查

## 排除的假设          ← 不能省。下一个来的人靠这段省下几轮

## 结论

## 门禁                ← 哪条测试钉住了它。没有门禁的结论会烂掉
```

## 索引

| 日期 | 条目 | 状态 |
|---|---|---|
| 2026-11-01 | [窗口透明模式；DWM 背景材质（Mica/Acrylic）整条移除](2026-11-01-window-transparent.md) | 已落地；透明是 `window.transparent`（主窗口默认）→ `WindowOpts::transparent`（per-window，创建时定）→ `ZEPHYRA_TRANSPARENT=1\|0`（运行时覆盖）三层入口，且 **transparent ⇒ frameless**。真正缺的从不是混合语义（内容侧早就全 premultiplied），而是「窗口层让 alpha 进合成器」：D3D12 本就走 DComp composition swapchain（只差窗口属性 + clear alpha=0）；D3D11 的普通 HWND swapchain **实测拒收** premultiplied（一律 `DXGI_ERROR_INVALID_CALL`）⇒ 透明时强制 DComp 且建链失败直接 panic（退回普通链 = 静默变不透明）；Vulkan 的 composite alpha 探测本就首选 `PRE_MULTIPLIED`、clear 本 `(0,0,0,0)`，只加 `with_transparent`。**冲突**：`DWMWA_SYSTEMBACKDROP_TYPE` 与逐像素 alpha 是两套互相排挤的合成路线 ⇒ `BackdropKind` / `WindowChrome.backdrop` / `ApplyOutcome.backdrop` / `Capabilities.backdrop` / `dwm_backdrop` / `DwmAttr::SystemBackdropType` / `backdrop_active` / gallery `.mica` 分类名与两条门禁**整条删除**（窗口内 `Filter::BackdropBlur`／磨砂与 D3D12 折射的 backdrop 副本**保留**，同名不同物）。排除的假设：①「Mica 与透明只差 clear 色」（错：backdrop 窗口是 opaque 合成，clear 透明也只会露出材质区域，给不了「半透明控件叠桌面」）；②「D3D11 普通链能开 alpha」；③「winit 透明在 Windows 是 `WS_EX_LAYERED`」（实为 `DwmEnableBlurBehindWindow` 空区域，抄 0.30.13 源码确认）；④「Vulkan 要补清屏色」。门禁：app-config 8 条（含 env 双向覆盖与 per-window 覆盖语义）、`zephyra-dcomp --test smoke::premultiplied_chain_survives_an_alpha_window`（真机：winit 同款 alpha 窗口 + premultiplied 链整链）、既有 premultiplied 断言、gallery 半透明令牌门禁改名留存、CLI 28 条。多窗口的 JSON 定义形状写进 TODO（`docs/developer/win/07-window-transparent.md` §5）不本轮做 |
| 2026-11-01 | [材质专区 + 液态玻璃合并，背景换成真实照片](2026-11-01-materials-photo-background.md) | 已落地；路由 `glass` → `materials`，材质专区与液态玻璃合成一页，页面中央是**真实山景照片**当玻璃舞台背景（用户指定的那张 `kinkate-mountain-landscape-2031539_1920.jpg`，编译期内嵌 JPEG、运行时解 RGBA8 注册进宿主；裁成 1556×1020 → 1464×960，宽高比 1.525 与照片板 732×480 精确一致）。板上三块材质样板（纯色 / 磨砂 / 液态玻璃）+ 三块可调参数的玻璃块。**但照片把压在它上面的纯色样板连同标签一起盖掉了** ⇒ 根因在 `zephyra-fx/src/quad.rs`：展开层把顶点分 `solid`/`images`/`paths` 三段存，`merge_runs` **按类别**拼 run ⇒ 绘制顺序成了「图元类型分组」而非 DisplayList 顺序 ⇒ 任何 `Command::Image` 都排在同段所有实心四边形**之后**绘制。修法：不动顶点流三段布局（重排它要连带改三后端索引定位），只把 run 的先后改回 DisplayList 顺序 —— `RunTally` 三份合成一份按序记、图像顶点与 run 在命令遍历里**就地产生**（分批规则不变）、段合并判据从「同类别里的上一条」改成「**紧邻的上一条**」。`DrawRun` 公开结构与三后端用法**一字未改**（索引定位逐段独立，与段序无关）；CPU 参考栅格 `draw_batch` 同步改逐 run 画（此前是同一缺陷的另一半）。新门禁 5 条（图像不盖住后面的实心 / 中间夹图像时实心不并段 / 两图像夹实心不并段 / **像素级**：蓝图像+后画红实心 ⇒ 画布必须红 / 900 条交错命令：段序列 == 命令序列的游程编码）；既有「runs 铺满顶点流」断言从「按 runs 顺序首尾相接」改成「**按 `first_vertex` 排序后**首尾相接」（段序是 DisplayList 顺序，`first_vertex` 是合并流位置，两者序不必一致）。变异验证：`merge_runs` 改回按类别分组 ⇒ 门禁当场全红。另记坑：① `zstylec` **不认 `//` 注释**（只认 `/* */`），而一份表解析失败的代价是**整份表被丢弃**、页面静默退回默认排版（与「多值 padding」同坑）；② 照片裁切也是设计 —— 初版裁全高，样板行正压在最亮的雪地上、纯色与磨砂的差别被压平，改裁掉大块天空后样板行落在岩石峰体上、下半是石阶与花海（折射与色散最明显处） |
| 2026-11-01 | [Button / CheckBox 与 i18n 深度融合](2026-11-01-button-checkbox-i18n.md) | 已落地；控件模板的文案实参改收**语义 key**（`label_key`），翻译发生在**控件内部**（`tr()`）—— 「控件只发 key，文案表是应用的」这条纪律写在 `Cargo.toml` 里很久却**从未兑现**（模板里是 `text="{label}"`，收的是应用已翻译好的文案），于是「控件层不发面向用户的文案」这条更严的纪律**没法做成门禁**（判据不存在）。新建 `ZephyraUI/src/i18n.rs`（`NAMESPACE` / `BUILTIN_KEYS` / `KEY_PARAMS` 契约表 / `builtin_messages` / `merge_builtin`）；控件层自带**控件语义**的默认译文（复选框组组头的「全选」= `zephyra.checkbox.select-all`，三语言，应用可覆盖）；应用侧 `CATALOG` 新增 32 键 × 3 语言（模板里约 30 处硬编码散文全量入表），`messages()` 改 `OnceLock` 缓存（`tr()` 是逐元素调的，`Messages` 内部是 `BTreeMap` ⇒ 每帧重建 = 200 多次节点分配；控件模板的 `tr(label_key)` 把次数翻倍）。新门禁 6 条（控件层零裸文案 / 应用层裸文案须命中豁免表 / 每个 `label_key` 三语言齐全 / 契约表与模板对得上 / 切语言后控件文案必须变 / 强制语言须活过系统协商）；豁免表按**值**列（命令行、符号、数字、语言自称 —— 自称不是翻译）。坑：① 改 `Checkbox` 的 `label` 语义为 key 时**类型没变**（都是 `&'static str`）⇒ 静默改语义会让「英文原文当 key」正好显示成原文，**画面上一模一样** ⇒ 字段与方法一起改名，让编译器把旧调用点指出来；② `--lang` 在**窗口路压根不读**（`--route`/`--family` 同款漏），而且就算读了也会被 `on_input` 的**系统语言协商**覆盖 —— 两个坑叠在一起，且**恰好在它唯一有用的场景里发作**（本机系统语言 zh-Hans 时 `--lang en` 看不出来）；③ 契约门禁第一次跑就红：门禁自己写错了 —— `CheckBoxGroup` 是**转发**不是直接翻译 ⇒ 规则改成「直接翻译**或**转发都算，但转发必须有终点」；④ 语言名一度被塞进表里（`ui.lang.*`，三语言同值），与 `docs/developer/i18n.md` §5.2「自称故意不进表」冲突 ⇒ 撤回，改走豁免表；⑤ `messages()` 改 `&'static` 后「每门语言条目数 == 键集合数」那条门禁必红 ⇒ 键集合要算上控件层的 `BUILTIN_KEYS` |
| 2026-11-01 | [gallery 裁剪：只演示 Button 与 CheckBox](2026-11-01-gallery-trim.md) | 已落地；**只砍 gallery 演示，控件库 54 个组件全部保留**（能力缺口是另一件事，记在 `docs/todo/control-capability-audit.md`）。口径：被演示的控件只留 Button 族 + CheckBox 族，**外壳/导航结构保留**（NavItem / InfoBadge / ToolButton / Segmented 族切换器是结构，砍掉就切不了页也切不了族）。删 8 个页面组件、24 个状态字段、25 张数据表、约 50 条 view 绑定、10 条死样式规则；`REF_FAMILIES` 九族 → 两族；`CATALOG` 136 键 → 59 键（231 → 177 条）。门禁四处适配（族数 9→2 写死、`only_fill_nodes_carry_local_width` 由「fill ≥ 4」改成反向的「fill == 0」—— 原判据在控件退场后**恒真**、`route_table_and_page_table_agree` 不再补 Settings、导航栏几何断言从「按名字钉」改成「按几何钉」）。坑：① `every_stylesheet_rule_matches_something` **只查应用层**（控件层不受影响）且**依赖每族一个脚本**，少一族 ⇒ 那族类名被误判成死规则；② i18n 门禁是**单向**的，多余条目不会红 ⇒ 删控件后得自己清（本次清 77 个死键，用 HEAD 版导出做对照，区分「本次产生的 72 个」与「更早轮遗留的 5 个」）；③ **脚本改文件把 LF 写成了 CRLF** ⇒ git 判整文件重写（`pages.zstyle` 变成 437 增/491 删）—— 别用 `git show \| grep -c $'\r'` 判断（MSYS 管道层会翻译，永远是假的），要用 `git cat-file blob` 走二进制；④ 脚本删绑定/删表项各坏过一次（吞掉下一条绑定、切错多行 const 位置）⇒ 一律先在副本上跑 |
| 2026-11-01 | [复选框：三态、组、禁用、键盘、无障碍](2026-11-01-checkbox-tristate.md) | 已接；`CheckBox` 从两态升为真三态（`tristate` 是 `.zypui` 的**结构开关**、状态值与流转规则住 `ZephyraUI/src/checkbox.rs`、状态存哪归宿主），新增 `CheckBoxGroup`（三态组头 + 平铺两态子项，组头状态**由子项聚合派生**），加 `disabled`（zpui 的 `BARE_ATTRS` 原本没有这个属性 ⇒ 补了两处）。坑：① zypui 的「实例补丁」会把 Use 的 `key` 打在组件体第一个元素上 ⇒ 组 key 必须走**实参** `gkey`，否则容器与组头同 key ⇒ 焦点环画不到框上、Space 闩不住，而画面上看不出来；② 禁用子项**不参与**聚合，否则组头卡在半选、点了永远没反应；③ 勾号与横杠是**同一节点的两个形状**（靠 `icon-color: transparent` 互换），少一个节点就少一处「状态增删节点」的抖动；④ 分类词由 Rust 侧产出（拼错分类名是静默失效）；⑤ gallery 的 `--family` 在窗口路漏读（与当初 `--route` 同款漏）。另清掉存量红：着色器 mtime 守卫（10-30 那次改动只重编了新入口，其余 19 个 `.bin` 是旧产物）|
| 2026-10-31 | [UI Automation 原始提供者：树通了，图案取不到](2026-10-31-uia-raw-provider-toggle-pattern.md) | 已接；`zephyra-uia`（`zephyra-base/extra/`，Windows 专属）交出整棵原始提供者树：名字 / 角色 / AutomationId / 禁用态 / **Toggle 图案**（On/Off/Indeterminate）/ Invoke 图案，跨进程真机 dump 通过。**根因**：raw provider 的图案接口是 **`IToggleProvider` / `IInvokeProvider`**，不是 `IUIAutomationTogglePattern` / `IUIAutomationInvokePattern`（后者是客户端侧、UIA 自己实现）—— 两者 IID 不同，UIA 的服务端 QI 的是前者 ⇒ 实现错就**静默**判定「不支持」（元素、名字、角色全对，唯独图案读不出来，极难归因）。另：`ProviderOptions` 必须报 `ServerSideProvider`（报 0 ⇒ 静默退回 HWND 代理，`FindAll` 返回 0 个元素）；必须实现 `IRawElementProviderFragmentRoot`（否则客户端当场放弃整棵树）；`WM_GETOBJECT` 的返回值是**连接 key 不是指针**（deref ⇒ 访问违规）；「没有元素 / 不支持这个图案」要表达成 **成功 + NULL** 不是错误码；窗口销毁要调 `UiaReturnRawElementProvider(hwnd,0,0,NULL)`。诊断教训：Rust 侧拿到「COM 成功 + 出参 NULL」时 windows-rs 会翻译成 `Err(code=S_OK)`，判据要落在 COM 层就**打 vtable** |
| 2026-10-31 | [`zephyra fmt`：三个排版器聚合](2026-10-31-cli-fmt-three-formatters.md) | 已接；`.rs` 走 `cargo fmt`、`.zstyle` 走 `zstylec::format`、`.zypui` 走 `zypuic::format`，CLI 做聚合（只找文件/写回/报错，**不判排版** —— 判据在编译器里，两份实现必然漂）。`.zstyle` 排版器**只动空白**：IR 把值归一化了（`10px`→`Float`、`@令牌`→`Resource`），从 IR 回写不可能还原源码拼写。不认识的源码返回 `None` 原样保留。坑：`split_attrs` 推入属性后忘了推进游标 ⇒ 死循环 OOM（三条仓内文件全跑不过）；`.zypui` 值里允许裸 `>`（`cond="a > b"`），`find('>')` 定位标签尾会撕坏；门禁判据从上一轮搬来时漏了「缩进是 2 的倍数」一条；测试里 `set_current_dir` 与并行用例互踩（改全传绝对路径） |
| 2026-10-31 | [.zypui 写法统一（排版门禁）](2026-10-31-zypui-style-normalization.md) | 已接；五条排版规则（标签不跨行 / 一元素一行 / `key`→内容→行为→`class` 收尾 / 缩进 2 空格 / 非空行以 `<` 开头）成文 `docs/developer/zypui-style.md`，`zypuic` 扫全仓 `*.zypui` 两条测试钉住；三份存量文件归一化，门禁违规 90 → 0（纯排版，属性/标签/注释多重集与改动前全等）。三个坑：归一化脚本曾吞掉跨行标签后的元素（靠多重集对照抓到）、门禁 `attr_names` 曾被缩进卡住少报近一半 class 违规（靠拿旧内容回跑门禁抓到）、PowerShell `Get-Content` 把 UTF-8 读成乱码且行数不对（别拿它数行） |
| 2026-10-29 | [元素定位：坐标进样式表](2026-10-29-element-positioning.md) | 已接；`position: static│absolute` + `left` / `top`（DIP，**窗口坐标**，等价 CSS `fixed`）；绝对子项脱离正常流但**仍测量**、只不占父内容尺寸；`arrange_absolute_children` 按窗口坐标摆位；新增 `Node::find(key)` / `find_rect(key)` 做按 key 查窗口矩形；`zstylec` 同步映射与关键字。画序仍在树序，未做 z-index |
| 2026-10-28 | [焦点状态机：Tab 顺序 / 圈闭 / FOCUSED 属性](2026-10-28-input-focus.md) | 已接；core 新建 `input::focus`（`FocusState`），props 加 `FOCUSABLE` / `FOCUSED`；Tab 按树深度优先移动、末端圈闭；FOCUSED 走 Local 层属性（`set_prop`），`.zstyle` 可按它给焦点环；**三宿主已接入**（点击设焦点 / Tab 当帧移动 / 窗口失焦清空，`InputState.focused` 每帧上报）——控件层尚未消费 FOCUSED |
| 2026-10-28 | [键盘 / 字符 / IME 事件契约与三后端接线](2026-10-28-input-keyboard-ime.md) | 已接；core 定 `TextEvent`（`Key` / `Char` / `Ime` 三通道，`KeyInput` 带语义键+键名+重复+修饰键），新建 `zephyra-winit-input` 一份翻译供三宿主共用；Vulkan 补上键盘/滚轮/IME（此前 `key_events` 与 `scroll` 写死空值）；MCP `press_key` 改走同一套词汇。控件层尚未消费 |
| 2026-10-26 | [无标题栏窗口模式](2026-10-26-frameless-window.md) | 已接入 D3D11 / D3D12 / Vulkan 宿主；各消费项目根目录的 `ZephyraUI.json` 用 `window.frameless` 控制系统装饰；Gallery D3D12 实机确认客户区顶边与窗口顶边齐平。 |
| 2026-10-26 | [MCP 拖动、性能分析与资源遥测](2026-10-26-mcp-clicks-resources.md) | `pointer_drag` 按住左键支持折线路径/直线/easing；一次性 profile 必填时长，自动停止并返回逐帧 CPU 阶段耗时与统计；`get_resources` 提供进程 CPU/内存和可用时的 GPU 引擎/显存遥测；D3D11/D3D12 真机闭环覆盖拖动、profile、资源及布局坐标点击。 |
| 2026-10-26 | [MCP 输入、布局与 D3D12 截图](2026-10-26-mcp-input-layout-d3d12-capture.md) | 已接；`get_layout` 暴露文本布局树，`press_key` / `scroll` 走原生输入状态；D3D12 后缓冲经 readback heap 读回，截图强制完整重绘；`.zypui render-source` 只保留宿主 kind 查找，不携带 GPU payload。D3D11 / D3D12 真机 MCP 各验键盘、滚动、点击后布局变化和 BMP；修复大响应非阻塞续传，以及 WSL 慢 loopback 下单次 poll 假设 |
| 2026-10-25 | [Vulkan 渲染源钩子与 validation 门禁](2026-10-25-vulkan-render-hook-validation.md) | 已接；Vulkan 两段式计划保留 Custom 下标，record 阶段在主 render pass 中调用宿主钩子；WSLg/llvmpipe 端到端读回 7×9 恰好 63 个红像素，未注册时全黑且计数正确；验证层门禁覆盖回调收集/抽干及成功帧零 ERROR。已修 debug callback Arc 指针误解引用、空几何零字节 map、未注册 Custom 不建立有效 PRESENT 图像；物理 GPU 驱动复核仍待 |
| 2026-10-25 | [外部渲染源拓展接口：Command::Custom](2026-10-25-render-source-extension.md) | 已接；DisplayList 开出唯一放行位 `Command::Custom{bounds,source,filter,blend}`——核心只搬运、后端按 `kind` 查钩子注册表、宿主解释 `Arc<dyn Any>` 载荷（指针相等 = diff 静止）；`Segment::Custom` 保序（段数不变量 `2f+2c+1`，压力测试同步扩展）；降级不崩（未注册跳过+计数，`custom_passes`/`unhandled_custom` 双面计数）；D3D11 端到端验收：钩子 `ClearView` 涂红、读回像素恰好 = bounds 面积。原记录时 Vulkan 尚未接线，后续实现与 WSLg 软件 Vulkan 验收见本索引上方新条目 |
| 2026-10-25 | [MCP 调试桥：AI 直连运行中的界面](2026-10-25-mcp-debug-bridge.md) | 已接；三层链路 `AI ── zephyra mcp（JSON-RPC/stdio）── TCP 行协议 ── fx::bridge`——库层纯 std 零依赖、JSON 只落在 CLI（显式破例有注释）；**操控 = 输入注入**（与物理鼠标同一条命中/路由路径，属性直改会被 diff 覆盖）；树 dump 出绝对坐标（抄坐标即可点击）；截图钉 present 前（flip model 读回实测表）；D3D12 控制流联动（has_client→Poll/Wait）；工具 7 个（tree/props/stats/viewport/hover/click/screenshot），D3D12 截图 v1 诚实降级 |
| 2026-10-25 | [core 结构审计与 API 收口](2026-10-25-core-structure-audit.md) | 已接；真实依赖图落地 `lib.rs`（node 枢纽三处双向 = 缓存写回形状，非违规）；模块表补 responsive/i18n/styler；`control_fill_role` 归位 `theme::roles`；收口删 8 项零引用 API（含文档谎称「布局趟在用」的 `content_size(_of)`——measure_stack 内联后遗留）；方法论坑：前缀掩码 + props 键必须 pub（字符串解析）。扩展缝盘点基线已记，DisplayList 命令闭集与布局策略闭集是下一步「拓展接口」的明显缺口 |
| 2026-10-25 | [.zypui 结构迁移：控件层 + gallery + demo 全量落地](2026-10-zypui-structure-migration.md) | 已接；三处 Rust 结构树各归一份 `.zypui`（树全等 parity 验收后删旧码）；语言层补 `Val::Raw` 结构值通道 + `filter`/`blend` bare attr；十条真红过的缝（实参必须 `{}` / 类序即匹配序 / for 内 hover 显式 key / 属性式 `<for>` / `&&` 非 `and` / 属性顺序非语义 / build-deps 段位…）；语言规范入 `docs/developer/07-declarative-ui.md` |
| 2026-10-25 | [严格测试与压力门禁：6 个真问题](2026-10-25-strict-tests-and-pressure-gates.md) | 已接；fx 确定性压力测试（段数 2f+1 文档错 / `plan_offscreens` 去重洞）+ 分段回放守恒律；D3D11 合成趟丢混合（144 命令压力场景抓出，Multiply+SoftLight 恰 60）+ 像素门禁；D3D12 内容趟对全 0 RT 混合（纯黑 + 计数器说谎，`into_fresh_rt` 修）；**`ID3D12InfoQueue` 是进程级单例** ⇒ 并行 drain 偷消息（假红之外更危险的是**假绿**）⇒ 两侧 harness 加进程级串行锁 + singleton 常驻测试；**SoftLight 公式死区**（0.25<b<0.5 ⇒ B=0 纯黑，产品级 bug）与两端 UNORM8 量化差 1 LSB（D3D11 128 vs D3D12 127）叠加造成 parity 差 87 ⇒ 换 Pegtop 标准式 + fxc 重编 20 entry ⇒ parity 转绿。全量 855 通过 / 0 失败。**Vulkan 同型审计未过真 GPU**（现已补 WSLg/llvmpipe 门禁；物理 GPU 仍待复核） |
| 2026-10-24 | [传参校验门禁：Close() 回 0x80070057 那一族](2026-10-24-command-validation-gates.md) | 已接；调试层开着回放每帧、成功路径也抽干队列。D3D12 harness 抓出 4 个存量 bug（resolve 源状态 538 / 空转屏障 525 / PSO 采样数 614 / drain 严重级写反）+ 计数器说谎 + `min()` 守卫漏洞；D3D11 harness 修好「一条消息都读不出」（两段式）+ redraw 少减账 + SRV 卫生（WARNING 每帧刷）。反向对照证明**同一错误不同驱动一个炸一个不炸**。用户贴的冻结链已修，真机 gallery 验证待用户。原记录时 Vulkan 门禁未写（本机无 WSL）；后续验证门禁见本索引上方 Vulkan 条目 |
| 2026-10-24 | [Vulkan 后端补上折射（液态玻璃在 Linux 侧真正生效）](2026-10-24-vulkan-refraction.md) | 已接（**未过真 GPU**）；WGSL 补 `fs_refraction`（与 HLSL `PSRefraction` 逐项对应）+ 渲染器取全屏目标副本（`snapshot_dst`）+ 退化路径；5 条字符串门禁钉住四个已实测的失效模式（1px 遮罩 / bezel 不按半径夹 / 有界楔形偏折 / 色散反向）。**待办**：WSL2 真机跑 gallery glass 路由（本机 wsl 被黑名单挡了） |
| 2026-10-24 | [两处「画面不更新」：陈旧文字成形缓存 + MSAA 折射读错源](2026-10-gallery-stale-text-and-msaa-backdrop.md) | 已修；`TEXT` 不参与布局 ⇒ 改字既不重排也不清 `measure_cache`，而 `paint` 拿**旧** `ShapedText` 算字形、把**新**文字写进 `glyph_quads`（缓存中毒，等 resize 才自愈）→ 未命中时无条件按当前文字重新成形；折射副本取的是**后缓冲**（MSAA 时那是上一帧的解析结果）→ 改取本帧绘制目标，按 `SampleDesc.Count` 分岔；**解析目标的状态必须是 `RESOLVE_DEST`**，用 `RENDER_TARGET` 会录进非法命令 ⇒ `Close()` 回 `0x80070057`、之后每帧 `Reset` 都失败、画面永久冻结（看着像「点导航没反应」） |
| 2026-10-22 | [液态玻璃：位移发散、遮罩压掉效果、几何错位](2026-10-liquid-glass-refraction-fixes.md) | 已修（两轮）；薄棱镜近似在大角度下发散（位移 491px vs 厚度 28px）→ 换有界的楔形偏折；遮罩 30px 把最强的折射乘成 0 → 收窄成 1px；倒角带被圆角半径夹住 → 上 3/4 档是空档；第三块玻璃下面**没有内容**可折射 → 加贯穿整宽的竖条带；标题 Thin/Normal/Thick 与三块相同的玻璃**说谎** → 厚度按档位偏移；**白 @30% 的玻璃压白竖条 ⇒ 玻璃块整个消失** → 竖条取中间调；**CPU 参考光栅器把 alpha 用了两次**（参考图整体偏暗）→ 修混合式 |
| 2026-10-22 | [切语言没反应（LCID 串 ⇒ 每帧重跑协商）](2026-10-language-switch-no-response.md) | 已修；平台层 `MUI_LANGUAGE_NAME` 写成了 `MUI_LANGUAGE_ID`（`0x4`）⇒ 拿到 LCID 串 ⇒ 协商得空表 ⇒ 判据「空表 = 还没协商」失效 ⇒ **每帧**把用户选的语言改回英文；两条缺一不可 |
| 2026-10-20 | [flex 折行：两趟量出两个宽度](2026-10-flex-wrap-two-pass-width.md) | 已修；显式 `width` 的容器折行用父上限而不是自己声明的宽度 ⇒ 行数错 ⇒ 内容叠在一起；端到端门禁直接量 `.zstyle` 驱动的几何 |
| 2026-10-20 | [画廊点了没反应（`DEVICE_RESET` 死设备）](2026-10-gpu-device-reset-dead-clicks.md) | 已通；死设备 ⇒ 每帧空帧 ⇒ 输入照常跑但画面冻结；恢复 = 重建整条 GPU 栈，限流每秒一次；6 条门禁全部不需要真 GPU |
| 2026-10-18 | [空壳控件（只有样式、点了没反应）](2026-10-hollow-controls.md) | 已通；审计出 30 个 key（4 类）；门禁按「扰动世界 + 树指纹」判定；顺带发现无字体布局的门禁看不见裁切 |
| 2026-10-18 | [离屏 pass + 14 档 PS 变体混合](2026-10-linux-vulkan-offscreen.md) | 已通；`未实现混合` 归零；四个 usage/loadOp/initial_layout 坑 + 「RT 装的是命令自己」；对照 Windows 95.79% 逐字节一致 |
| 2026-10-18 | [1:1 像素级达成（sRGB 取舍实测）](2026-10-linux-vulkan-parity.md) | 前 6 大颜色与 Windows 逐字节一致；UNORM 直通是正解 |
| 2026-10 | [Linux 裸 Vulkan 后端（wgpu → ash）](2026-10-linux-vulkan-raw.md) | demo/gallery 跑通；Entry 生命周期是 segfault 根因；14 档 PS 变体未接线（已记账） |
| 2026-10 | [Linux 适配（WSL2 实测）](2026-10-linux-port.md) | 已通；14 档 PS 变体混合模式未接线（已记账） |
| 2026-10 | [GPU 设备挂死（DEVICE_HUNG）](2026-10-gpu-device-hung.md) | 已定案（`ResizeBuffers` 本机不可用） |
| 2026-10 | [Chromium 合成器对照](2026-10-chromium-compositor-reference.md) | 参考 |

以下几份是**任务清单**格式，尚未改写成日志格式：

| 文件 | 内容 |
|---|---|
| [task-resize.md](task-resize.md) | resize 优化（交换链容量制） |
| [task-live-resize.md](task-live-resize.md) | 窗口拖动时实时帧 |
| [task-responsive.md](task-responsive.md) | 响应式断点 + 窄屏 |
| [task-anim-wire.md](task-anim-wire.md) | 动画接线 |
| [task-glyph-cache.md](task-glyph-cache.md) | 字形位图缓存 |

---

## 长期存在的问题

写在这里以免被当成「已完成」：

- **`begin_frame` 每帧 `wait_gpu()` 全等一次。** 根因是顶点/索引只有一份 resident
  缓冲。正解是环形多缓冲（像命令分配器那两槽按帧轮转）。这是唯一会动 GPU 资源生命
  周期语义的改动，要连带验证 resize / recreate，所以在真做过完整验证之前不动。
  详见 [`../developer/05-frame-pipeline.md`](../developer/05-frame-pipeline.md)。

- **D3D12 调试层本机现在可用（2026-10-24 更正）。** 曾记录「两条路都不可用」
  （事后 `EnableDebugLayer` 设备刚建就 HUNG；事前 `D3D12_CREATE_DEVICE_DEBUG`
  访问违例）。本轮 `command_validation` harness 走的是**第三条路**：
  `D3D12GetDebugInterface` → `EnableDebugLayer`（建设备**之前**）+ 挂
  `ID3D12InfoQueue`，9/9 绿、反向对照能读到 ERROR 级消息 —— 旧结论不再成立，
  保留在此以防又把「本机没有调试层」当成既定事实。

- **`docs/win/03-*.md` 缺号。** 原 `win/` 目录本来就是这个状态，没有漏搬。
  `win/02-mica.md` 是**另一个**缺号：它在 2026-11-01 随 DWM 背景材质一起删除
  （见本索引首条的 dev-log），新内容在 `win/07-window-transparent.md`。
  不重编号 04..07 是刻意的——重编号会让所有既有引用（含 dev-log 的历史条目）
  悄悄指错文件。

- **触控/字体的几何判据依赖系统字体。** `gallery/tests/gates.rs` 的 `laid_out`
  用 `zephyra_fx::from_system_font()`（与 `--dump-cpu` 同源）排版，所以「内容一屏
  装得下」这条判据跟**本机字体度量**绑定：换一个行高更大的字体，某族就可能重新
  贴边。取不到系统字体时 `from_system_font` 内部有兜底（退化成旧的无字体度量），
  于是同一份门禁在无字体机器上会**变松**。当前各族的余量：`--dump-cpu` 逐族量
  可见（basics 726/800、status 743/800 最紧）。要摆脱这一点得让 core 支持
  「按字体度量留白」或者给参考页加滚动 —— 都还没做。

- **`cargo clippy --workspace` 现在干净。** 曾记过「`zephyra-vulkan` 12 处警告」，
  本轮实测 `-D warnings` 退出 0（那 12 处已在别轮修掉），本条改为记录现状。

- **`cargo doc --no-deps --workspace` 现在干净。** 曾有两处 broken intra-doc link：
  `zephyra-style/src/anim.rs` 的 `[`curve`]` 与 `zephyra-core/src/filter.rs` 的
  `[`Self::bezel`]`（都指向方法/字段而不是可链接项）。本轮各改一处（`Self::curve` /
  `` `bezel` ``），整个 workspace 文档生成退出 0，剩余 42 条是既有的链接解析警告。
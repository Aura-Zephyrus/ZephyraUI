# 2026-11-01 复选框：三态、组、禁用、键盘、无障碍

> 现象（需求）：`CheckBox` 原本只有「勾 / 不勾」两态，且**键盘完全不可达** ——
> 既没接 `focusable` / `activatable`，也没有半选。本次补齐三态流转、复选框组
> （全选 / 半选聚合）、禁用态、键盘（Space 与 Enter 都切换），并让 UIA 能读到
> Toggle 图案。

## 设计决策（先定，再写码）

| 问题 | 结论 |
|---|---|
| 三态复选框 / 两态布尔复选框 / 复选框组怎么划分 | `CheckBox` 复用，`tristate: true` 开三态（两态是默认）；组另写 `CheckBoxGroup` |
| 半选点一下去哪 | `Indeterminate → Checked`（WinUI / HTML / macOS 主流）。半选是**派生**状态，用户点它表达的是「我要全要」 |
| 状态模型放哪一层 | **全部放 `zephyra-ui` 控件层**，非必要不进 core |
| 状态怎么跨帧留存 | `.zypui` 是**每帧重建**的无状态模板 ⇒ 状态住在**宿主（RS 侧）** |
| 三态开关放哪 | `tristate` 放 `.zypui`（它是**结构**开关），状态值放 RS |
| 半选怎么画 | **自绘横杠**；选中态改用**勾号图标**（`check.svg` 已在图标表） |
| 禁用态算不算本次范围 | 算，一并加 `disabled` |
| 无障碍做到哪一步 | 完整 UIA 接入（原始提供者 + Toggle 图案） |
| 键盘 | Space 与 Enter **都**切换（复用 core 现成的 `ActivationState`，零 core 改动） |
| 组规则 | 组头 = 全选 / 全部不选；点子项只改自己；子项只有 true / false；全不选→全不选，部分→半选，全选→全选；组头半选时点它 → 全选 |

## 落地

| 层 | 内容 |
|---|---|
| `ZephyraUI/src/checkbox.rs` | `CheckState`（三态枚举 + 分类词 + 流转）、`Checkbox`、`CheckboxGroup` |
| `ZephyraUI/src/tests/checkbox_tests.rs` | 18 条单测 |
| `ZephyraUI/ui/controls.zypui` | `CheckBox` 重写 + 新增 `CheckBoxGroup` |
| `ZephyraUI/controls-checkbox.zstyle` | 新样式表（指示器 / 勾号 / 横杠 / 焦点环 / 禁用） |
| `zypui` | 新增 `disabled` 这个**裸属性**（`BARE_ATTRS` + `instantiate`），映射到 core 的 `props::DISABLED` |
| `gallery` | RS 侧绑定（`Gallery::check_group`、`demo_check`）、`--family` 窗口路补齐、3 条新门禁 |
| `zephyra-uia` | Toggle / Invoke 图案（承接 [2026-10-31 条目](2026-10-31-uia-raw-provider-toggle-pattern.md)） |

## 踩到的坑

### 1. `tristate` 是**结构开关**，不是状态值的一半

半选横杠那个节点**只在三态框里存在**。若把它做成「状态决定要不要这个节点」，
每次状态翻转都要增删节点 —— 树在两帧之间形状不同，下游（焦点、命中测试、
无障碍树）全要跟着抖。所以：`tristate` 是**每个实例固定**的结构参数，
状态只换**分类名**（`""` / `checked` / `indeterminate`）。

### 2. 勾号与横杠是**同一个节点的两个形状**，不是两个节点

`check-mark`（带 `check.svg` 图标）是父节点，`check-dash`（自绘 12×2 小条）是
它的子节点，两者都居中于 12×12 的标记框里：半选时勾号被 `icon-color:
transparent` 隐掉、横杠浮上来；选中时反过来。少一个节点就少一处抖动。

### 3. 分类词由 **Rust 侧**产出，模板只当管道

`CheckState::class_word()` 产出 `""` / `"checked"` / `"indeterminate"`，
与 `controls-checkbox.zstyle` 里的 `.checked` / `.indeterminate` **同名**。
分类名是**样式表的词汇**，而样式表与状态模型同属控件层 ⇒ 由模型产出这个词，
模板不需要知道有哪些状态，也就不可能与样式表对不上。
（拼错分类名是**静默失效**：画面上只是没那个态。）

### 4. ⚠ 复选框组的 key **不能**是 Use 的 `key` 属性

zypui 的「实例补丁」（`RootPatch`）会把 Use 的 `key` 打在**组件体的第一个元素**上。
`CheckBoxGroup` 的第一个元素是外层容器 ⇒ 它会拿走组 key ⇒ **容器与组头那颗
复选框同 key**。

按 key 找节点是**深度优先、第一个命中**（core 的 `FocusState::write_state` /
`ActivationState` 都这么找），于是坏掉的是两个**功能**：

- `FOCUSED` 写到外层容器上 ⇒ **焦点环画不到复选框上**；
- 容器没有 `ACTIVATABLE` ⇒ Space/Enter **闩不住** ⇒ 键盘切不动。

这类失效在画面上**完全看不出来**（控件照常显示、鼠标照常能点），只有用键盘
才发现「这颗框按空格没反应」。

所以组 key 改由**实参 `gkey`** 进来：组头那颗复选框的 key 就是 `gkey`（唯一、
可聚焦、可激活），外层容器另用 `grp:` 前缀的装饰性 key。
这与 `PageRoot` / `Section` 用 `id` 实参承载 key 是同一手法。

> 门禁：`the_checkbox_group_header_is_keyboard_reachable` 专门钉这条 ——
> 它会真的去查组头那颗框有没有 `FOCUSABLE` + `ACTIVATABLE`，以及焦点能不能
> 落到它身上（而不是被同 key 的容器截胡）。

### 5. ⚠ 禁用子项**不参与**组头聚合

若把禁用项算进聚合，会出现一个**卡死**的组头：

```text
3 个子项，其中 1 个禁用且未选
  → 聚合：2 选 1（半选）
  → 点组头：全选（跳过禁用项）→ 那 2 个本来就选了，**什么都没变**
  → 组头永远是半选，点它永远没反应
```

那个「点了没反应」在画面上完全看不出来 —— 画面只是一颗不变的头。
把禁用项排除在聚合之外，组头就只反映**可切换**的那批，「全选」也就一定能
走到「全选」态。代价是「组头全选」时那一行禁用项仍是未选 —— 但它是**禁用**的
（画成灰的、点不动），用户看得出它不在这一批里。

### 6. `disabled` 属性在 zypui 里**原本不存在**

core 早有 `props::DISABLED`（命中测试与 `ActivationState` 都认它），但 zypui 的
`BARE_ATTRS` 清单里**没有** `disabled` ⇒ 模板里写 `disabled="{disabled}"`
会被当成未知属性静默忽略 ⇒ 禁用的框照样能点。
补 `BARE_ATTRS` + `instantiate` 两处即可（属性名与其它布尔属性同名，
`BARE_ATTRS` 是「按名字进白名单」的，所以只能加进清单，没有别的接法）。

### 7. gallery 的 `--family` 在**窗口路**没被消费

与当初 `--route` 同款漏：`--family` 只在离屏路解析，窗口路直接忽略 ⇒ 只能开
到第 0 族，验证不了复选框组。补上窗口路解析（未知值退出码 2）。
这类漏的共同特征：**参数解析有两份实现**（离屏 / 窗口），改一份忘另一份。

### 8. 存量红：着色器 mtime 守卫

`shader_assets_are_in_sync` 要求每个 `.bin` 比 `.hlsl` 新。2026-10-30 那次
着色器改动只重编了新入口 `PSBackdropBlur`，其余 19 个 `.bin` 仍是旧产物 ⇒
门禁红。不是本次引入，但门禁红着就不能算「全绿」。
按仓库约定用 fxc 重编全部入口（两侧各 21 个产物，两份 `.hlsl` 逐字节相同）。

## 门禁

```bash
cargo test -p zephyra-ui                       # 复选框 18 条
cargo test -p zephyra-gallery                  # 3 条新增：树上状态 / 组头键盘 / 无障碍树
cargo test -p zephyra-uia                      # 无障碍 33 条 + 端到端 2 条
cargo test --workspace                          # 全量
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

真机（跨进程 UIA 客户端读 gallery）：

```text
Name                         Type       AutomationId      Enabled Toggle
Select all                   50002      group:checks      true    Indeterminate
Enable feature               50002      check:alpha       true    On
Send analytics               50002      check:beta        true    Off
Beta channel                 50002      check:gamma       true    Off
Locked option                50002      check:locked      false   Off
```

组头半选、子项 On/Off、禁用项 `enabled=false` 全部正确。

## 仍未做

- 组**嵌套**（组里再套组）—— 当前是**平铺一层**的结构，模板层面就不支持；
- `RadioButton` / `Segmented` 的键盘激活（`Button` / `CheckBox` 已接，其余未接）；
- UIA **事件**（属性变化 / 结构变化）没接 ⇒ 屏读器只能轮询。

# 声明式结构（.zypui）

`.zypui` 描述「建哪些节点、发哪些分类名、key 是什么」——**只有结构**。外观
全部在 `.zstyle`（见 [04-stylesheets](04-stylesheets.md)），行为（click 之后
干什么）留在 Rust 侧的宿主里。三者的分工：

| 关注 | 写在哪 |
|---|---|
| 结构（节点树、key、分类名） | `.zypui` |
| 外观（颜色、尺寸、间距、状态样式） | `.zstyle` |
| 行为（状态、事件分派、数据） | Rust（`View` + `zypui::Host`） |

仓里有两个完整的真实用户：

- `gallery/`（应用外壳 + 17 个页面，结构来源 `gallery/ui/shell.zypui`）；
- `examples/ui/demo.zypui`（最小示例，六个视图函数的翻译）。

## 编译成二进制，运行时零解析

```text
build.rs: zypuic::compile(&[("ui/demo.zypui", src)]) → OUT_DIR/*.zypui.bin
运行时:   include_bytes! + zypui::decode → RawUi → Ui::build("页名", host, …)
```

诊断（属性名拼错、未知组件、表达式引用不存在的名字）在**编译期**报；运行期
求值是全函数（缺绑定判假、跨类型比较判 false），不炸帧。

组件模板跨文件共享一张组件表：引用外部组件库时，把多个文件**一起**传给
`zypuic::compile`（见 `gallery/build.rs`）。

## 语言

XML/HTML 风格。裸元素四种：`container` / `text` / `image` / `control`（其它
标签名按组件引用处理）。全部展开成裸元素树，没有运行时组件对象。

### key 与分类名

```xml
<control key="{b.name}" class="{press(b.name) ? 'pressed'}" interactive="true">
  <container key="glyph" class="glyph {hover(b.name) ? 'hovered'}" icon="{b.name}"/>
</control>
```

- `key` 是事件路由身份（`&'static`），实例化期求值；
- `class` 按**空白切词**：裸词是静态分类名，`{cond ? 'word'}` 是条件词
  （单臂三元合法，else 缺省空串），`{expr}` 求出整词的是动态词（空串不发）；
- **类序即匹配序**：`.zstyle` 里 `Control.caption-btn.close.hovered` 这种
  多类选择器按 `.zypui` 里 class 属性的词序匹配，两处必须同改。

### 属性白名单（BARE_ATTRS）

不在表内的属性名 zypuic 直接报错——静默忽略比报错更糟：

| 属性 | 去向 | 值 |
|---|---|---|
| `text` / `icon` / `texture` | TEXT / ICON_NAME / IMAGE_TEXTURE | 字符串 |
| `interactive` / `collapsed` | INTERACTIVE / COLLAPSED | 布尔（真值语义） |
| `uv` | IMAGE_SRC | `x,y,w,h` 字面串或四元数列表 |
| `width` / `height` | WIDTH / HEIGHT | 数值——**仅限 `fill:` 开头的 key**（编译期强制） |
| `url` | 宿主 `prop_key("url")` 给的键 | 字符串 |
| `filter` / `blend` | FILTER / BLEND_MODE | **结构值**（见下） |

外观值（颜色/字号/圆角…）**刻意不在表内**：`.zypui` 里写不了外观，这是规矩
不是限制。对应的门禁是「源码/模板零外观」扫描。

### 结构值：`Val::Raw`

滤镜链与混合模式是结构值，表达式语言没有它们的字面量。唯一通道是宿主绑定
返回 `Val::Raw(PropertyValue)`：

```rust
// 宿主绑定
("filter", Val::Raw(PropertyValue::filter(chain))),
("blend",  Val::Raw(PropertyValue::blend_mode(blend))),
```

```xml
<container key="swatch" class="swatch {fill}" filter="{e.filter}" blend="{e.blend}"/>
```

`Raw` 在条件/拼接等其它上下文里按「假」降级——给错类型时画面上少一个效果，
不崩帧。

## 组件模板

```xml
<component name="FxSwatch" params="id caption fill filter blend">
  <container key="{id}" class="fx">
    <children/>
  </container>
</component>
<FxSwatch id="fx-none" caption="对照" fill="accent" …/>
```

- 实参是**表达式**，按调用方作用域逐帧求值；数值/布尔实参必须带 `{}`
  （`value01="{0.65}"`）——不带花括号的整体是字符串字面量；
- `<children/>` 是子内容槽（恰好一个）；模板没写槽位却传了子树是**错**；
- 保留名不能作参数：`key`（实例身份专属通道）/ `class` / `hover` / `press`；
- **实例 key 既是根元素身份，也是模板内所有 `hover` / `press` 内置词的基准**
  （交互态只住组件模板里）；`for` 循环项不是组件实例，逐项状态必须显式给：
  `hover(item.key)`。

## 表达式

字面量（`'串'` / 数 / `true`）、绑定 `@name`、`tr('key')`、字段访问 `e.field`、
四则与比较、`&&` `||` `!`、三元 `cond ? a : b`（单臂合法）。内置调用：
`min / max / range(n) / at(list, i) / hover(k) / press(k) / str(v)`。

字符串只有 `&'static` 一个形状：字面量来自编译产物、动态拼接经 `Interner`
去重——没有「每帧一次泄漏」能藏的地方。宿主侧 `Interner` 要**跨帧复用**
（`OnceLock<Mutex<Interner>>`，见 `gallery/src/ui.rs`）。

## 宿主契约

```rust
impl zypui::Host for App {
    fn binding(&self, name: &str) -> Option<Val>;          // @name 数据面
    fn tr(&self, key: &'static str) -> &'static str;       // i18n 文案表
    fn hovers(&self, key: &str) -> bool;                   // hover 内置词
    fn holds(&self, key: &str) -> bool;                    // press 内置词
}
```

`binding` 的大匹配表是「模板引用了哪些名字」的唯一清单；数据表集中放一个
`data` 模块（`&'static` 常量），页面与事件分派共享同一份 id 清单。

## 验收：树全等 parity

迁移的验收门禁是**逐节点对照**：`.zypui` 实例化的树与旧 Rust 视图函数产出
的树必须 tag / key / 分类名序列 / Local 属性全等（`gallery/tests/zypui_parity.rs`
对 9 路由 × 输入态 × 扰动快照做过全量；demo 侧同法，全绿后脚手架下线）。
属性是按名字取用的映射，**插入顺序不是语义**，对照时按键名排序。

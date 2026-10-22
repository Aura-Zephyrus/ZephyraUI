# 2026-10-22 切语言没反应（LCID 串 ⇒ 每帧重跑协商）

> 现象：在设置页点「中文」，界面毫无变化 —— 切到 English / 日本語 同样没反应。
> 导航、切主题、拖滑块全都正常，**只有语言这一项**是空壳。

## 排查

先按「点击链路」从上往下拆，每一层单独验：

| 层 | 怎么验 | 结果 |
|---|---|---|
| 命中测试 | 在 `laid_out("settings")` 的树上打点，看 `hit.target()` | `Some("zh-Hans")` ✅ |
| 分派 | `Gallery::click("zh-Hans")` | `lang = 1` ✅ |
| 事件配对 | 造 `InputState` 走「按下 → 抬起」 | `lang = 1` ✅ |
| 取文案 | `tr("route.guide")` | `快速开始` ✅ |
| 重建视图 | 比较两棵树的 TEXT 属性 | 23 处变化 ✅ |
| 帧判据 | `FrameCache::frame` | `Updated { dirtied: 57 }` ✅ |

**每一层都是好的。** 于是问题只能落在「这些层之外」—— 而最后两行
（重建视图 / 帧判据）都发生在**应用状态被改过之后**。

于是把探针往前挪一格：不再验「点一下之后状态对不对」，而是验
「点一下之后**下一帧**状态还对不对」。这一验就露馅了：

```
system languages = ["0804", "0409"]
after idle frame: lang=0 locale=en prefs_len=0
after press:      lang=0 locale=en
after release:    lang=0 locale=en
```

`prefs_len=0` 是关键：**协商的结果是一张空表**。

## 排除的假设

- **「命中测试偏了 / 点到别的节点上」** —— 排除。`hit.target()` 返回
  `Some("zh-Hans")`，且是真实布局后的矩形中心。
- **「`click` 里语言分支没接上 / 被 `resolve_route` 抢先」** —— 排除。
  直接调 `click("zh-Hans")` 得到 `lang = 1`；`ROUTES` 里也没有叫
  `en` / `zh-Hans` / `ja` 的路由 id。
- **「段 key 发错了（`seg:zh-Hans` 之类）」** —— 排除。`controls::segmented`
  段 key 就是裸 id（`with_key(*id)`）；`click` 也认 `lang:` 前缀，两种形态都能接住。
- **「文案表缺 `zh-Hans` 的条目」** —— 排除。`tr("route.guide")` 在
  `lang = 1` 时返回 `快速开始`。
- **「帧没重绘」** —— 排除。`FrameCache::frame` 返回 `Updated` 且
  `dirtied: 57`；diff 对 TEXT 变化标的是 `Dirty::Paint`。
- **「平台层给的系统语言是 `zh-CN`/`en-US`，只是协商逻辑写错了」** ——
  **这是真正的根因，但方向相反**：平台层给的**不是**语言名。

## 结论

**两个 bug 叠在一起，缺一个都不会出现这个症状。**

### 1. 平台层：`MUI_LANGUAGE_NAME` 的取值写错了

`zephyra-platform-win/src/locale.rs` 自声明了这个 Win32 常量，写的是 `0x4`。
而 `windows` crate 0.62 里：

```
MUI_LANGUAGE_ID   = 4
MUI_LANGUAGE_NAME = 8
```

**`0x4` 是 `MUI_LANGUAGE_ID`。** 两者只差一位，传错**不报错**，只是把
「语言名」换成「LCID 串」。实测本机（Windows 11，zh-CN 系统）：

| `dwflags` | `GetUserPreferredUILanguages` 返回 |
|---|---|
| `0x4`（当时写的） | `["0804", "0409"]` |
| `0x8`（`MUI_LANGUAGE_NAME`） | `["zh-CN", "en-US"]` |

### 2. 应用层：协商的「只做一次」判据是错的

`Gallery::on_input` 原来这样写：

```rust
if self.prefs.is_empty() && !state.languages.is_empty() {
    self.prefs = Preferences::from_owned(&state.languages);
    self.lang = Self::pick_initial_language(&self.prefs);
}
```

注释写的是「只记**第一次**收到的」，但 `prefs.is_empty()` 当不出这个判据 ——
**协商完全可能得到空表**：宿主给的语言串不是合法 BCP 47 标签时
（`"0804"` 的主语言子标签是数字，`Tag::parse` 直接拒绝），
`Preferences::from_owned` 会把每一项都丢掉。

于是「空表」同时意味着「还没协商」和「协商了但没结果」，用同一个判据的
后果是**每帧重跑协商**。而每帧协商的结果是「回落 `SUPPORTED` 第一项」
⇒ **每一帧都把用户刚选的语言改回英文**。

关键在于顺序：`on_input` 里 click 在**前**、协商在**后**。所以

1. 抬起那一帧：click 生效，`lang = 1`；
2. 同一帧末尾：协商重跑，`lang = 0`。

**点击真的生效了**，只是同一次调用结束时被改回去 —— 于是无论怎么回读
状态都只看得到旧值（`en`），而画面上也确实一个字没变。
这就是「点了没反应」而**没有**任何异常输出的原因。

修法：协商的「只做一次」判据必须**与协商结果无关**，于是加了一个独立
标志 `lang_negotiated`；同时手动切语言时也把它置真 —— 用户的选择
优先于系统协商（`prefs_for_current` 的文档里本来就是这么写的）。

### 为什么两条缺一不可

- 只修平台层：协商会成功（表非空）⇒ `prefs.is_empty()` 恰好也能当判据 ⇒
  症状消失。**但**那个判据仍然是错的：只要系统语言给的不是合法标签
  （别的平台的桩实现、未来的格式变更），症状原样复现。
- 只修应用层：手动选择不再被覆盖，但**初始语言**永远是英文 ——
  中文系统的用户打开应用看到的是英文，还是「语言没生效」。

## 门禁

| 门禁 | 钉住什么 |
|---|---|
| `zephyra-platform-win` `flags_are_pinned` | 常量必须是 `0x8`，且明确断言**不是** `0x4` |
| `zephyra-platform-win` `system_locale_readable_on_real_machine` | 真机返回的每项**首字符必须是字母**（LCID 串非空且长度也对，只查非空查不出来） |
| `gallery` `a_manual_language_choice_survives_the_system_negotiation` | 用**不合法**的系统语言串走完整的「按下 → 抬起 → 再走 5 帧」，断言 `lang` 仍是 1 且文案确实是中文 |
| `gallery` `the_system_language_is_negotiated_at_most_once` | 协商**成功**时也只跑一次：用户切到英文后，后续帧带系统语言不得改回去 |

两条 gallery 门禁都验过「去掉修复就红」：
把判据改回 `self.prefs.is_empty()` ⇒ 第一条红（`left: 0`）；
把判据整个去掉 ⇒ 两条都红。

## 记账

- `docs/developer/i18n.md` §3.1 里那个 `MUI_LANGUAGE_NAME = 0x4`
  同样是错的（还顺带把 `MUI_LANGUAGE_ID` 说成了 `0`），一并改了。
- `zephyra-platform` 的 trait 文档里补了「`0x8` vs `0x4` 只差一位、
  传错不报错」的警告 —— 契约本来就写着「BCP 47 标签的有序表」，
  错的是实现，但下一个实现者会先看契约。

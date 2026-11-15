//! 开关族的状态模型 —— 开关（[`ToggleSwitch`]）与单选组（[`RadioGroup`]）。
//!
//! # 与 [`crate::checkbox`] 的分工
//!
//! 复选框是**三态**（未选 / 选中 / 半选），开关是**两态**（关 / 开）。
//! 半选是「组里选了一部分」这件事的**派生态** —— 开关没有「组」，
//! 于是连半选这个值都不存在。用 `bool` 而不是复用 [`crate::CheckState`]：
//! 让类型把那个非法状态（既开又半）删掉。
//!
//! | 关注 | 住在哪 |
//! |---|---|
//! | 状态**值**与流转**规则** | 本模块（Rust） |
//! | 状态**长什么样**（节点树、分类名） | `ui/controls.zypui` |
//! | 状态的**颜色与尺寸** | `controls-toggle.zstyle` |
//! | 状态的**存在哪**（跨帧留存） | 宿主应用 |
//!
//! # 一比一复刻的蓝本
//!
//! 开关的外观与交互按 Kotlin 原版 `LiquidToggle.kt`（Apache-2.0）：
//! 两态开关 + 可拖动的把手 + `Role.Switch` 的无障碍语义。
//! 蓝本里的**玻璃装饰参数**（高光 / 外投影 / 内阴影）不进本模块 ——
//! 那是**外观**，归 `.zstyle` 与渲染管线。
//!
//! # 单选组的「组状态是派生的」
//!
//! 单选组**不存**一个「组里选了几项」的计数：它只存**哪一项被选中**
//! （至多一项）。选中一项 ⇒ 其余全部取消 —— 这条规则在本模块，
//! 画面上看起来像「点一下就换了个圆点」。

/// 开关「开」时的状态分类词。
///
/// 与 `controls-toggle.zstyle` 里的 `.on` 规则同名。
/// 两处必须同改（样式表拼错分类名是**静默失效**：画面上只是没那个态）。
///
/// # 为什么不是 `checked`
///
/// 复选框用的是 `checked`，开关用的是 `on` —— 因为两者在无障碍树里是
/// **不同的角色**（复选框是 CheckBox，开关是 Switch），共用一个词会让
/// 「按分类名反查角色」变得有歧义。
pub const CLASS_ON: &str = "on";

/// 单选钮「选中」时的状态分类词。
///
/// 与 `controls-toggle.zstyle` 里的 `.checked` 规则同名。
/// 与复选框的 `checked` **同一个词** —— 因为在 UIA 眼里两者都是
/// 「这个项被选中了」，差别在角色（RadioButton vs CheckBox），不在状态词。
pub const CLASS_CHECKED: &str = "checked";

/// 把手（knob）演示里"行程"的常量：轨道 64、内边距 2、把手 40 ——
/// `64 - 2*2 - 40 = 20px`。与 `controls-toggle.zstyle` 的度量对齐，
/// 两处同改时这条常量会被 `knob_travel_matches_the_stylesheet_geometry` 钉住。
pub const KNOB_TRAVEL: f32 = 20.0;

use std::sync::OnceLock;

use zephyra_bezier::{Bezier, EaseTable};
use zephyra_core::filter::Material;

/// 按压时把手的目标缩放（原版 `pressedScale = 1.5`）。
pub const KNOB_PRESSED_SCALE: f32 = 1.5;

/// 点击与拖动的分界（DIP，与 [`KNOB_TRAVEL`] 同一单位）。
///
/// # 为什么原版没有这个数
///
/// 原版 `DragGestureInspector.kt` 的判据是 `didDrag = dragAmount.x != 0f`
/// —— **只要有位移就算拖过**。它在触屏上成立，因为「点一下」不产生任何移动
/// 事件。桌面鼠标 / 触摸板每次点按都带 1–3 物理像素的抖动，于是点按一律被
/// 判成「拖了一下」；而拖拽提交只在越过半程时才改变状态，症状恰好是
/// **点开关不切换、把手抖一下又弹回原位**。
///
/// 判据因此改成「位移大到足以是一次拖动」：净位移越过本阈值之前，
/// 把手**完全不动**；越过那一步只应用**超出阈值的那部分**
/// （Compose `awaitTouchSlopOrCancellation` 的 `overSlop` 也是这么来的：
/// slop 被吞掉，不进入拖动量）。
///
/// 4 DIP = 行程（20 DIP）的 20%：点击抖动（实测 ≤2 DIP，125% 缩放下
/// 3 物理像素 = 2.4 DIP）被吞掉，而真想拖的人会移动远超 4 DIP。
/// 与 [`KNOB_TRAVEL`] 一样，改这里的数必须同时看样式表里的轨道 / 把手尺寸。
pub const KNOB_DRAG_SLOP: f32 = 4.0;

/// 把手位移 tween 的时长（ms）。
///
/// 官方「直接入口（快入）」：缓动 `cubic-bezier(0,0,0,1)`，可作用属性
/// 「位置 / 刻度 / 旋转」，给 167 / 250 / 333 ms 三档；本仓取中档 250 ms。
/// 见 `docs/developer/animation.md` §1。
pub const FRACTION_TWEEN_MS: f32 = 250.0;

/// 250 ms「快入」缓动表的单例（`cubic-bezier(0,0,0,1)`，251 采样点）。
/// 每帧只做索引 + 线性插值，零 `powf`、零牛顿迭代。
static FRACTION_EASE_TABLE: OnceLock<EaseTable> = OnceLock::new();

/// 250 ms「快入」预计算缓动表。
fn fraction_ease_table() -> &'static EaseTable {
    FRACTION_EASE_TABLE
        .get_or_init(|| EaseTable::new(FRACTION_TWEEN_MS, Bezier::new(0.0, 0.0, 0.0, 1.0), 251))
}

/// 位移/按压弹簧的角频率（ω = √stiffness，stiffness = 1000）。
const OMEGA_1000: f32 = 31.6228;
/// 把手缩放弹簧的角频率（stiffness = 250）。
const OMEGA_250: f32 = 15.8114;
/// 收敛阈值（visibilityThreshold；原版 `0.001`）。
const SNAP: f32 = 0.001;

/// 弹簧积分的最大子步（秒）。显式积分的稳定上限是 `2/ω`，最快的一根
/// （√300）也只有 ≈116 ms；取 4 ms 留足余量。详见 [`spring_step`]。
const MAX_SPRING_STEP: f32 = 0.004;

/// 子步数上限（防住异常大的 dt 把一帧拖成几万次循环）。按 [`MAX_SPRING_STEP`]
/// 算，256 步覆盖 1.024 s —— 更长的 dt 已经由 `advance` 的「一步到位」分支接管。
const MAX_SPRING_STEPS: f32 = 256.0;

/// 一个开关（两态：关 / 开）。
///
/// 存的是**状态**，不是外观 —— 外观在 `.zstyle`，结构在 `.zypui`。
/// 它存在的意义与 [`crate::Checkbox`] 一样：让宿主把「这个开关的 key、
/// 标签、状态」放在一起，于是点击分派只需按 key 找到它。
///
/// # 按压动画（一比一复刻 `DampedDragAnimation`）
///
/// 原版（Kotlin）按下时：`pressProgress → 1`（spring damping=1, stiffness=1000，
/// 临界阻尼快弹簧），松开回 0。玻璃视觉全部挂在这个进度上：
///
/// - 高光：`Highlight.Ambient.copy(alpha = progress)` —— 松开时没有高光，
///   按住时渐显；
/// - 内阴影：`InnerShadow(radius = 4dp * progress, alpha = progress)`；
/// - 表面白罩：`White.copy(alpha = 1 - progress)`（本仓没有 surface 层，暂缺）。
///
/// 本仓没有协程，进度由宿主**每帧推进**：[`Self::set_pressed`] 在按下通道
/// 变化时调，[`Self::advance`] 在每帧的时钟回调里调。趋近用**指数逼近**
/// 临界阻尼弹簧的包络（时间常数 1/ω，ω=√stiffness≈31.6/s）—— 对"快弹簧"
/// 的观感逐帧等价。
///
/// 松手后不是立刻回落：原版 `release()` 会先等 `value` 收敛到 `valueRange`
/// 的 2.5% 以内，再让 pressProgress/scale 回去。本仓用 `release_pending`
/// 表达这一段保持期；把手到位后，按压弹簧才开始回落。
///
/// # 材质（Solid 默认 / Glass 液态玻璃）
///
/// `material` 决定**行为**，不只是外观：
/// - `Solid`（默认）：把手位移走 250 ms「快入」tween（与玻璃同一条非线性曲线），
///   `press` / 缩放瞬变；轨道色由 `.zstyle` 静态双类（`.switch-track` /
///   `.switch-track.on`）定义，切换瞬变；
/// - `Glass`：维持复刻的 iOS LiquidToggle 行为（fraction tween / press 缩放 /
///   白罩 / 透镜 / 装饰，外观由绑定驱动）。
///
/// 见 [`Self::with_material`] 与 `controls.zypui` 的注释。
/// # 把手动画（对照 `DampedDragAnimation.kt` / `LiquidToggle.kt`，一处特性 + 一处偏离）
///
/// 原版用五根弹簧驱动把手与指示器，本仓的映射（`advance` 在每帧的时钟回调里
/// 调用，`dt_ms` 是距上一帧的毫秒数）：
///
/// | 原版 | 形状 | 本仓字段 |
/// |---|---|---|
/// | `valueAnimationSpec` | 官方「快入」（`cubic-bezier(0,0,0,1)`）250 ms tween | `fraction`（见下；位置属性的中档） |
/// | `pressProgressAnimationSpec` | 1（临界） | 1000 | `press`（指数包络） |
/// | `scaleXAnimationSpec` | **1（临界）** | 250 | `scale_x` + `scale_x_vel` |
/// | `scaleYAnimationSpec` | **1（临界）** | 250 | `scale_y` + `scale_y_vel` |
/// | `velocityAnimationSpec` | — | 300 | **未复刻**（理由见下） |
///
/// - `fraction`：`0 = 关`，`1 = 开`。与 `checked` **解耦** —— 状态立刻翻转，
///   画面渐进。走官方「直接入口（快入）」（见 [`FRACTION_TWEEN_MS`]）：
///   `u ∈ [0,1]` 时 `progress = 3·u^(2/3) − 2·u`（`cubic-bezier(0,0,0,1)` 的
///   x = t³ 退化，y = 3t² − 2t³）。端点精确、单调、先快后慢；画面渐进的位移
///   与轨道色（lerp fraction）完全同步。
///   中途被打断（快速来回切）时**从当前 fraction 重新计时** —— 位移从原位开始
///   整段重来，不回退；
/// - `press`：同一临界弹簧（原版 `pressProgressAnimation`）；
/// - `scale_x` / `scale_y`：**临界阻尼**（ζ=1）。原版 ω=√250 的 ζ=0.6/0.7
///   欠阻尼会到位后先挤扁再撑出略高于 1 的「过冲」—— 用户验收时明确要求
///   「放大 → 移动 → 缩小」的线条，到位末端不允许再突然撑宽。刚度不变、
///   只把阻尼比改为 1；
/// - `velocity`：**整体删除**。原版 `layerBlock` 的速度拉伸（`scaleX /= 1 −
///   clamp(v/50 · 0.75)`、`scaleY *= 1 − clamp(v/50 · 0.25)`）吃的是
///   VelocityTracker——**手**的读数。本仓按解析导数时，拖拽期 `fraction`
///   由 Move 直接写入、`velocity = (fraction − prev) / dt` 恒为 0，
///   **永远**只有弹簧自主推进（单击 / 程序切换）那一刻喂出非零值。
///   也就是说这项拉伸只存在于「没有人手」的位移里，正是用户判为
///   「莫名其妙变长再变正常」的那一项；指纹拖动因从来不吃这一项，
///   拖拽的手感从来就是「正常」的。限制到手势分支等于留一个全程不触发
///   的死分支，所以按用户验收删除而不是门控。
/// - 视觉位移 = `fraction * KNOB_TRAVEL`，缩放以把手**自己的中心**为原点
///   （core 的 `scale-x` / `scale-y` 语义）—— 都不参与布局与命中测试。
///
/// # 拖拽手势（一比一复刻 `DampedDragAnimation.kt` 的 pointerInput）
///
/// 原版在按下时开始追踪拖拽：
///
/// - `Down`：记录起始位置，进入拖拽模式；
/// - `Move`：按增量累加 `fraction`（钳制到 0..1），**不走弹簧** —— 拖拽是用户直接操控；
/// - `Up`：退出拖拽模式。**真的拖过**才按 `fraction >= 0.5` 提交 `checked`
///   （原版 `if (targetValue >= 0.5f) 1f else 0f`）；只是点一下的话，
///   `checked` 由宿主的激活路径翻转 —— 本仓 `View::click` 是鼠标 / 键盘 /
///   UIA 的唯一漏斗，模型再翻一次就是双翻转（症状是「点了没反应」）。
///
/// **唯一的偏离**是 `did_drag` 的判据：「有过位移」换成了「净位移越过
/// [`KNOB_DRAG_SLOP`]」。理由与取值见那个常量的文档 —— 原版的判据在
/// 触屏上等价于「有没有拖动」，在桌面上不等价。
///
/// 宿主在 `on_input` 里把 `pointer_events` 传给 [`Self::process_pointer_events`]，
/// 由它维护拖拽状态并更新 `fraction`。
///
/// **不复刻的部分**（写明排除）：原版 `layerBlock` 里按 `scale(2/3→0.75, 0→0.75)`
/// 把「轨道自身的离屏副本」画进把手（`CombinedBackdrop`）—— 本仓把离屏源
/// 只有节点自己这一份，跨兄弟层采样需要第二条 slot 管线，记为已知缺口。
#[derive(Debug, Clone, PartialEq)]
pub struct ToggleSwitch {
    key: &'static str,
    /// 标签的**语义 key**（不是文案）。
    ///
    /// 与 [`crate::Checkbox`] 同纪律：控件层走 i18n 通道 —— 模板内部 `tr()`
    /// 翻译它。字段名与 `label_key()` 方法名一起改，是为了让**编译器**把
    /// 所有旧调用点指出来。
    label_key: &'static str,
    /// 材质选型（默认 [`Material::Solid`]，与按钮一致：未声明 = 纯色）。
    ///
    /// - `Solid`：把手位移走 **250 ms「快入」tween**（与玻璃同一条非线性曲线），
    ///   但 `press` / 缩放瞬变，且不使用白罩 / 透镜 / 装饰。轨道色由
    ///   `.zstyle` 的 `.switch-track` / `.switch-track.on` 静态双类定义
    ///   （切换即瞬变，无颜色过渡）。`.zypui` 未传 `material` 时就是它；
    /// - `Glass`（液态玻璃）：维持复刻的 iOS LiquidToggle 行为（fraction tween /
    ///   press 缩放 / 白罩 / 透镜 / 装饰，外观由绑定驱动）。
    ///
    /// `<ToggleSwitch material="glass"/>` —— 与按钮同一个枚举、同一个词汇表。
    material: Material,
    /// 开（`true`）还是关（`false`）。
    checked: bool,
    disabled: bool,
    /// 按压**目标**（宿主在按下 / 松开时设）。
    pressed: bool,
    /// 按压**进度**（0 = 松开，1 = 按住；向 `pressed` 指的目标指数趋近）。
    ///
    /// `f32` 让 `PartialEq` 变成逐位比较 —— 进度每帧都变，属性 diff 会在
    /// 动画期间每帧重建这一条（正确行为：画面在动）。静止时是精确的
    /// `0.0` / `1.0`，diff 稳定，稳态零重建不受影响。
    press: f32,
    /// 把手位置（`0 = 关`，`1 = 开`）。向 `checked ? 1 : 0` 走 Direct 250 ms 的
    /// tween（官方「快入」）。
    ///
    /// 与 `checked` 解耦的理由见类型文档：状态立刻翻转，画面渐进。
    /// 静止时是精确的 `0.0` / `1.0`。
    fraction: f32,
    /// tween 开始时的 fraction（即发布位移的数学起点）。`checked` 中途改回来
    /// 时，tween 从**当前** fraction 重启 —— 这是用户验收的 interruption 语义
    /// （从当前位置完整走 250 ms），不是从旧起点重播也不是压缩时间走。
    fraction_from: f32,
    /// tween 的走向目标（`checked ? 1 : 0`）。与当前 `fraction_target` 不一致时
    /// 重启 tween。
    fraction_target: f32,
    /// tween 已走的毫秒数；到 [`FRACTION_TWEEN_MS`] 时收敛到 `fraction_target`
    /// 并清零。
    fraction_elapsed_ms: f32,
    /// 是否处于位移 tween 中（用于 `is_animating` 与中断重启判据）。
    fraction_tweening: bool,
    /// 把手 X 缩放弹簧（ζ=1，√250）—— 临界阻尼，单调无过冲。
    scale_x: f32,
    /// `scale_x` 的弹簧速度。
    scale_x_vel: f32,
    /// 把手 Y 缩放弹簧（ζ=1，√250）—— 临界阻尼，单调无过冲。
    scale_y: f32,
    /// `scale_y` 的弹簧速度。
    scale_y_vel: f32,
    /// 是否正在拖拽（按下且鼠标移动了）。
    dragging: bool,
    /// 拖拽期间鼠标**真的动过**（原版 `didDrag`）。
    ///
    /// 判据是「净位移越过 [`KNOB_DRAG_SLOP`]」而不是「有没有过位移」：
    /// 桌面指针的点击抖动必须被吞掉（理由见常量文档）。越过阈值之后
    /// 一直为真，且不再清除 —— 拖出去再拖回原点仍算拖过。它决定松手时走
    /// **拖拽提交**还是「交给宿主的激活路径」。
    did_drag: bool,
    /// 按下点的鼠标 X（DIP）。`did_drag` 的净位移从这里算。
    down_x: f32,
    /// 上一次 Move 的鼠标 X（DIP）。
    ///
    /// 原版按**增量** `dragAmount.x` 累加 `fraction`——拖过端点再拖回来时，
    /// 越界的那部分不会"攒着"。这里保留同一段语义。
    last_x: f32,
    /// 末态保持的**按压释放**状态（原版 `release()`）。
    ///
    /// 原版松手时不会立刻把 pressProgress/scale 放回去，而是先等 `value`
    /// 收敛到 `valueRange` 的 2.5% 以内，再让按压相关弹簧回落。本仓用这一个
    /// 布尔表达同样的语义：`set_pressed(false)` / `set_checked` 触发，等
    /// `fraction` 接近目标后在 `advance()` 里清掉。
    release_pending: bool,
    /// 拖拽完成时的目标 `checked` 状态（由 `process_pointer_events` 的 Up 设置）。
    ///
    /// 原版 `onDragStopped` 里：拖拽时根据 `targetValue >= 0.5` 决定最终状态，
    /// 然后通过 `onSelect` 回调通知宿主。本仓由宿主的 `click` 处理，
    /// 所以这里暂存目标状态，供宿主在 `click` 里读取并应用。
    drag_completed_target_checked: Option<bool>,
}

impl ToggleSwitch {
    /// 新建一个**关**着的开关。
    ///
    /// `key` 是**事件路由身份**（宿主按它把点击分派回来），`label_key` 是
    /// 标签的**语义 key** —— 译文由控件模板内部 `tr()` 从宿主的文案表取。
    pub fn new(key: &'static str, label_key: &'static str) -> Self {
        Self {
            key,
            label_key,
            checked: false,
            disabled: false,
            pressed: false,
            press: 0.0,
            fraction: 0.0,
            fraction_from: 0.0,
            fraction_target: 0.0,
            fraction_elapsed_ms: 0.0,
            fraction_tweening: false,
            material: Material::Solid,
            scale_x: 1.0,
            scale_x_vel: 0.0,
            scale_y: 1.0,
            scale_y_vel: 0.0,
            dragging: false,
            did_drag: false,
            down_x: 0.0,
            last_x: 0.0,
            release_pending: false,
            drag_completed_target_checked: None,
        }
    }

    /// 指定初始状态（把手也**直接**钉在对应位置 —— 启动时不滑动）。
    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self.fraction = if checked { 1.0 } else { 0.0 };
        self
    }

    /// 设为禁用。
    ///
    /// 禁用的开关不响应点击，也不参与键盘激活 —— core 的命中测试与
    /// `ActivationState` 都认 `props::DISABLED`，样式表则把它画成灰的。
    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 直接改禁用态（`push` 之后链式设置用不了 `with_*` 时用它）。
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
    }

    /// 指定材质（默认 [`Material::Solid`]，与按钮一致：未声明 = 纯色）。
    ///
    /// `Glass` 维持 iOS LiquidToggle 复刻行为；`Solid` 保留位移 tween、按压瞬变
    /// ——见类型文档。
    pub fn with_material(mut self, material: Material) -> Self {
        self.material = material;
        self
    }

    /// 当前材质。
    pub fn material(&self) -> Material {
        self.material
    }

    /// 事件路由身份。
    pub fn key(&self) -> &'static str {
        self.key
    }

    /// 标签的**语义 key**（不是译文）。
    pub fn label_key(&self) -> &'static str {
        self.label_key
    }

    /// 现在是开还是关。
    pub fn is_checked(&self) -> bool {
        self.checked
    }

    /// 直接改状态（不做流转）。
    ///
    /// 与 `with_checked` 不同：`checked` 立刻翻转，把手位置**不**钉在
    /// 对应端点 —— 由 [`Self::advance`] 渐进过去（翻转开关时看到
    /// "滑"的动画，而不是瞬移）。
    ///
    /// 原版 `LaunchedEffect(selected)` 会对任何外部状态变化执行
    /// `animateToValue`，其中包含一次 press → hold → release。这里对齐：
    /// 状态真的改变时，按压相关弹簧也会保持到把手收敛，而不是立刻回落。
    pub fn set_checked(&mut self, checked: bool) {
        if self.checked == checked {
            return;
        }
        self.checked = checked;
        self.release_pending = true;
        if self.material == Material::Solid {
            // solid 的 press 瞬变（见 set_pressed），位移则走与 glass 相同的
            // 250 ms 快入 tween（由 advance 推进，宿主按 is_animating 泵帧）。
            self.release_pending = false;
        }
    }

    /// 翻转一次。
    ///
    /// 禁用的开关**不翻转** —— 禁用态下界面不该有任何反应。
    pub fn toggle(&mut self) {
        if self.disabled {
            return;
        }
        self.set_checked(!self.checked);
    }

    /// 是否禁用。
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// 当前状态的分类词（给 `.zypui` 的 `state` 实参）。
    ///
    /// 关 = 空串（不发分类名，样式表管它叫基础态）；开 = [`CLASS_ON`]。
    pub fn class_word(&self) -> &'static str {
        if self.checked {
            CLASS_ON
        } else {
            ""
        }
    }

    /// 设置按压**目标**（宿主在按下通道变化时调）。
    ///
    /// 禁用的开关不进动画 —— 禁用态下界面不该有任何反应。
    ///
    /// 按 `false` 时不会立刻让按压弹簧回落——原版 `release()` 会等
    /// `value` 接近目标后再释放；这里记为 `release_pending`，由 `advance()` 接管。
    pub fn set_pressed(&mut self, pressed: bool) {
        if self.disabled {
            return;
        }
        if self.pressed == pressed {
            return;
        }
        self.pressed = pressed;
        // 按下就取消待释放；松开则进入「等把手到位再回落」的保持期。
        self.release_pending = !pressed;
        if self.material == Material::Solid {
            // solid：press 瞬变（按压不触发缩放/白罩等玻璃反馈）。
            self.press = if pressed { 1.0 } else { 0.0 };
            self.release_pending = false;
        }
    }

    /// 当前按压进度（0..1）。玻璃装饰的强度按它缩放 —— 见类型文档。
    pub fn press_progress(&self) -> f32 {
        self.press
    }

    /// 取走并清除拖拽完成时的目标 `checked` 状态。
    ///
    /// 原版 `onDragStopped` 通过回调通知宿主。本仓由宿主的 `click` 调用此方法：
    /// - 若 `Some(target)`，宿主应把 `checked` 设为 `target`（拖拽提交）
    /// - 若 `None`，宿主按普通点击逻辑翻转 `checked`（点击切换）
    pub fn take_drag_completed_target_checked(&mut self) -> Option<bool> {
        self.drag_completed_target_checked.take()
    }

    /// 把手位置（`0 = 关`，`1 = 开`）。
    ///
    /// `checked` 翻转后它**不**瞬间同步 —— 由 [`Self::advance`] 逼近，
    /// 宿主要把这一帧的值喂给绑定，动画自然成立。静止时精确为
    /// `0.0` / `1.0`（见 `advance` 的阈值钉死）。
    pub fn fraction(&self) -> f32 {
        self.fraction
    }

    /// 把手当前的视觉位移（DIP）—— `fraction * KNOB_TRAVEL`。
    pub fn knob_x(&self) -> f32 {
        self.fraction * KNOB_TRAVEL
    }

    /// 按压时把手 X 缩放 —— `scale-x` 的绑定值。
    ///
    /// 原版 `layerBlock` 的速度拉伸（`scaleX /= 1 − clamp(v/50 · 0.75)`）依赖
    /// VelocityTracker（手的读数）。本仓 **不复刻这一项**（用户验收：单击时
    /// fraction 由弹簧自主推进、白罩莫名其妙被拉长再缩回，松手后像弹性体）。
    /// 拖拽期 `fraction` 由 Move 直接写入，速度天然为 0——拖动根本没有拉伸，
    /// 于是两个路径统一为纯缩放弹簧。
    pub fn knob_scale_x(&self) -> f32 {
        self.scale_x
    }

    /// 按压时把手 Y 缩放 —— `scale-y` 的绑定值（理由见 [`Self::knob_scale_x`]）。
    pub fn knob_scale_y(&self) -> f32 {
        self.scale_y
    }

    /// 动画是否仍在进行（供宿主决定是否继续泵帧）。
    ///
    /// 与 `advance` 的钉死规则配对：收敛后所有量都**精确**落在端点
    /// （`press`/`fraction` → 0/1、`scale_*` → 1.0/1.5），
    /// 所以这里用精确比较 —— 与 steady-state diff「不抖」的判据同源。
    /// 动画中的任何一次宿主 `Wait` 都会冻住把手，宿主应当在泵帧 —— 见
    /// `zephyra_fx::View::wants_frames`。
    pub fn is_animating(&self) -> bool {
        let hold_press = self.pressed || self.release_pending;
        let press_target = if hold_press { 1.0 } else { 0.0 };
        let fraction_target = if self.checked { 1.0 } else { 0.0 };
        let scale_target = if hold_press { KNOB_PRESSED_SCALE } else { 1.0 };
        self.press != press_target
            // 拖拽中 fraction 是用户的手驱动的 —— 位置停在中间不是
            // 「还没收敛」，不强制泵帧（否则按住不动 = 每秒 60 帧空转）。
            || (!self.dragging && self.fraction != fraction_target)
            || self.scale_x != scale_target
            || self.scale_y != scale_target
            || self.scale_x_vel != 0.0
            || self.scale_y_vel != 0.0
    }

    /// fraction 的 250 ms「快入」tween——玻璃与实色共用的唯一位移通道。
    ///
    /// 打断规则与玻璃一致：中途反向切换时从**当前** fraction 重走完整 250 ms。
    fn advance_fraction(&mut self, dt_ms: f32) {
        let fraction_target = if self.checked { 1.0 } else { 0.0 };
        if !self.fraction_tweening && self.fraction == fraction_target {
            return;
        }
        if !self.fraction_tweening || self.fraction_target != fraction_target {
            self.fraction_from = self.fraction;
            self.fraction_elapsed_ms = 0.0;
            self.fraction_tweening = true;
            self.fraction_target = fraction_target;
        }
        self.fraction_elapsed_ms += dt_ms;
        let eased = fraction_ease_table().progress(self.fraction_elapsed_ms);
        self.fraction = self.fraction_from + (self.fraction_target - self.fraction_from) * eased;
        if self.fraction_elapsed_ms >= FRACTION_TWEEN_MS {
            self.fraction = self.fraction_target;
            self.fraction_tweening = false;
        }
    }

    /// 每帧推进按压与把手动画（宿主的时钟回调里调；`dt_ms` 是距上一帧的毫秒数）。
    ///
    /// 四根弹簧/tween各自推进（见类型文档的表）：`fraction` 走 250 ms 的官方
    /// 「快入」 `cubic-bezier(0,0,0,1)` tween；`press` 用临界阻尼的指数包络
    /// （原版 `spring(1f, 1000f)`）；`scale_x` / `scale_y` 用**半隐式欧拉积分**
    /// 的临界阻尼弹簧（ζ = 1；原版 ζ = 0.6 / 0.7，本仓按用户验收改临界，消除
    /// 到位末端的过冲撑宽）。收敛到阈值后**钉死**在端点：不钉的话进度
    /// 永远停在 0.999，属性 diff 每帧都变，稳态零重建被破坏。
    pub fn advance(&mut self, dt_ms: f32) {
        let dt = dt_ms.max(0.0) / 1000.0;
        if dt <= 0.0 {
            return;
        }

        let fraction_target = if self.checked { 1.0 } else { 0.0 };
        // ⚠ Solid（默认）：位移走与玻璃相同的 250 ms 快入 tween；press 与
        // 缩放瞬变（按压不移动，只是状态）。
        if self.material == Material::Solid {
            if self.dragging {
                self.fraction_tweening = false;
                self.fraction_elapsed_ms = 0.0;
            } else {
                self.advance_fraction(dt_ms);
            }
            self.release_pending = false;
            self.press = if self.pressed { 1.0 } else { 0.0 };
            self.scale_x = 1.0;
            self.scale_y = 1.0;
            self.scale_x_vel = 0.0;
            self.scale_y_vel = 0.0;
            return;
        }
        // ⚠ 拖拽期间 fraction 由用户直接设（见 process_pointer_events），tween
        // 必须让位；松手后若未到端点，下面重新从当前位置走完整的一趟。
        if self.dragging {
            self.fraction_tweening = false;
            self.fraction_elapsed_ms = 0.0;
        } else {
            self.advance_fraction(dt_ms);
        }
        // 原版 `release()`：先等 value 收敛到 valueRange 的 2.5% 以内，
        // 再让 pressProgress/scale 回落。
        if self.release_pending && (self.fraction - fraction_target).abs() <= 0.025 {
            self.release_pending = false;
        }
        let hold_press = self.pressed || self.release_pending;
        // 1) 临界阻尼（ζ=1）的两根进度：exponential envelope，与 spring(1, 1000) 等价。
        advance_toward(&mut self.press, if hold_press { 1.0 } else { 0.0 }, dt_ms);

        // 2) 把手缩放弹簧（ζ=1, ω=√250）—— 临界阻尼。原版 ζ=0.6/0.7 到位末端
        //    过冲（先挤扁再反弹），「莫名其妙变长」；用户明确要「放大 → 移动 →
        //    缩小」的线条（见类型文档的表与「velocity」一项的整体删除）。
        let scale_target = if hold_press { KNOB_PRESSED_SCALE } else { 1.0 };
        spring_step(
            &mut self.scale_x,
            &mut self.scale_x_vel,
            scale_target,
            dt,
            OMEGA_250,
            1.0,
            SNAP,
        );
        spring_step(
            &mut self.scale_y,
            &mut self.scale_y_vel,
            scale_target,
            dt,
            OMEGA_250,
            1.0,
            SNAP,
        );
    }
}

impl ToggleSwitch {
    /// 处理指针事件（宿主在 `on_input` 里每帧调用一次）。
    ///
    /// # 行为（一比一复刻 `DampedDragAnimation.kt` 的 pointerInput）
    ///
    /// - `Down` 在开关上：进入拖拽模式，记录按下点。
    /// - `Move` 且正在拖拽：净位移越过 [`KNOB_DRAG_SLOP`] 之后，按**增量** `dx`
    ///   累加 `fraction`（`dx / KNOB_TRAVEL`，钳制到 0..1），与原版
    ///   `fraction + dragAmount.x / dragWidth` 一致 —— 拖过端点再拖回来时，
    ///   越界部分不会"攒起来"突然生效。
    /// - `Up` 且**真的拖过**：按 `fraction >= 0.5` 提交 `checked`，让弹簧把
    ///   `fraction` 推向新目标（0 或 1）。
    /// - `Up` 但没拖（点一下，含点击抖动）：**不碰 `checked`** —— 交给宿主的激活路径
    ///   （`View::click`）。原版在 `onDragStopped` 里用 `if (selected()) 0 else 1`
    ///   自己翻转；本仓的宿主是**唯一的激活漏斗**（鼠标点击、键盘 Space/Enter、
    ///   UIA 都汇到 `click`），模型再翻一次就是**双翻转** —— 症状恰好是
    ///   「按下去有玻璃、松手却什么都没变」。
    ///
    /// 宿主只需把 `InputState.pointer_events` 与 `InputState.scale` 传进来。
    /// 坐标从物理像素转为 DIP（除以 scale），因为轨道几何常量是 DIP 单位。
    ///
    /// # 为什么要检查 `self.pressed`
    ///
    /// `pointer_events` 包含**全窗口**的所有指针事件。只有当宿主判定
    /// 这个开关被按下时（`self.pressed == true`），`Down` 才应开始拖拽。
    /// 否则点击别处再拖动也会错误地触发本开关的拖拽。
    pub fn process_pointer_events(&mut self, events: &[zephyra_fx::PointerInput], scale: f32) {
        if self.disabled {
            return;
        }
        for ev in events {
            let x_dip = ev.x / scale.max(1.0);
            match ev.phase {
                zephyra_fx::PointerInputPhase::Down => {
                    // 只有在开关被按下时才开始拖拽（`pressed` 由宿主同步）。
                    if self.pressed {
                        self.dragging = true;
                        self.did_drag = false;
                        self.down_x = x_dip;
                        self.last_x = x_dip;
                        // 上一段手势若留下了没被宿主的激活漏斗取走的提交目标，
                        // 在这里丢掉：否则下一次点击会拿它当结论，把「点一下」
                        // 变成「应用上一次拖拽的结果」。
                        self.drag_completed_target_checked = None;
                    }
                }
                zephyra_fx::PointerInputPhase::Move => {
                    if self.dragging {
                        if !self.did_drag {
                            // 净位移越过阈值的那一步只应用**超出部分**（`overSlop`）；
                            // 阈值以内把手一动不动 —— 点击抖动不该让把手抖一下。
                            let travel = x_dip - self.down_x;
                            if travel.abs() > KNOB_DRAG_SLOP {
                                self.did_drag = true;
                                let over = travel - KNOB_DRAG_SLOP * travel.signum();
                                self.fraction =
                                    (self.fraction + over / KNOB_TRAVEL).clamp(0.0, 1.0);
                            }
                        } else {
                            let dx = x_dip - self.last_x;
                            // 原版 `fraction + dragAmount.x / dragWidth`：
                            // 增量累加，越界部分在 clamp 时丢掉。
                            self.fraction = (self.fraction + dx / KNOB_TRAVEL).clamp(0.0, 1.0);
                        }
                        self.last_x = x_dip;
                    }
                }
                zephyra_fx::PointerInputPhase::Up => {
                    if self.dragging {
                        self.dragging = false;
                        // 原版 `onDragStopped`：拖拽时根据 `targetValue >= 0.5` 决定最终状态，
                        // 然后通过 `onSelect` 回调通知宿主。本仓由宿主的 `click` 处理，
                        // 所以这里暂存目标状态，供宿主在 `click` 里读取并应用。
                        if self.did_drag {
                            self.drag_completed_target_checked = Some(self.fraction >= 0.5);
                            self.did_drag = false;
                        }
                        // fraction 会由 advance() 的 250 ms 快入 tween 推向新目标（0 或 1）
                    }
                }
                zephyra_fx::PointerInputPhase::Cancel => {
                    // ⚠ **取消不是抬起**：系统抢走了这次交互（Windows 11 的
                    // 三指/四指手势）。这时**不能**提交 —— 用户没抬手。
                    if self.dragging {
                        self.dragging = false;
                        self.did_drag = false;
                        // 取消的那一下**不算数**：不产生提交目标。
                        self.drag_completed_target_checked = None;
                    }
                }
            }
        }
    }
}

/// 指数逼近 `target` 一步（收敛到 [`SNAP`] 后钉死）。
///
/// 与临界阻尼 `spring(1f, 1000f)` 的阶跃响应**包络等价**（临界弹簧的幅值被
/// e^{−ω·t} 包住），对逐帧推进的 UI 而言等价 —— 用它的好处是不需要
/// 速度状态，且永不会过冲。
fn advance_toward(value: &mut f32, target: f32, dt_ms: f32) {
    let dt = dt_ms.max(0.0) / 1000.0;
    // 时间步长异常大（掉帧 / 首帧）时一步到位，别让 e^(-ω·dt) 下溢成 NaN。
    let blend = if dt <= 0.0 {
        0.0
    } else if dt > 0.5 {
        1.0
    } else {
        1.0 - (-OMEGA_1000 * dt).exp()
    };
    *value += (target - *value) * blend;
    if (*value - target).abs() < 0.001 {
        *value = target;
    }
}

/// 半隐式欧拉积分的弹簧（原版 `spring(ζ, stiffness)`；本仓调用方现用 ζ = 1，临界）。
///
/// 与 [`advance_toward`] 的分工：临界弹簧没有过冲，用不到速度状态；需要超调形状的
/// 弹簧必须积分速度，所以这里 x / v 都留在结构体里。收敛到 `snap` 后**钉死**
/// （x = target, v = 0）—— 与 `advance_toward` 同一条稳态规则。
///
/// # 为什么要分子步
///
/// 显式积分的稳定域由 ω 决定（半隐式欧拉约 `dt < 2/ω`）。本仓最快的弹簧是
/// `ω = √250 ≈ 15.8`，临界步长 ≈ 126 ms —— 而宿主在拖拽期间**会睡死**
/// （`is_animating` 故意不把拖拽中的 `fraction` 算作未收敛），下一次输入常带着
/// 130–500 ms 的真实 dt。一步跨过稳定边界，弹簧不是收敛而是**发散**：
/// 实测释放那一帧 `dt=381.6 ms`，`scale_x` 从 1.5 一步跳到 −16.7，把手被翻转
/// 放大到整块空白 —— 看起来就是「按住左右滑动之后动画卡住/消失」。
///
/// 所以固定用 [`MAX_SPRING_STEP`] 的子步积分：物理曲线不变（同一微分方程，
/// 只是步长更小），而任意 dt 都不会越出稳定域。代价一帧最多几百次乘加，
/// 相对一次布局可以忽略。
fn spring_step(x: &mut f32, v: &mut f32, target: f32, dt: f32, omega: f32, zeta: f32, snap: f32) {
    if !x.is_finite() || !v.is_finite() {
        // 已经坏掉的状态（历史帧留下的 NaN/inf）没有恢复价值：钉回目标，
        // 否则下面每一步都会继续是 NaN，`is_animating` 永远为真。
        *x = target;
        *v = 0.0;
        return;
    }
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
    let steps = (dt / MAX_SPRING_STEP).ceil().clamp(1.0, MAX_SPRING_STEPS);
    let h = dt / steps;
    for _ in 0..steps as u32 {
        let acc = -omega * omega * (*x - target) - 2.0 * zeta * omega * *v;
        *v += acc * h;
        *x += *v * h;
    }
    if !x.is_finite() || !v.is_finite() {
        *x = target;
        *v = 0.0;
        return;
    }
    if (*x - target).abs() < snap && v.abs() < snap {
        *x = target;
        *v = 0.0;
    }
}

/// 单选组里的一个选项。
///
/// 与 [`ToggleSwitch`] 一样存**状态**，但「选中」这个状态由**组**决定
/// —— �� [`RadioGroup::select`]，不自己存。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioOption {
    key: &'static str,
    label_key: &'static str,
    disabled: bool,
}

impl RadioOption {
    /// 事件路由身份。
    pub fn key(&self) -> &'static str {
        self.key
    }

    /// 标签的**语义 key**（不是译文）。
    pub fn label_key(&self) -> &'static str {
        self.label_key
    }

    /// 是否禁用。
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// 直接改禁用态。
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
    }
}

/// 一个单选组：**至多一项被选中**。
///
/// # 规则
///
/// - 选中一项 ⇒ 同组**其余全部取消**（这是「单选」的定义，不是附加行为）；
/// - 禁用的选项**选不上**，但**仍留在组里**（画成灰的）—— 与复选框组
///   「禁用项不参与聚合」不同：单选组没有「全选」这个动作，
///   所以没有聚合，也就没有那个会卡死的角落；
/// - 选中项可以被**取消**吗？不能 —— 单选组至少留一项选中（若组非空）。
///   允许清空会让「点已选中的那项」变成「取消选中」，那与 WinUI / HTML /
///   macOS 的单选钮行为都不一致（点已选中的单选钮，它保持选中）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioGroup {
    options: Vec<RadioOption>,
    /// 被选中的那一项的**下标**（`None` = 组是空的）。
    ///
    /// 存下标而不是 key：下标是 `usize`（Copy、无分配），而 key 是
    /// `&'static str` —— 两者都能比，但下标让「取消其余」的循环更快，
    /// 且不依赖 key 的唯一性。
    selected: Option<usize>,
}

impl RadioGroup {
    /// 新建一个空组。
    pub fn new() -> Self {
        Self {
            options: Vec::new(),
            selected: None,
        }
    }

    /// 追加一个选项，返回它以便继续设置（例如禁用）。
    ///
    /// `label_key` 是标签的**语义 key**（译文由控件模板内部 `tr()` 取）。
    pub fn push(&mut self, key: &'static str, label_key: &'static str) -> &mut RadioOption {
        self.options.push(RadioOption {
            key,
            label_key,
            disabled: false,
        });
        self.options.last_mut().expect("刚 push 进去的选项必然存在")
    }

    /// 追加一个选项并返回整个组（链式用法）。
    pub fn with_option(mut self, key: &'static str, label_key: &'static str) -> Self {
        self.push(key, label_key);
        self
    }

    /// 按 key 找选项。
    pub fn option(&self, key: &str) -> Option<&RadioOption> {
        self.options.iter().find(|o| o.key() == key)
    }

    /// 按 key 找选项（可变）。
    pub fn option_mut(&mut self, key: &str) -> Option<&mut RadioOption> {
        self.options.iter_mut().find(|o| o.key() == key)
    }

    /// 选中某一项（其余全部取消）。
    ///
    /// 返回是否真的切了：key 不存在、该项被禁用、**或者它本来就是选中项**
    /// 时返回 `false`。
    ///
    /// # 为什么「本来就是选中项」算「没切」
    ///
    /// 点已选中的单选钮，它**保持选中** —— 这与「取消选中」是两件事。
    /// 返回 `false` 让宿主知道「状态没变」，于是不会重建出一个看起来
    /// 「点了没反应」的界面（那个界面恰恰在提示用户操作失败了）。
    pub fn select(&mut self, key: &str) -> bool {
        let Some(idx) = self.options.iter().position(|o| o.key() == key) else {
            return false;
        };
        if self.options[idx].is_disabled() {
            return false;
        }
        if self.selected == Some(idx) {
            return false;
        }
        self.selected = Some(idx);
        true
    }

    /// 当前被选中的那一项的 key（`None` = 组是空的）。
    pub fn selected_key(&self) -> Option<&'static str> {
        self.selected.map(|idx| self.options[idx].key())
    }

    /// 某一项是不是当前选中的。
    pub fn is_selected(&self, key: &str) -> bool {
        self.selected_key() == Some(key)
    }

    /// 某一项的状态分类词（给 `.zypui` 的 `checked` 实参）。
    ///
    /// 权威口径在**组**这里 —— 选项自己不知道组选中了谁。
    pub fn option_class_word(&self, key: &str) -> &'static str {
        if self.is_selected(key) {
            CLASS_CHECKED
        } else {
            ""
        }
    }

    /// 全部选项。
    pub fn options(&self) -> &[RadioOption] {
        &self.options
    }

    /// 选项个数。
    pub fn len(&self) -> usize {
        self.options.len()
    }

    /// 组里一个选项都没有吗。
    pub fn is_empty(&self) -> bool {
        self.options.is_empty()
    }
}

impl Default for RadioGroup {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../tests/unit/toggle_tests.rs"]
mod tests;

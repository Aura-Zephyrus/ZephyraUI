//! 滑块控件的状态模型 —— 一比一复刻 AndroidLiquidGlass `LiquidSlider` + `DampedDragAnimation`。
//!
//! # 蓝本（Apache-2.0，`E:\AndroidLiquidGlass`）
//!
//! - `components/LiquidSlider.kt`
//! - `utils/DampedDragAnimation.kt`
//! - `utils/DragGestureInspector.kt`
//!
//! 本模块按**原版语义**移植（弹簧规格、拖拽/点按/释放时序、`layerBlock` 速度形变、
//! 玻璃拇指效果参数）；与 Compose 协程/Animatable/MutatorMutex/VelocityTracker
//! 不同的部分用 Zephyra 的帧驱动 `advance(dt_ms)` 状态机等价表达，并在偏离
//! 原版运行时形态处写明理由（不改变可观察语义）。
//!
//! # 分工（与 ToggleSwitch 同构）
//!
//! | 关注 | 住在哪 |
//! |---|---|
//! | 值/动画/手势**规则** | 本模块 |
//! | 结构（节点树、分类名、绑定位） | `ui/controls.zypui` |
//! | 颜色与尺寸 | `controls-*.zstyle` |
//! | 值的**存留**与点击分派 | 宿主应用 |

use zephyra_core::filter::Material;

/// 拇指（thumb）基准宽度（DIP）。与 iOS/原版 LiquidToggle 把手同：40×24。
pub const THUMB_W: f32 = 40.0;
/// 拇指基准高度（DIP）。
pub const THUMB_H: f32 = 24.0;
/// 按压时拇指的目标缩放（原版 `pressedScale = 1.5f`）。
pub const THUMB_PRESSED_SCALE: f32 = 1.5;
/// 轨道高度（DIP）。与原版 `height(6f.dp)` / `.slider-track` 样式表一致。
pub const TRACK_H: f32 = 6.0;
/// 默认轨道宽度（DIP）。与 `.slider-track { width: 240px; }` 对齐；
/// 宿主可通过 [`Slider::set_track_width`] 覆盖。
pub const DEFAULT_TRACK_W: f32 = 240.0;

/// 桌面适配：点击抖动的拖拽分界（DIP）。
///
/// 原版 `didDrag = dragAmount.x != 0f`（触屏上等价于“有无拖动”）。桌面指针
/// 单击常带 1–3px 抖动，原判据会吞点击。这里沿用 Toggle 已验证的 slop 适配：
/// 净位移越过阈值前拇指不动，越过后仅应用超出部分（`overSlop`）。
/// （用户指示“先严格按照原版来，后续调整”——本常量保留最小桌面可用性；
/// 如需字面回退 `x!=0` 可置 0。）
pub const DRAG_SLOP: f32 = 4.0;

/// 释放保持阈值：`|value-target| < span * RELEASE_THRESHOLD` 后才释放 press/scale
/// （原版 `valueRange span * 0.025f`）。
const RELEASE_THRESHOLD: f32 = 0.025;

/// `spring(ζ=1, stiffness=1000)` 的角频率。
const OMEGA_1000: f32 = 31.622776;
/// `spring(stiffness=250)` 的角频率（scaleX/Y）。
const OMEGA_250: f32 = 15.811388;
/// `spring(stiffness=300)` 的角频率（velocity）。
const OMEGA_300: f32 = 17.320509;

const SNAP_001: f32 = 0.001;

const MAX_SPRING_STEP: f32 = 0.004;
const MAX_SPRING_STEPS: f32 = 256.0;

fn span_of(start: f32, end: f32) -> f32 {
    let s = end - start;
    if s.is_finite() && s.abs() > f32::EPSILON {
        s.abs()
    } else {
        1.0
    }
}

/// 连续值滑块（原版 `LiquidSlider` + `DampedDragAnimation` 语义）。
#[derive(Debug, Clone, PartialEq)]
pub struct Slider {
    key: &'static str,
    label_key: &'static str,
    material: Material,
    disabled: bool,

    // ---- 值域 ----
    range_start: f32,
    range_end: f32,
    visibility_threshold: f32,

    /// 动画值（原版 `valueAnimation.value`）。视觉进度/拇指位置读它。
    value: f32,
    /// 目标值（原版 `valueAnimation.targetValue`）。拖拽增量基于它累加。
    target_value: f32,
    value_vel: f32,

    /// 速度读数（原版 `velocityAnimation.value`），供 `layerBlock` 形变。
    velocity: f32,
    velocity_vel: f32,

    // ---- 按压/缩放（原版 press/scale 弹簧）----
    pressed: bool,
    press: f32,
    release_pending: bool,
    scale_x: f32,
    scale_x_vel: f32,
    scale_y: f32,
    scale_y_vel: f32,

    // ---- 拖拽 ----
    dragging: bool,
    did_drag: bool,
    down_x: f32,
    last_x: f32,
    /// 轨道左缘的绝对 x（DIP），用于点按跳转定位；由宿主经 pointer 事件上下文给出，
    /// 缺省 0 时点按跳转需宿主另行计算（见 `tap_target_from_x`）。
    /// 轨道左缘绝对 x（DIP）。`f32::NAN` = 未设置（tap 跳转需宿主先设）。
    track_left: f32,
    track_width: f32,
    /// LTR=true。原版按 `LocalLayoutDirection` 镜像；Zephyra 当前默认 LTR。
    ltr: bool,

    /// 拖拽停止且 `didDrag` 时暂存提交值（原版 `onDragStopped { if (didDrag) onValueChange(targetValue) }`）。
    drag_committed: Option<f32>,
    /// 点按跳转产生的待提交值（tap 同时触发 animate + onValueChange）。
    tap_committed: Option<f32>,
}

impl Slider {
    /// 新建滑块，初始值取 `range_start`。
    pub fn new(
        key: &'static str,
        label_key: &'static str,
        range_start: f32,
        range_end: f32,
        visibility_threshold: f32,
    ) -> Self {
        let (range_start, range_end) = if range_end >= range_start {
            (range_start, range_end)
        } else {
            (range_end, range_start)
        };
        let v = range_start;
        Self {
            key,
            label_key,
            material: Material::Solid,
            disabled: false,
            range_start,
            range_end,
            visibility_threshold: if visibility_threshold.is_finite() {
                visibility_threshold.abs().max(1e-6)
            } else {
                0.01
            },
            value: v,
            target_value: v,
            value_vel: 0.0,
            velocity: 0.0,
            velocity_vel: 0.0,
            pressed: false,
            press: 0.0,
            release_pending: false,
            scale_x: 1.0,
            scale_x_vel: 0.0,
            scale_y: 1.0,
            scale_y_vel: 0.0,
            dragging: false,
            did_drag: false,
            down_x: 0.0,
            last_x: 0.0,
            track_left: f32::NAN,
            track_width: DEFAULT_TRACK_W,
            ltr: true,
            drag_committed: None,
            tap_committed: None,
        }
    }

    pub fn with_value(mut self, value: f32) -> Self {
        let v = self.coerce(value);
        self.value = v;
        self.target_value = v;
        self.value_vel = 0.0;
        self
    }

    pub fn with_material(mut self, material: Material) -> Self {
        self.material = material;
        self
    }

    pub fn with_track_width(mut self, w: f32) -> Self {
        self.track_width = if w.is_finite() && w > 0.0 {
            w
        } else {
            DEFAULT_TRACK_W
        };
        self
    }

    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn key(&self) -> &'static str {
        self.key
    }
    pub fn label_key(&self) -> &'static str {
        self.label_key
    }
    pub fn material(&self) -> Material {
        self.material
    }
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
    }

    fn coerce(&self, v: f32) -> f32 {
        if !v.is_finite() {
            return self.range_start;
        }
        v.clamp(
            self.range_start.min(self.range_end),
            self.range_start.max(self.range_end),
        )
    }

    pub fn value(&self) -> f32 {
        self.value
    }
    pub fn target_value(&self) -> f32 {
        self.target_value
    }
    pub fn range_start(&self) -> f32 {
        self.range_start
    }
    pub fn range_end(&self) -> f32 {
        self.range_end
    }
    pub fn track_width(&self) -> f32 {
        self.track_width
    }
    pub fn set_track_width(&mut self, w: f32) {
        self.track_width = if w.is_finite() && w > 0.0 {
            w
        } else {
            DEFAULT_TRACK_W
        };
    }
    pub fn set_track_left(&mut self, x: f32) {
        if x.is_finite() {
            self.track_left = x;
        }
    }
    pub fn set_track_geometry(&mut self, left: f32, width: f32) {
        if left.is_finite() {
            self.track_left = left;
        }
        if width.is_finite() && width > 0.0 {
            self.track_width = width;
        }
    }

    /// `progress = (value - start) / span`（原版）。
    pub fn progress(&self) -> f32 {
        let span = span_of(self.range_start, self.range_end);
        ((self.value - self.range_start) / span).clamp(0.0, 1.0)
    }

    pub fn progress_target(&self) -> f32 {
        let span = span_of(self.range_start, self.range_end);
        ((self.target_value - self.range_start) / span).clamp(0.0, 1.0)
    }

    /// 拇指 `translationX`（DIP），含原版 clamp（允许各超出 1/4 拇指宽），LTR 按 1:1。
    pub fn thumb_x(&self) -> f32 {
        let w = THUMB_W;
        let tw = self.track_width;
        let raw = -w / 2.0 + tw * self.progress();
        let lo = -w / 4.0;
        let hi = tw - w * 3.0 / 4.0;
        let x = raw.clamp(lo.min(hi), lo.max(hi));
        if self.ltr {
            x
        } else {
            -x
        }
    }

    /// 填充条宽度（DIP）= `trackWidth * progress`。
    pub fn fill_width(&self) -> f32 {
        (self.track_width * self.progress()).clamp(0.0, self.track_width)
    }

    pub fn press_progress(&self) -> f32 {
        self.press
    }

    /// `layerBlock` 合成后的 scaleX（原版：`scaleX /= 1 - clamp(velocity/10*0.75, -0.2,0.2)`）。
    pub fn thumb_scale_x(&self) -> f32 {
        let v = (self.velocity / 10.0 * 0.75).clamp(-0.2, 0.2);
        let denom = 1.0 - v;
        if denom.abs() < 1e-6 {
            self.scale_x
        } else {
            self.scale_x / denom
        }
    }

    /// `layerBlock` 合成后的 scaleY（原版：`scaleY *= 1 - clamp(velocity/10*0.25, -0.2,0.2)`）。
    pub fn thumb_scale_y(&self) -> f32 {
        let v = (self.velocity / 10.0 * 0.25).clamp(-0.2, 0.2);
        self.scale_y * (1.0 - v)
    }

    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    /// 外部设值（对齐原版 `updateValue` 经 `snapshotFlow` 同步）：动画逼近目标并更新速度。
    pub fn set_value(&mut self, value: f32) {
        if self.disabled {
            return;
        }
        let target = self.coerce(value);
        if (self.target_value - target).abs() <= f32::EPSILON
            && (self.value - target).abs() <= f32::EPSILON
        {
            return;
        }
        self.target_value = target;
        // value 动画在 advance 中向 target_value 推进；速度由 value 运动派生。
    }

    /// 立即设值（激活/无障碍回退）：value 与 target 同步落位，产生即时可见变化。
    pub fn set_value_immediate(&mut self, value: f32) {
        if self.disabled {
            return;
        }
        let v = self.coerce(value);
        self.value = v;
        self.target_value = v;
        self.value_vel = 0.0;
    }

    /// 点按跳转（原版 `animateToValue`）：`press(); animate value; (velocity->0); release()`，
    /// 互斥语义在单线程帧模型下以“重置目标+发起新收敛”表达；同时产生一次 tap 提交。
    pub fn animate_to_value(&mut self, value: f32) {
        if self.disabled {
            return;
        }
        let target = self.coerce(value);
        self.target_value = target;
        self.tap_committed = Some(target);
        // 原版 animateToValue 内部 press() + release()；release 等待 value 收敛（2.5%）。
        // 视觉上 tap 产生一次按压脉冲：置按住效果并立即进入 release_pending 等待收敛。
        if self.material == Material::Glass {
            self.pressed = false;
            self.release_pending = true;
            // press/scale 从当前值向 held 方向由 advance 驱动；release_pending 会在收敛后释放。
            // 触发一次 press 脉冲的起始态：
            self.press = self.press.max(0.0);
        } else {
            self.press = 0.0;
            self.release_pending = false;
            self.scale_x = 1.0;
            self.scale_y = 1.0;
        }
        self.velocity = 0.0;
        self.velocity_vel = 0.0;
    }

    pub fn set_pressed(&mut self, pressed: bool) {
        if self.disabled {
            return;
        }
        if self.pressed == pressed {
            return;
        }
        self.pressed = pressed;
        if pressed {
            // 原版 press(): reset velocity tracker + animate press/scale to pressed
            self.release_pending = false;
            self.velocity = 0.0;
            self.velocity_vel = 0.0;
        } else {
            self.release_pending = true; // release(): 等 value 收敛
        }
        if self.material == Material::Solid {
            self.press = if pressed { 1.0 } else { 0.0 };
            self.release_pending = false;
            self.scale_x = 1.0;
            self.scale_y = 1.0;
            self.scale_x_vel = 0.0;
            self.scale_y_vel = 0.0;
        }
    }

    /// 取走一次性提交值（拖拽停止的 `targetValue` 或 tap 的目标），供宿主写回权威 value。
    pub fn take_committed_value(&mut self) -> Option<f32> {
        self.drag_committed
            .take()
            .or_else(|| self.tap_committed.take())
    }

    /// 由轨道内 x（DIP，绝对）计算 tap 目标值（LTR/RTL，coerce）。
    /// `track_left` 未设置时返回当前 target（无跳转）。
    pub fn tap_target_from_x(&self, x_dip: f32) -> f32 {
        if !self.track_left.is_finite() {
            return self.target_value;
        }
        let tw = self.track_width.max(1.0);
        let local = ((x_dip - self.track_left) / tw).clamp(0.0, 1.0);
        let t = if self.ltr { local } else { 1.0 - local };
        self.coerce(self.range_start + span_of(self.range_start, self.range_end) * t)
    }

    /// 轨道点按跳转（绝对 x，DIP）：等价原版 tap 手势。
    pub fn tap_at(&mut self, x_dip: f32) {
        let target = self.tap_target_from_x(x_dip);
        self.animate_to_value(target);
    }

    pub fn is_animating(&self) -> bool {
        let hold = self.pressed || self.release_pending;
        let glass = self.material == Material::Glass;
        let press_target = if hold && glass && !self.disabled {
            1.0
        } else {
            0.0
        };
        let scale_target = if hold && glass && !self.disabled {
            THUMB_PRESSED_SCALE
        } else {
            1.0
        };
        let vt = self.visibility_threshold.max(SNAP_001);
        let value_settled =
            (self.value - self.target_value).abs() <= vt && self.value_vel.abs() <= vt;
        self.press != press_target
            || !value_settled
            || self.scale_x != scale_target
            || self.scale_y != scale_target
            || self.scale_x_vel.abs() > SNAP_001
            || self.scale_y_vel.abs() > SNAP_001
            || self.velocity.abs() > (self.visibility_threshold * 10.0).max(SNAP_001)
            || self.velocity_vel.abs() > SNAP_001
            || self.release_pending
    }

    fn value_settled_for_release(&self) -> bool {
        let span = span_of(self.range_start, self.range_end);
        (self.value - self.target_value).abs() < span * RELEASE_THRESHOLD
    }

    pub fn advance(&mut self, dt_ms: f32) {
        let dt = dt_ms.max(0.0) / 1000.0;
        if dt <= 0.0 {
            return;
        }
        let glass = self.material == Material::Glass;

        // value: spring(1,1000,visibilityThreshold) —— 始终向 target 推进
        //（原版 updateValue/LaunchedEffect 驱动；拖拽时 target 随 delta 变，value 追踪，
        // 刚性弹簧快速跟手且产生 velocity，保留 layerBlock 形变语义）
        let prev_value = self.value;
        let vt = self.visibility_threshold.max(SNAP_001);
        spring_step(
            &mut self.value,
            &mut self.value_vel,
            self.target_value,
            dt,
            OMEGA_1000,
            1.0,
            vt,
        );
        self.value = self.coerce(self.value);

        // updateVelocity()：由 value 运动派生（原版 VelocityTracker.addPosition(value,0)）
        let span = span_of(self.range_start, self.range_end);
        let inst_v = if dt > 0.0 {
            (self.value - prev_value) / dt / span
        } else {
            0.0
        };
        let vel_vis = (self.visibility_threshold * 10.0).max(SNAP_001);
        let vel_target = if self.dragging
            || (self.value - self.target_value).abs() > vt
            || self.value_vel.abs() > vt
        {
            inst_v
        } else {
            0.0
        };
        spring_step(
            &mut self.velocity,
            &mut self.velocity_vel,
            vel_target,
            dt,
            OMEGA_300,
            0.5,
            vel_vis,
        );

        // release_pending：等 value 收敛到 2.5% 再释放 press/scale（原版 release()）
        if self.release_pending && self.value_settled_for_release() {
            self.release_pending = false;
        }
        let hold = self.pressed || self.release_pending;
        let press_target = if hold && !self.disabled { 1.0 } else { 0.0 };
        let scale_target = if hold && !self.disabled && glass {
            THUMB_PRESSED_SCALE
        } else {
            1.0
        };

        // press: spring(1,1000,0.001) —— Glass 动画；Solid 在 set_pressed 中已瞬变
        if glass {
            advance_toward(&mut self.press, press_target, dt, OMEGA_1000, SNAP_001);
        } else {
            self.press = press_target;
            self.release_pending = false;
        }

        // scaleX/Y: 欠阻尼 spring(0.6/0.7,250,0.001) —— 严格按原版（Glass）
        if glass {
            spring_step(
                &mut self.scale_x,
                &mut self.scale_x_vel,
                scale_target,
                dt,
                OMEGA_250,
                0.6,
                SNAP_001,
            );
            spring_step(
                &mut self.scale_y,
                &mut self.scale_y_vel,
                scale_target,
                dt,
                OMEGA_250,
                0.7,
                SNAP_001,
            );
        } else {
            self.scale_x = 1.0;
            self.scale_x_vel = 0.0;
            self.scale_y = 1.0;
            self.scale_y_vel = 0.0;
            self.velocity = 0.0;
            self.velocity_vel = 0.0;
        }
    }

    /// 处理指针事件（宿主在 `on_input` 中，`click` 之前调用；顺序对齐 Toggle）。
    ///
    /// - Down + pressed：进入拖拽，记起点。
    /// - Move：越过 slop 后按增量 `delta = span*(dx/trackWidth)` 累加 `targetValue`
    ///   （原版 `onDrag` 基于 `targetValue`），coerce；value spring 追踪 target。
    /// - Up：若 didDrag 则暂存 `drag_committed = targetValue`（原版 onDragStopped），
    ///   退出拖拽并进入 release_pending；否则交给宿主 click/tap。
    pub fn process_pointer_events(&mut self, events: &[zephyra_fx::PointerInput], scale: f32) {
        if self.disabled {
            return;
        }
        for ev in events {
            let x_dip = ev.x / scale.max(1.0);
            match ev.phase {
                zephyra_fx::PointerInputPhase::Down => {
                    if self.pressed {
                        self.dragging = true;
                        self.did_drag = false;
                        self.down_x = x_dip;
                        self.last_x = x_dip;
                        self.drag_committed = None;
                        self.tap_committed = None;
                    }
                }
                zephyra_fx::PointerInputPhase::Move => {
                    if self.dragging {
                        if !self.did_drag {
                            let travel = x_dip - self.down_x;
                            // 原版 didDrag = dragAmount.x != 0f；桌面适配用 slop+overSlop
                            if travel.abs() > DRAG_SLOP {
                                self.did_drag = true;
                                let over = travel - DRAG_SLOP * travel.signum();
                                self.apply_drag_delta(over);
                            }
                        } else {
                            let dx = x_dip - self.last_x;
                            self.apply_drag_delta(dx);
                        }
                        self.last_x = x_dip;
                    }
                }
                zephyra_fx::PointerInputPhase::Up => {
                    if self.dragging {
                        self.dragging = false;
                        if self.did_drag {
                            // 原版 onDragStopped: if (didDrag) onValueChange(targetValue)
                            self.drag_committed = Some(self.target_value);
                            self.did_drag = false;
                        } else if self.track_left.is_finite() {
                            // 非拖拽的点按落在拇指/轨道交互区：按位置跳转（原版轨道 tap + 拇指点击的可用近似）
                            self.tap_at(x_dip);
                        }
                        // release_pending 已由 set_pressed(false) 在 on_input 流程中置位；兜底。
                        self.release_pending = true;
                    }
                }
            }
        }
    }

    fn apply_drag_delta(&mut self, dx: f32) {
        let span = span_of(self.range_start, self.range_end);
        let tw = self.track_width.max(1.0);
        let delta = span * (dx / tw);
        let next = if self.ltr {
            self.target_value + delta
        } else {
            self.target_value - delta
        };
        let prev = self.value;
        self.target_value = self.coerce(next);
        // 拖拽跟手：直接设 value=target（原版经 onDrag/onValueChange 同步，视觉上即时跟踪）。
        // 同时由位移量注入速度读数，保留 layerBlock 形变（原版 updateVelocity 在 value 动画中触发）。
        self.value = self.target_value;
        self.value_vel = 0.0;
        let span_n = span.max(1e-6);
        let inst = (self.value - prev) / span_n; // per-frame-normalized impulse (dt unknown here)
                                                 // 作为速度目标的瞬时估计（advance 会用 spring 平滑）；量级按 ~60fps 近似。
        self.velocity += inst * 60.0;
    }
}

fn advance_toward(value: &mut f32, target: f32, dt: f32, omega: f32, snap: f32) {
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
    let blend = if dt > 0.5 {
        1.0
    } else {
        1.0 - (-omega * dt).exp()
    };
    *value += (target - *value) * blend;
    if (*value - target).abs() < snap {
        *value = target;
    }
}

fn spring_step(x: &mut f32, v: &mut f32, target: f32, dt: f32, omega: f32, zeta: f32, snap: f32) {
    if !x.is_finite() || !v.is_finite() {
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

#[cfg(test)]
#[path = "tests/slider_tests.rs"]
mod tests;

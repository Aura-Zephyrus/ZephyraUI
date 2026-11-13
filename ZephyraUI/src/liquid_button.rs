//! 胶囊/液态按钮的交互模型 —— 一比一复刻 AndroidLiquidGlass `LiquidButton` + `InteractiveHighlight`。
//!
//! # 蓝本（Apache-2.0，`E:\AndroidLiquidGlass`）
//!
//! - `components/LiquidButton.kt`
//! - `utils/InteractiveHighlight.kt`
//! - `utils/DragGestureInspector.kt`
//!
//! `layerBlock` 语义（严格按原版）：
//! - `progress` = press 弹簧（`spring(0.5, 300, 0.001)`）
//! - `baseScale = 1 + 4dp/height * progress`
//! - 位移：`maxOffset=min(w,h)`, `translation = maxOffset*tanh(0.05*offset/maxOffset)`
//! - 方向性形变：`scaleX += 4dp/height * |cos(angle)*offset.x/maxDim| * min(w/h,1)`，
//!   `scaleY += 4dp/height * |sin(angle)*offset.y/maxDim| * min(h/w,1)`
//!
//! `InteractiveHighlight` 的径向着色器高光（RuntimeShader + Plus 混合）在 Zephyra
//! 当前装饰/滤镜原语中无直接等价，**不伪造**；本模型输出 `press_progress` 与
//! `offset`，供宿主用 `decoration`/`background` 做可用近似（白罩/高光强度）。
//! `vibrancy()` 亦无独立 Filter 变体，玻璃滤镜用 `blur(2) + lens(12,24)` 近似。
//!
//! # 分工
//!
//! | 关注 | 住在哪 |
//! |---|---|
//! | 按压/拖拽偏移/形变**规则** | 本模块 |
//! | 结构（节点树、分类名、绑定位） | `ui/controls.zypui` |
//! | 颜色与尺寸 | `controls-button.zstyle` |
//! | 状态存留与点击分派 | 宿主应用 |

use zephyra_core::filter::{Filter, FilterChain, Material};
use zephyra_core::property::PropertyValue;

/// 默认胶囊高度（DIP）。原版 `height(48f.dp)`。
pub const LIQUID_BUTTON_H: f32 = 48.0;
/// 默认胶囊最小宽参考（DIP，仅用于未设尺寸时的形变计算）。
pub const LIQUID_BUTTON_W: f32 = 120.0;

const OMEGA_300: f32 = 17.320509; // sqrt(300)
const ZETA: f32 = 0.5;
const SNAP_001: f32 = 0.001;
const MAX_SPRING_STEP: f32 = 0.004;
const MAX_SPRING_STEPS: f32 = 256.0;

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

/// 胶囊按钮/下拉按钮的液态交互状态（`InteractiveHighlight` + `LiquidButton.layerBlock`）。
#[derive(Debug, Clone, PartialEq)]
pub struct LiquidButton {
    key: &'static str,
    material: Material,
    disabled: bool,
    width: f32,
    height: f32,

    pressed: bool,
    press: f32,
    press_vel: f32,

    // 位置：相对按下点的偏移（DIP），释放后弹回 0（原版 positionAnimation -> startPosition）
    start_x: f32,
    start_y: f32,
    pos_x: f32,
    pos_y: f32,
    pos_x_vel: f32,
    pos_y_vel: f32,
    tracking: bool,
}

impl LiquidButton {
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            material: Material::Solid,
            disabled: false,
            width: LIQUID_BUTTON_W,
            height: LIQUID_BUTTON_H,
            pressed: false,
            press: 0.0,
            press_vel: 0.0,
            start_x: 0.0,
            start_y: 0.0,
            pos_x: 0.0,
            pos_y: 0.0,
            pos_x_vel: 0.0,
            pos_y_vel: 0.0,
            tracking: false,
        }
    }

    pub fn with_material(mut self, material: Material) -> Self {
        self.material = material;
        self
    }

    pub fn with_size(mut self, width: f32, height: f32) -> Self {
        self.set_size(width, height);
        self
    }

    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn key(&self) -> &'static str {
        self.key
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

    pub fn set_size(&mut self, width: f32, height: f32) {
        if width.is_finite() && width > 0.0 {
            self.width = width;
        }
        if height.is_finite() && height > 0.0 {
            self.height = height;
        }
    }

    pub fn press_progress(&self) -> f32 {
        self.press
    }

    pub fn offset(&self) -> (f32, f32) {
        (self.pos_x - self.start_x, self.pos_y - self.start_y)
    }

    /// `translationX/Y`：`maxOffset*tanh(0.05*offset/maxOffset)`（原版 layerBlock）。
    pub fn translation(&self) -> (f32, f32) {
        let (ox, oy) = self.offset();
        let max_offset = self.width.min(self.height).max(1.0);
        let k = 0.05;
        (
            max_offset * (k * ox / max_offset).tanh(),
            max_offset * (k * oy / max_offset).tanh(),
        )
    }

    /// `(scaleX, scaleY)`：原版 `layerBlock`（base + directional drag scale）。
    pub fn scale(&self) -> (f32, f32) {
        let h = self.height.max(1.0);
        let w = self.width.max(1.0);
        let progress = self.press;
        let base = 1.0 + 4.0 / h * progress;
        if self.material == Material::Solid || !self.pressed && progress <= SNAP_001 {
            // Solid 或静止：仅 base（Solid 在 advance 中 press 瞬变，仍给轻微 base 反馈可选；
            // 这里按“Solid = 最小/瞬变”处理为无拖拽形变，base 随 press）。
            let b = if self.material == Material::Solid {
                1.0
            } else {
                base
            };
            return (b, b);
        }
        let (ox, oy) = self.offset();
        let max_dim = w.max(h);
        let max_drag = 4.0 / h;
        let angle = oy.atan2(ox);
        let sx = base + max_drag * (angle.cos() * ox / max_dim).abs() * (w / h).min(1.0);
        let sy = base + max_drag * (angle.sin() * oy / max_dim).abs() * (h / w).min(1.0);
        (sx, sy)
    }

    pub fn set_pressed(&mut self, pressed: bool) {
        if self.disabled {
            return;
        }
        if self.pressed == pressed {
            return;
        }
        self.pressed = pressed;
        if !pressed {
            self.tracking = false;
            // 释放：position 弹回 start（offset -> 0）
            self.pos_x_vel = 0.0;
            self.pos_y_vel = 0.0;
        }
        if self.material == Material::Solid {
            self.press = if pressed { 1.0 } else { 0.0 };
            self.press_vel = 0.0;
            self.pos_x = self.start_x;
            self.pos_y = self.start_y;
            self.pos_x_vel = 0.0;
            self.pos_y_vel = 0.0;
        }
    }

    pub fn process_pointer_events(&mut self, events: &[zephyra_fx::PointerInput], scale: f32) {
        if self.disabled {
            return;
        }
        for ev in events {
            let x_dip = ev.x / scale.max(1.0);
            let y_dip = ev.y / scale.max(1.0);
            match ev.phase {
                zephyra_fx::PointerInputPhase::Down => {
                    if self.pressed {
                        self.tracking = true;
                        self.start_x = x_dip;
                        self.start_y = y_dip;
                        self.pos_x = x_dip;
                        self.pos_y = y_dip;
                        self.pos_x_vel = 0.0;
                        self.pos_y_vel = 0.0;
                    }
                }
                zephyra_fx::PointerInputPhase::Move => {
                    if self.tracking && self.pressed && self.material == Material::Glass {
                        self.pos_x = x_dip;
                        self.pos_y = y_dip;
                    }
                }
                zephyra_fx::PointerInputPhase::Up => {
                    self.tracking = false;
                }
                zephyra_fx::PointerInputPhase::Cancel => {
                    // ⚠ **取消不是抬起**：系统抢走了这次交互（Windows 11 的
                    // 三指/四指手势）。这时**不能**提交 —— 用户没抬手。
                    self.tracking = false;
                }
            }
        }
    }

    pub fn advance(&mut self, dt_ms: f32) {
        let dt = dt_ms.max(0.0) / 1000.0;
        if dt <= 0.0 {
            return;
        }
        let target = if self.pressed && !self.disabled {
            1.0
        } else {
            0.0
        };
        if self.material == Material::Glass {
            spring_step(
                &mut self.press,
                &mut self.press_vel,
                target,
                dt,
                OMEGA_300,
                ZETA,
                SNAP_001,
            );
            // 位置：按住时 snap 已在 process 中完成；释放后弹回 start
            let tx = self.start_x;
            let ty = self.start_y;
            if !self.pressed {
                spring_step(
                    &mut self.pos_x,
                    &mut self.pos_x_vel,
                    tx,
                    dt,
                    OMEGA_300,
                    ZETA,
                    SNAP_001,
                );
                spring_step(
                    &mut self.pos_y,
                    &mut self.pos_y_vel,
                    ty,
                    dt,
                    OMEGA_300,
                    ZETA,
                    SNAP_001,
                );
            }
        } else {
            self.press = target;
            self.press_vel = 0.0;
            self.pos_x = self.start_x;
            self.pos_y = self.start_y;
            self.pos_x_vel = 0.0;
            self.pos_y_vel = 0.0;
        }
    }

    pub fn is_animating(&self) -> bool {
        let target = if self.pressed && !self.disabled && self.material == Material::Glass {
            1.0
        } else {
            0.0
        };
        if self.material == Material::Solid {
            return false;
        }
        (self.press - target).abs() > SNAP_001
            || self.press_vel.abs() > SNAP_001
            || (!self.pressed
                && ((self.pos_x - self.start_x).abs() > SNAP_001
                    || (self.pos_y - self.start_y).abs() > SNAP_001
                    || self.pos_x_vel.abs() > SNAP_001
                    || self.pos_y_vel.abs() > SNAP_001))
    }
}

#[cfg(test)]
#[path = "../tests/unit/liquid_button_tests.rs"]
mod tests;

/// 原版 `effects { vibrancy(); blur(2dp); lens(12dp,24dp) }` 的 Zephyra 近似。
///
/// `vibrancy()` 在 Zephyra Filter 中无独立变体，省略。
///
/// # 为什么用 **SVG 式像素偏移玻璃**而不是 `blur(2) + lens(12,24)`
///
/// `lens()` 要读元素**背后**的画面 ⇒ 每个带滤镜的元素都要拷一份**全屏**的
/// 目标副本（MSAA 开着时那是一次**整幅解析** —— D3D12 没有「按区解析」）。
///
/// ⇒ 一颗按钮，代价是**一次全屏拷贝**。实测（gallery 1400×800 /
/// Intel UHD 630）：材质页一帧 7 个带滤镜的元素 ⇒ **玻璃税 8.4 ms/帧**，
/// 占那一帧 GPU 开销的 74% —— 而这一颗按钮就是其中之一。
///
/// [`Filter::PixelOffset`] 的位移是**解析**的（掰形状自己的边，不采样背景）
/// ⇒ 不开离屏 pass ⇒ 不拷全屏副本 ⇒ **几乎不花钱**。
///
/// ⚠ 视觉上的差别：**背景不再被透镜掰弯**。真实液态玻璃那条路**完整保留**
/// （[`Filter::Lens`] / `material: glass-refract`），要就用它。
pub fn glass_filter_value() -> PropertyValue {
    PropertyValue::filter(FilterChain::single(Filter::PixelOffset {
        // ⚠ 48px 高的按钮：位移是**形状位移**（像素），给 12 会推掉四分之一高度。
        // 5px 是「看得见有厚度、形状还站得住」的量（着色器另有一道短边 40% 的夹）
        amount: 5.0,
        bezel: 12.0,
    }))
}

/// 按压高光强度的装饰近似（原版 `InteractiveHighlight` 用径向 RuntimeShader + Plus 混合；
/// 此处用 `control_with_progress` 近似强度变化，不复刻径向跟随）。
pub fn glass_decoration_value(press: f32) -> PropertyValue {
    PropertyValue::decorations(zephyra_core::decoration::Decorations::control_with_progress(press))
}

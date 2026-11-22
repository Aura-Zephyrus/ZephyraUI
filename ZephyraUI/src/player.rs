//! 视频播放器控件的状态模型。
//!
//! # 分工（与 `checkbox` / `toggle` / `slider` 同构）
//!
//! | 关注 | 住在哪 |
//! |---|---|
//! | 播放状态、时间、音量、倍速、全屏的**值与流转规则** | 本模块 |
//! | 节点树 / 分类名 / key / 绑定位 | `ui/controls.zypui` 的 `VideoPlayer` |
//! | 颜色与尺寸 | `controls-video.zstyle` |
//! | 状态的**存留**与真实副作用（解码、音频、窗口） | 宿主应用 |
//!
//! 控件自己**不解码、不计时**：`advance(dt_ms)` 只是「时间怎么走」的规则，
//! 真正的时钟、帧、音量输出与全屏都由宿主按本模型暴露的值去执行。

/// 播放状态分类词（落在 `VideoPlayer` 根节点自己的 class 上）。
pub const CLASS_PLAYING: &str = "playing";
pub const CLASS_PAUSED: &str = "paused";
pub const CLASS_BUFFERING: &str = "buffering";
pub const CLASS_ENDED: &str = "ended";

/// 交互 key。
///
/// - `act:` 是按钮（点击分派到宿主）；
/// - `fill:` 是**允许写本地 width** 的填充条节点（见 gallery 的
///   `only_fill_nodes_carry_local_width` 门禁），拖动条因此能用绑定值改变宽度。
pub const KEY_PLAY_PAUSE: &str = "act:player-play-pause";
pub const KEY_SEEK: &str = "act:player-seek";
pub const KEY_SEEK_FILL: &str = "fill:player-seek";
pub const KEY_VOLUME: &str = "act:player-volume";
pub const KEY_VOLUME_FILL: &str = "fill:player-volume";
pub const KEY_MUTE: &str = "act:player-mute";
pub const KEY_RATE: &str = "act:player-rate";
pub const KEY_FULLSCREEN: &str = "act:player-fullscreen";

/// 进度条/音量条的轨道宽度（DIP），与 `.player-seek-track` / `.player-volume-track` 对齐。
pub const SEEK_TRACK_W: f32 = 240.0;
pub const VOLUME_TRACK_W: f32 = 80.0;
/// 拇指宽度（DIP），用于把进度换算成 `translate-x`。
pub const THUMB_W: f32 = 12.0;

/// 倍速循环档位。`1.0` 是默认，末尾的 `0.75` 让循环回到 1.0 前经过慢放。
pub const RATES: &[f64] = &[1.0, 1.25, 1.5, 2.0, 0.75];

/// 播放状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Buffering,
    Playing,
    Paused,
    Ended,
}

impl PlaybackState {
    /// 分类词，必须与 `controls-video.zstyle` 的 `.playing` 等规则同名。
    pub fn class_word(self) -> &'static str {
        match self {
            Self::Buffering => CLASS_BUFFERING,
            Self::Playing => CLASS_PLAYING,
            Self::Paused => CLASS_PAUSED,
            Self::Ended => CLASS_ENDED,
        }
    }
}

/// 播放器状态。
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    key: &'static str,
    label_key: &'static str,
    playback: PlaybackState,
    current_us: i64,
    duration_us: i64,
    volume: f32,
    muted: bool,
    rate_index: usize,
    fullscreen: bool,
    /// 宿主在布局后写入的几何（左边缘、宽度，DIP），用于把指针 x 换算成比例。
    seek_geom: (f32, f32),
    volume_geom: (f32, f32),
}

impl PlayerState {
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            label_key: "ui.player",
            playback: PlaybackState::Paused,
            current_us: 0,
            duration_us: 0,
            volume: 1.0,
            muted: false,
            rate_index: 0,
            fullscreen: false,
            seek_geom: (0.0, SEEK_TRACK_W),
            volume_geom: (0.0, VOLUME_TRACK_W),
        }
    }

    pub fn with_label_key(mut self, label_key: &'static str) -> Self {
        self.label_key = label_key;
        self
    }

    pub fn with_duration_us(mut self, duration_us: i64) -> Self {
        self.duration_us = duration_us.max(0);
        self
    }

    pub fn key(&self) -> &'static str {
        self.key
    }
    pub fn label_key(&self) -> &'static str {
        self.label_key
    }
    pub fn playback(&self) -> PlaybackState {
        self.playback
    }
    pub fn state_class(&self) -> &'static str {
        self.playback.class_word()
    }
    pub fn is_playing(&self) -> bool {
        self.playback == PlaybackState::Playing
    }
    pub fn current_us(&self) -> i64 {
        self.current_us
    }
    pub fn duration_us(&self) -> i64 {
        self.duration_us
    }
    pub fn rate(&self) -> f64 {
        RATES[self.rate_index.min(RATES.len() - 1)]
    }
    pub fn volume(&self) -> f32 {
        self.volume
    }
    pub fn muted(&self) -> bool {
        self.muted
    }
    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub fn set_duration_us(&mut self, duration_us: i64) {
        self.duration_us = duration_us.max(0);
        self.clamp_time();
    }

    pub fn set_current_us(&mut self, us: i64) {
        self.current_us = us.max(0);
        if self.playback == PlaybackState::Ended && self.duration_us > 0 {
            self.current_us = self.current_us.min(self.duration_us);
        } else {
            self.clamp_time();
        }
    }

    fn clamp_time(&mut self) {
        if self.duration_us > 0 {
            self.current_us = self.current_us.clamp(0, self.duration_us);
        } else {
            self.current_us = self.current_us.max(0);
        }
    }

    /// 开始播放。已结束的播放器从头开始（宿主据此重新 seek）。
    pub fn play(&mut self) {
        if self.playback == PlaybackState::Ended {
            self.current_us = 0;
        }
        self.playback = PlaybackState::Playing;
    }

    pub fn pause(&mut self) {
        if self.playback == PlaybackState::Playing || self.playback == PlaybackState::Buffering {
            self.playback = PlaybackState::Paused;
        }
    }

    pub fn toggle_play(&mut self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn buffer(&mut self) {
        self.playback = PlaybackState::Buffering;
    }

    pub fn end(&mut self) {
        self.playback = PlaybackState::Ended;
        if self.duration_us > 0 {
            self.current_us = self.duration_us;
        }
    }

    /// 按帧推进：只在播放态走，按倍速缩放，到末端自动结束。
    pub fn advance(&mut self, dt_ms: f64) {
        if self.playback != PlaybackState::Playing {
            return;
        }
        if !dt_ms.is_finite() || dt_ms <= 0.0 {
            return;
        }
        self.current_us += (dt_ms * 1000.0 * self.rate()) as i64;
        if self.duration_us > 0 && self.current_us >= self.duration_us {
            self.end();
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
    }

    /// 实际输出音量（静音时为 0）。宿主把它交给 `MediaPlayer::set_volume`。
    pub fn effective_volume(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.volume
        }
    }

    pub fn cycle_rate(&mut self) {
        self.rate_index = (self.rate_index + 1) % RATES.len();
    }

    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
    }

    /// 进度 0..1。
    pub fn progress01(&self) -> f32 {
        if self.duration_us <= 0 {
            return 0.0;
        }
        (self.current_us as f64 / self.duration_us as f64).clamp(0.0, 1.0) as f32
    }

    /// 填充宽度留出拇指的位置：布局里拇指紧跟填充之后，两者相加不得超过轨道宽，
    /// 否则 100% 时拇指会伸出轨道（更糟的是旧版还叠加 translate-x，直接飘到音量条上）。
    pub fn seek_fill_w(&self) -> f32 {
        (SEEK_TRACK_W - THUMB_W) * self.progress01()
    }

    pub fn volume_fill_w(&self) -> f32 {
        (VOLUME_TRACK_W - THUMB_W) * self.effective_volume()
    }

    /// 宿主在布局后写入拖动条的实际几何。
    pub fn set_seek_geometry(&mut self, left: f32, width: f32) {
        if left.is_finite() && width.is_finite() && width > 0.0 {
            self.seek_geom = (left, width);
        }
    }

    pub fn set_volume_geometry(&mut self, left: f32, width: f32) {
        if left.is_finite() && width.is_finite() && width > 0.0 {
            self.volume_geom = (left, width);
        }
    }

    /// 指针落在进度条上时按 x 跳转到对应时间。
    pub fn seek_at(&mut self, x: f32) {
        let (left, width) = self.seek_geom;
        let usable = (width - THUMB_W).max(1.0);
        let ratio = ((x - left) / usable).clamp(0.0, 1.0);
        self.current_us = (ratio as f64 * self.duration_us as f64) as i64;
        self.clamp_time();
    }

    /// 指针落在音量条上时按 x 设置音量。
    pub fn set_volume_at(&mut self, x: f32) {
        let (left, width) = self.volume_geom;
        let usable = (width - THUMB_W).max(1.0);
        let ratio = ((x - left) / usable).clamp(0.0, 1.0);
        self.set_volume(ratio);
    }

    /// 播放/暂停按钮的图标名（登记在 UI 层图标集里）。
    pub fn play_icon(&self) -> &'static str {
        if self.is_playing() {
            "pause"
        } else {
            "play"
        }
    }

    pub fn mute_icon(&self) -> &'static str {
        if self.muted || self.volume <= 0.0 {
            "volume-off"
        } else {
            "volume-up"
        }
    }

    pub fn fullscreen_icon(&self) -> &'static str {
        if self.fullscreen {
            "minimize" // 框架内置图标
        } else {
            "fullscreen"
        }
    }

    pub fn rate_text(&self) -> String {
        format!("{}x", self.rate())
    }

    /// `mm:ss`（超过一小时用 `h:mm:ss`）。语言无关，属于数值格式而非文案。
    pub fn format_time(us: i64) -> String {
        let total = (us.max(0) / 1_000_000) as u64;
        let secs = total % 60;
        let mins = (total / 60) % 60;
        let hours = total / 3600;
        if hours > 0 {
            format!("{hours}:{mins:02}:{secs:02}")
        } else {
            format!("{mins}:{secs:02}")
        }
    }

    pub fn time_text(&self) -> String {
        Self::format_time(self.current_us)
    }

    pub fn duration_text(&self) -> String {
        Self::format_time(self.duration_us)
    }
}

#[cfg(test)]
#[path = "../tests/unit/player_tests.rs"]
mod tests;

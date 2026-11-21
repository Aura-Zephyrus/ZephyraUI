//! presentation clock：把媒体时间戳（微秒）映射到墙钟。
//!
//! 音频设备是 master 时钟时，本类型只做"当前播放位置"的估计，视频帧按 PTS
//! 与它的差值决定显示 / 等待 / 丢弃。

use std::time::Instant;

#[derive(Debug, Clone)]
pub struct PresentationClock {
    /// 媒体时间基点（微秒），seek 后更新。
    media_base_us: i64,
    /// 墙钟基点。
    wall_base: Instant,
    /// 暂停时记录的媒体时间。
    paused_us: Option<i64>,
    rate: f64,
}

impl Default for PresentationClock {
    fn default() -> Self {
        Self::new()
    }
}

impl PresentationClock {
    pub fn new() -> Self {
        Self {
            media_base_us: 0,
            wall_base: Instant::now(),
            paused_us: None,
            rate: 1.0,
        }
    }

    /// 当前媒体播放位置（微秒）。
    pub fn now_us(&self) -> i64 {
        self.paused_us.unwrap_or_else(|| {
            let elapsed = self.wall_base.elapsed().as_secs_f64() * 1_000_000.0 * self.rate;
            self.media_base_us.saturating_add(elapsed as i64)
        })
    }

    pub fn pause(&mut self) {
        if self.paused_us.is_none() {
            self.paused_us = Some(self.now_us());
        }
    }

    pub fn resume(&mut self) {
        if let Some(us) = self.paused_us.take() {
            self.media_base_us = us;
            self.wall_base = Instant::now();
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused_us.is_some()
    }

    pub fn seek(&mut self, media_us: i64) {
        self.media_base_us = media_us;
        self.wall_base = Instant::now();
        self.paused_us = None;
    }

    pub fn set_rate(&mut self, rate: f64) {
        // 换速率前先固定当前位置，避免跳变。
        let current = self.now_us();
        self.media_base_us = current;
        self.wall_base = Instant::now();
        self.rate = rate.max(0.0);
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// 某个 PTS 相对当前播放位置的差（正数表示还要等多久）。
    pub fn delta_us(&self, pts_us: i64) -> i64 {
        pts_us.saturating_sub(self.now_us())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_clock_starts_at_zero() {
        let c = PresentationClock::new();
        assert!(c.now_us() >= 0);
        assert!(!c.is_paused());
    }

    #[test]
    fn pause_freezes_position() {
        let mut c = PresentationClock::new();
        std::thread::sleep(std::time::Duration::from_millis(2));
        c.pause();
        let a = c.now_us();
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert_eq!(c.now_us(), a);
        c.resume();
    }

    #[test]
    fn seek_moves_position() {
        let mut c = PresentationClock::new();
        c.seek(1_000_000);
        assert!(c.now_us() >= 1_000_000);
    }

    #[test]
    fn delta_sign() {
        let mut c = PresentationClock::new();
        c.pause();
        c.seek(1_000);
        assert!(c.delta_us(2_000) > 0);
        assert!(c.delta_us(500) < 0);
    }
}

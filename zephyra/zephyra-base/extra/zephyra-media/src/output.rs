//! 音频输出：把解码后的 PCM 帧送到系统音频设备。
//!
//! 使用 `cpal` 屏蔽 WASAPI / ALSA / CoreAudio 差异。设备输出格式固定为
//! `f32` 或 `i16`，由 constructing 时选择；解码器的 PlanarS16 / PlanarF32 /
//! PlanarS32 由调用方转换为交错格式后送入。

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};

use crate::error::{MediaError, Result};

/// 输出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputSampleFormat {
    I16,
    F32,
}

/// 音频设备输出。
pub struct AudioSink {
    _stream: Stream,
    /// 设备回调读取的环形缓冲区（交错采样，字节）。
    ring: Arc<RingBuffer>,
    bytes_per_frame: u32,
    volume: Arc<AtomicU32>,
}

impl AudioSink {
    /// 打开默认输出设备。
    pub fn open(sample_rate: u32, channels: u16, format: OutputSampleFormat) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| MediaError::Platform("no default audio output device".into()))?;

        let sample_format = match format {
            OutputSampleFormat::I16 => SampleFormat::I16,
            OutputSampleFormat::F32 => SampleFormat::F32,
        };
        let config = cpal::StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };
        let mut supported = device
            .supported_output_configs()
            .map_err(|e| MediaError::Platform(format!("query output configs: {e}")))?;
        if !supported.any(|c| {
            c.channels() == channels
                && c.sample_format() == sample_format
                && c.min_sample_rate() <= cpal::SampleRate(sample_rate)
                && c.max_sample_rate() >= cpal::SampleRate(sample_rate)
        }) {
            return Err(MediaError::Platform(format!(
                "unsupported output config: {channels}ch {sample_rate}Hz {format:?}"
            )));
        }

        let bytes_per_sample = if format == OutputSampleFormat::F32 {
            4
        } else {
            2
        };
        let bytes_per_frame = channels as u32 * bytes_per_sample;
        let ring = Arc::new(RingBuffer::new(bytes_per_frame as usize * 256));
        let volume = Arc::new(AtomicU32::new(u32::from_ne_bytes(1.0f32.to_ne_bytes())));

        let ring_for_callback = Arc::clone(&ring);
        let vol_for_callback = Arc::clone(&volume);
        let channels_usize = channels as usize;

        let err_fn = |e| eprintln!("audio output error: {e}");
        let stream = match format {
            OutputSampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    let vol =
                        f32::from_ne_bytes(vol_for_callback.load(Ordering::Relaxed).to_ne_bytes());
                    let mut buf = [0u8; 8];
                    for frame in data.chunks_exact_mut(channels_usize) {
                        for sample in frame.iter_mut() {
                            if ring_for_callback.read_exact(&mut buf[..2]) {
                                let v = i16::from_ne_bytes([buf[0], buf[1]]);
                                *sample = (v as f32 * vol) as i16;
                            } else {
                                *sample = 0;
                            }
                        }
                    }
                },
                err_fn,
                None,
            ),
            OutputSampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    let vol =
                        f32::from_ne_bytes(vol_for_callback.load(Ordering::Relaxed).to_ne_bytes());
                    let mut buf = [0u8; 4];
                    for frame in data.chunks_exact_mut(channels_usize) {
                        for sample in frame.iter_mut() {
                            if ring_for_callback.read_exact(&mut buf[..4]) {
                                let v = f32::from_ne_bytes(buf[0..4].try_into().unwrap());
                                *sample = v * vol;
                            } else {
                                *sample = 0.0;
                            }
                        }
                    }
                },
                err_fn,
                None,
            ),
        }
        .map_err(|e| MediaError::Platform(format!("build output stream: {e}")))?;

        stream
            .play()
            .map_err(|e| MediaError::Platform(format!("play audio stream: {e}")))?;

        Ok(Self {
            _stream: stream,
            ring,
            bytes_per_frame,
            volume,
        })
    }

    /// 写入交错 PCM 字节（格式与 `open` 时一致）。
    pub fn write(&self, pcm: &[u8]) -> usize {
        self.ring.write(pcm)
    }

    pub fn bytes_per_frame(&self) -> u32 {
        self.bytes_per_frame
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.store(
            u32::from_ne_bytes(volume.clamp(0.0, 1.0).to_ne_bytes()),
            Ordering::Relaxed,
        );
    }

    pub fn volume(&self) -> f32 {
        f32::from_ne_bytes(self.volume.load(Ordering::Relaxed).to_ne_bytes())
    }

    /// 丢弃所有未播放的数据。
    pub fn clear(&self) {
        self.ring.clear();
    }
}

/// 简单的环形字节缓冲（音频线程读，解码线程写）。
struct RingBuffer {
    inner: Mutex<RingInner>,
}

struct RingInner {
    buf: Vec<u8>,
    read: usize,
    write: usize,
}

impl RingBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(RingInner {
                buf: vec![0u8; capacity],
                read: 0,
                write: 0,
            }),
        }
    }

    fn write(&self, data: &[u8]) -> usize {
        let Ok(mut inner) = self.inner.lock() else {
            return 0;
        };
        let cap = inner.buf.len();
        let used = if inner.write >= inner.read {
            inner.write - inner.read
        } else {
            cap - inner.read + inner.write
        };
        let n = data.len().min(cap - used);
        for &b in &data[..n] {
            let w = inner.write;
            inner.buf[w] = b;
            inner.write = (w + 1) % cap;
        }
        n
    }

    fn read_exact(&self, out: &mut [u8]) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        let cap = inner.buf.len();
        let used = if inner.write >= inner.read {
            inner.write - inner.read
        } else {
            cap - inner.read + inner.write
        };
        if used < out.len() {
            return false;
        }
        for b in out.iter_mut() {
            *b = inner.buf[inner.read];
            inner.read = (inner.read + 1) % cap;
        }
        true
    }

    fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.read = inner.write;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_wraps() {
        let r = RingBuffer::new(8);
        assert_eq!(r.write(&[1, 2, 3]), 3);
        let mut out = [0u8; 3];
        assert!(r.read_exact(&mut out));
        assert_eq!(out, [1, 2, 3]);
        assert_eq!(r.write(&[4, 5, 6, 7, 8, 9, 10]), 7);
        let mut out2 = [0u8; 7];
        assert!(r.read_exact(&mut out2));
        assert_eq!(out2, [4, 5, 6, 7, 8, 9, 10]);
    }
}

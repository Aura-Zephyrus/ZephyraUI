//! 解码线程：从 demux 得到的包列表里挑出对应轨道，逐包解码并送进队列。

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
#[cfg(feature = "video")]
use std::sync::Mutex;

#[cfg(feature = "video")]
use crate::clock::PresentationClock;
#[cfg(feature = "audio")]
use crate::demux::audio_codec_from;
#[cfg(feature = "video")]
use crate::demux::video_codec_from;
use crate::demux::MediaPacket;
use crate::pipeline::{is_stop, Command, MediaTrackInfo};

#[cfg(feature = "video")]
use zephyra_video::{
    AlphaMode, DecoderBuffer, Rect, Size, VideoCodec, VideoDecoder, VideoDecoderConfig,
};

/// 消费已排队的命令；返回是否收到 Reset。
fn take_reset(cmd_rx: &Receiver<Command>) -> bool {
    let mut reset = false;
    while let Ok(cmd) = cmd_rx.try_recv() {
        if matches!(cmd, Command::Reset) {
            reset = true;
        }
        // Seek 需要重新 demux，当前一次性载入模型下不支持；忽略但消费掉，
        // 避免旧命令一直堵在 channel 里。
    }
    reset
}

#[cfg(feature = "video")]
pub(crate) fn video_loop(
    packets: Vec<MediaPacket>,
    tracks: Vec<MediaTrackInfo>,
    tx: Sender<crate::video::VideoFrameSource>,
    _clock: Arc<Mutex<PresentationClock>>,
    stop: Arc<AtomicBool>,
    cmd_rx: Receiver<Command>,
) {
    let track = tracks.iter().find(|t| video_codec_from(&t.codec).is_some());
    let Some(track) = track else { return };
    let Some(codec) = video_codec_from(&track.codec) else {
        return;
    };
    let (w, h) = match (track.width, track.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
        _ => return,
    };

    let mut converter = {
        #[cfg(feature = "video")]
        {
            use zephyra_video::h264::{AvcDecoderConfigurationRecord, H264ToAnnexBConverter};
            let cfg = track
                .extra_data
                .as_deref()
                .and_then(AvcDecoderConfigurationRecord::parse);
            cfg.map(H264ToAnnexBConverter::new)
        }
        #[cfg(not(feature = "video"))]
        None
    };
    let mut decoder = make_video_decoder(codec);
    let config = VideoDecoderConfig::new(
        codec,
        zephyra_video::VideoCodecProfile::Unknown,
        zephyra_video::level_8bit_420(),
        AlphaMode::IsOpaque,
        zephyra_video::bt709_color_space(),
        zephyra_video::VideoTransformation::None,
        Size::new(w, h),
        Rect::new(0, 0, w, h),
        Size::new(w, h),
        Vec::new(),
    );
    let init = decoder.initialize(&config, false);
    if init.is_error() {
        return;
    }

    for packet in packets {
        if is_stop(&stop) {
            return;
        }
        if take_reset(&cmd_rx) {
            decoder.reset();
        }
        if packet.track_id != track.id {
            continue;
        }
        let data = if codec == VideoCodec::H264 {
            let out = converter
                .as_mut()
                .and_then(|c| c.convert(&packet.data))
                .unwrap_or_else(|| avcc_to_annexb(&packet.data));
            out
        } else {
            packet.data
        };
        match decoder.decode(&DecoderBuffer::new(data, packet.pts_us, packet.duration_us)) {
            Ok(frames) => {
                for f in frames {
                    if tx.send(crate::video::from_video_frame(f)).is_err() {
                        return;
                    }
                }
            }
            Err(_) => continue,
        }
    }
    // 排空
    if let Ok(frames) = decoder.decode(&DecoderBuffer::end_of_stream(0)) {
        for f in frames {
            let _ = tx.send(crate::video::from_video_frame(f));
        }
    }
}

/// MP4 里的 H.264 是 AVCC（4 字节长度前缀）；解码器要 Annex-B 起始码。
#[cfg(feature = "video")]
fn avcc_to_annexb(data: &[u8]) -> Vec<u8> {
    if data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1]) {
        return data.to_vec();
    }
    let mut out = Vec::with_capacity(data.len());
    let mut pos = 0;
    while pos + 4 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if len == 0 || pos + len > data.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&data[pos..pos + len]);
        pos += len;
    }
    out
}

#[cfg(feature = "video")]
fn make_video_decoder(codec: VideoCodec) -> Box<dyn VideoDecoder> {
    match codec {
        VideoCodec::H264 => Box::new(zephyra_video::OpenH264Decoder::new()),
        VideoCodec::HEVC => Box::new(zephyra_video::HevcDecoder::new()),
        VideoCodec::AV1 => Box::new(zephyra_video::Dav1dDecoder::new()),
        VideoCodec::VP8 => Box::new(zephyra_video::Vpx8Decoder::new()),
        VideoCodec::VP9 => Box::new(zephyra_video::Vpx9Decoder::new()),
        VideoCodec::Vvc => Box::new(zephyra_video::VvdecDecoder::new()),
        VideoCodec::Evc => Box::new(zephyra_video::XevdDecoder::new()),
        VideoCodec::Theora => Box::new(zephyra_video::TheoraDecoder::new()),
        VideoCodec::Mjpeg => Box::new(zephyra_video::MjpegDecoder::new()),
        VideoCodec::Av3 => Box::new(zephyra_video::Uavs3dDecoder::new()),
        _ => Box::new(NullVideoDecoder),
    }
}

#[cfg(feature = "video")]
struct NullVideoDecoder;
#[cfg(feature = "video")]
impl VideoDecoder for NullVideoDecoder {
    fn initialize(&mut self, _: &VideoDecoderConfig, _: bool) -> zephyra_video::DecoderStatus {
        zephyra_video::ok()
    }
    fn decode(
        &mut self,
        _: &DecoderBuffer,
    ) -> Result<Vec<zephyra_video::VideoFrame>, zephyra_video::DecoderStatus> {
        Ok(Vec::new())
    }
    fn reset(&mut self) {}
    fn decoder_type(&self) -> zephyra_video::VideoDecoderType {
        zephyra_video::VideoDecoderType::Unknown
    }
}

#[cfg(feature = "audio")]
pub(crate) fn audio_loop(
    packets: Vec<MediaPacket>,
    tracks: Vec<MediaTrackInfo>,
    tx: Sender<crate::audio_frames::AudioFrame>,
    stop: Arc<AtomicBool>,
    cmd_rx: Receiver<Command>,
) {
    use zephyra_video::{
        AudioBuffer, AudioCodec, AudioDecoder, AudioDecoderConfig, AudioSampleFormat,
    };

    let track = tracks
        .iter()
        .find(|t| t.sample_rate.is_some() && t.channels.is_some());
    let Some(track) = track else { return };
    let rate = track.sample_rate.unwrap() as i32;
    let channels = track.channels.unwrap();

    // AAC 走 symphonia-codec-aac；其余交给 zephyra-video。
    let is_aac = track.codec.to_ascii_lowercase().contains("aac");
    #[cfg(feature = "aac")]
    if is_aac && aac_loop(&packets, track.id, rate, channels, &tx, &stop) {
        return;
    }
    #[cfg(not(feature = "aac"))]
    let _ = is_aac;

    let Some(codec) = audio_codec_from(&track.codec) else {
        return;
    };
    let cfg = AudioDecoderConfig::new(
        codec,
        AudioSampleFormat::Unknown,
        channels,
        rate,
        Vec::new(),
    );
    let mut decoder: Box<dyn AudioDecoder> = match codec {
        AudioCodec::Mp3 => Box::new(zephyra_video::Mp3Decoder::new()),
        AudioCodec::Opus => Box::new(zephyra_video::OpusDecoder::new()),
        AudioCodec::Vorbis => Box::new(zephyra_video::VorbisDecoder::new()),
        AudioCodec::Flac => Box::new(zephyra_video::FlacDecoder::new()),
        AudioCodec::Pcm => Box::new(zephyra_video::PcmDecoder::new()),
        _ => return,
    };
    if decoder.initialize(&cfg).is_error() {
        return;
    }
    for packet in packets {
        if is_stop(&stop) {
            return;
        }
        if take_reset(&cmd_rx) {
            decoder.reset();
        }
        if packet.track_id != track.id {
            continue;
        }
        if let Ok(frames) = decoder.decode(&AudioBuffer::new(
            packet.data,
            packet.pts_us,
            packet.duration_us,
        )) {
            for f in frames {
                if tx.send(f).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(all(feature = "audio", feature = "aac"))]
fn aac_loop(
    packets: &[MediaPacket],
    track_id: u32,
    rate: i32,
    _channels: u16,
    tx: &Sender<crate::audio_frames::AudioFrame>,
    stop: &Arc<AtomicBool>,
) -> bool {
    use symphonia::core::audio::AudioBufferRef;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_AAC};
    use symphonia::core::formats::Packet;
    use zephyra_video::{AudioFrame, AudioSampleFormat};

    let opts = DecoderOptions { verify: false };
    let params = symphonia::core::codecs::CodecParameters {
        codec: CODEC_TYPE_AAC,
        sample_rate: Some(rate as u32),
        channels: Some(
            symphonia::core::audio::Channels::FRONT_LEFT
                | symphonia::core::audio::Channels::FRONT_RIGHT,
        ),
        ..Default::default()
    };
    let mut decoder = match symphonia::default::get_codecs().make(&params, &opts) {
        Ok(d) => d,
        Err(_) => return false,
    };

    for packet in packets {
        if is_stop(stop) {
            return true;
        }
        if packet.track_id != track_id {
            continue;
        }
        let p = Packet::new_from_slice(
            packet.track_id,
            packet.pts_us as u64,
            packet.duration_us as u64,
            &packet.data,
        );
        let decoded = match decoder.decode(&p) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let n_samples = decoded.frames();
        let (spec, planes_f32): (_, Vec<&[f32]>) = match &decoded {
            AudioBufferRef::F32(buf) => (*buf.spec(), buf.planes().planes().to_vec()),
            _ => continue,
        };
        let mut out = Vec::with_capacity(spec.channels.count());
        for src in planes_f32 {
            let mut bytes = Vec::with_capacity(n_samples * 4);
            for i in 0..n_samples {
                let v = src.get(i).copied().unwrap_or(0.0);
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            out.push(bytes);
        }
        if let Some(frame) = AudioFrame::from_planar(
            AudioSampleFormat::PlanarF32,
            spec.channels.count() as u16,
            spec.rate as i32,
            out,
            n_samples,
            packet.pts_us,
            packet.duration_us,
        ) {
            if tx.send(frame).is_err() {
                return true;
            }
        }
    }
    true
}

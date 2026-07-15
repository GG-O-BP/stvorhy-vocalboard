//! 오디오 파일 디코딩 (Symphonia 0.6 — 스펙 §3: AAC-LC 가능,
//! HE-AAC/DRM 불가 시 사용자 안내 오류).

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::sample::Sample;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::ReferenceError;

/// interleaved f32 PCM.
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
}

impl DecodedAudio {
    pub fn duration_ms(&self) -> u32 {
        if self.channels == 0 || self.sample_rate == 0 {
            return 0;
        }
        (self.samples.len() as u64 / self.channels as u64 * 1000 / self.sample_rate as u64) as u32
    }

    /// mono mixdown.
    pub fn to_mono(&self) -> Vec<f32> {
        let ch = self.channels.max(1) as usize;
        self.samples
            .chunks_exact(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect()
    }

    /// 스테레오 (mono면 복제) 분리 채널.
    pub fn to_stereo(&self) -> (Vec<f32>, Vec<f32>) {
        let ch = self.channels.max(1) as usize;
        if ch == 1 {
            return (self.samples.clone(), self.samples.clone());
        }
        let n = self.samples.len() / ch;
        let mut left = Vec::with_capacity(n);
        let mut right = Vec::with_capacity(n);
        for f in self.samples.chunks_exact(ch) {
            left.push(f[0]);
            right.push(f[1]);
        }
        (left, right)
    }
}

fn friendly(e: SymphoniaError, stage: &str) -> ReferenceError {
    let msg = e.to_string();
    let lowered = msg.to_lowercase();
    if lowered.contains("drm") || lowered.contains("encrypt") {
        return ReferenceError::User(
            "DRM 보호된 파일은 임포트할 수 없습니다. DRM 없는 사본을 사용하세요.".into(),
        );
    }
    if lowered.contains("sbr") || lowered.contains("he-aac") || lowered.contains("unsupported") {
        return ReferenceError::User(format!(
            "지원하지 않는 코덱/형식입니다 ({stage}: {msg}). HE-AAC(SBR)·DRM 파일은 지원되지 않습니다 — \
             AAC-LC/MP3/FLAC/WAV/OGG로 변환해 다시 시도하세요."
        ));
    }
    ReferenceError::User(format!("디코딩 실패 ({stage}): {msg}"))
}

/// 파일 전체를 interleaved f32로 디코드한다.
pub fn decode_file(path: &Path) -> Result<DecodedAudio, ReferenceError> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| friendly(e, "컨테이너 인식"))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| ReferenceError::User("오디오 트랙이 없습니다".into()))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| ReferenceError::User("오디오 코덱 파라미터가 없습니다".into()))?;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| friendly(e, "디코더 생성"))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut chunk: Vec<f32> = Vec::new();
    let mut channels: u16 = 0;
    let mut sample_rate: u32 = 0;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(friendly(e, "패킷 읽기")),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let spec = audio_buf.spec();
                channels = spec.channels().count() as u16;
                sample_rate = spec.rate();
                chunk.resize(audio_buf.samples_interleaved(), f32::MID);
                audio_buf.copy_to_slice_interleaved(&mut chunk);
                samples.extend_from_slice(&chunk);
            }
            // 손상 패킷은 건너뛴다 (일반적인 관행).
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(friendly(e, "디코드")),
        }
    }

    if samples.is_empty() || channels == 0 || sample_rate == 0 {
        return Err(ReferenceError::User("디코드된 오디오가 없습니다".into()));
    }
    Ok(DecodedAudio { samples, channels, sample_rate })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_wav(path: &Path, sr: u32, channels: u16, secs: f32) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: sr,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        let n = (sr as f32 * secs) as usize;
        let om = 2.0 * std::f64::consts::PI * 440.0 / sr as f64;
        for i in 0..n {
            let v = (((om * i as f64) % (2.0 * std::f64::consts::PI)).sin() * 0.4 * 32767.0) as i16;
            for _ in 0..channels {
                w.write_sample(v).unwrap();
            }
        }
        w.finalize().unwrap();
    }

    #[test]
    fn decodes_wav_stereo() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.wav");
        write_wav(&p, 44_100, 2, 1.0);
        let d = decode_file(&p).unwrap();
        assert_eq!(d.channels, 2);
        assert_eq!(d.sample_rate, 44_100);
        assert!((d.duration_ms() as i64 - 1000).abs() < 50, "{}", d.duration_ms());
        let (l, r) = d.to_stereo();
        assert_eq!(l.len(), r.len());
        let mono = d.to_mono();
        assert_eq!(mono.len(), l.len());
        // 진폭 확인.
        let peak = mono.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!((peak - 0.4).abs() < 0.02, "{peak}");
    }

    #[test]
    fn missing_file_errors() {
        assert!(decode_file(Path::new("no/such/file.mp3")).is_err());
    }

    #[test]
    fn garbage_file_gives_user_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("junk.mp4");
        std::fs::write(&p, vec![0u8; 4096]).unwrap();
        match decode_file(&p) {
            Err(ReferenceError::User(m)) => assert!(!m.is_empty()),
            other => panic!("expected user error, got {other:?}"),
        }
    }
}

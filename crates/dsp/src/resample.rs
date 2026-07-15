//! 16kHz 변환 전면 인터페이스.
//!
//! 정수비(48k/32k/96k…)는 FIR 데시메이터, 비정수(44.1k 등)는 rubato Fft
//! (FixedSync::Input, 스트리밍) 경로 (스펙 §3).

use rubato::audioadapter_buffers::direct::SequentialSlice;
use rubato::{Fft, FixedSync, Resampler};

use crate::decimate::FirDecimator;
use crate::{DspError, ANALYSIS_SR};

const RUBATO_CHUNK: usize = 1024;

pub struct RubatoResampler {
    rs: Fft<f32>,
    staging: Vec<f32>,
    tmp: Vec<f32>,
}

impl RubatoResampler {
    pub fn new(in_sr: u32) -> Result<Self, DspError> {
        let rs = Fft::<f32>::new(in_sr as usize, ANALYSIS_SR as usize, RUBATO_CHUNK, 1, FixedSync::Input)
            .map_err(|e| DspError::Resample(e.to_string()))?;
        Ok(Self {
            rs,
            staging: Vec::with_capacity(RUBATO_CHUNK * 2),
            tmp: Vec::new(),
        })
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), DspError> {
        self.staging.extend_from_slice(input);
        let chunk = self.rs.input_frames_next();
        while self.staging.len() >= chunk {
            let cap = self.rs.output_frames_next();
            self.tmp.resize(cap, 0.0);
            let in_adapter = SequentialSlice::new(&self.staging[..chunk], 1, chunk)
                .map_err(|e| DspError::Resample(e.to_string()))?;
            let mut out_adapter = SequentialSlice::new_mut(&mut self.tmp, 1, cap)
                .map_err(|e| DspError::Resample(e.to_string()))?;
            let (used, written) = self
                .rs
                .process_into_buffer(&in_adapter, &mut out_adapter, None)
                .map_err(|e| DspError::Resample(e.to_string()))?;
            out.extend_from_slice(&self.tmp[..written]);
            self.staging.drain(..used);
        }
        Ok(())
    }
}

/// 전체 클립을 임의 SR 쌍으로 일괄 변환한다 (mono, 오프라인 용도 —
/// 재생 프리롤/레퍼런스 준비). rubato Fft의 지연 트리밍 포함.
pub fn resample_all(input: &[f32], from: u32, to: u32) -> Result<Vec<f32>, DspError> {
    if from == to || input.is_empty() {
        return Ok(input.to_vec());
    }
    let mut rs = Fft::<f32>::new(from as usize, to as usize, RUBATO_CHUNK, 1, FixedSync::Input)
        .map_err(|e| DspError::Resample(e.to_string()))?;
    let expected = (input.len() as f64 * to as f64 / from as f64).ceil() as usize;
    let mut out = vec![0.0f32; expected + RUBATO_CHUNK * 4];
    let in_adapter = SequentialSlice::new(input, 1, input.len())
        .map_err(|e| DspError::Resample(e.to_string()))?;
    let cap = out.len();
    let mut out_adapter = SequentialSlice::new_mut(&mut out, 1, cap)
        .map_err(|e| DspError::Resample(e.to_string()))?;
    let (_used, written) = rs
        .process_all_into_buffer(&in_adapter, &mut out_adapter, input.len(), None)
        .map_err(|e| DspError::Resample(e.to_string()))?;
    out.truncate(written);
    Ok(out)
}

/// 입력 SR → 16k 변환기 (경로 자동 선택).
pub enum To16k {
    Passthrough,
    Fir(FirDecimator),
    Rubato(RubatoResampler),
}

impl To16k {
    pub fn new(in_sr: u32) -> Result<Self, DspError> {
        if in_sr == 0 {
            return Err(DspError::UnsupportedSampleRate(0));
        }
        if in_sr == ANALYSIS_SR {
            Ok(Self::Passthrough)
        } else if in_sr % ANALYSIS_SR == 0 {
            Ok(Self::Fir(FirDecimator::new(in_sr, ANALYSIS_SR)?))
        } else {
            Ok(Self::Rubato(RubatoResampler::new(in_sr)?))
        }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), DspError> {
        match self {
            Self::Passthrough => {
                out.extend_from_slice(input);
                Ok(())
            }
            Self::Fir(d) => {
                d.process(input, out);
                Ok(())
            }
            Self::Rubato(r) => r.process(input, out),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{AcfEngine, InferenceEngine};

    fn sine(freq: f32, sr: u32, n: usize) -> Vec<f32> {
        let w = 2.0 * std::f64::consts::PI * freq as f64 / sr as f64;
        (0..n)
            .map(|i| ((w * i as f64) % (2.0 * std::f64::consts::PI)).sin() as f32)
            .collect()
    }

    #[test]
    fn rubato_44100_to_16k_preserves_pitch() {
        let mut c = To16k::new(44_100).unwrap();
        assert!(matches!(c, To16k::Rubato(_)));
        let x = sine(1000.0, 44_100, 44_100 * 2);
        let mut y = Vec::new();
        for chunk in x.chunks(1000) {
            c.process(chunk, &mut y).unwrap();
        }
        // 길이 비율 검증 (리샘플러 지연 허용).
        let expected = (44_100.0 * 2.0 * 16_000.0 / 44_100.0) as isize;
        assert!(
            (y.len() as isize - expected).abs() < 4096,
            "len {} vs {expected}",
            y.len()
        );
        // 변환 후 피치가 1kHz로 유지되는지 ACF로 확인 (정착 구간).
        let settled = &y[y.len() / 2..y.len() / 2 + 4096];
        let est = AcfEngine::new().infer(settled).unwrap();
        let last = est.last().unwrap();
        let cents = 1200.0 * (last.f0 / 1000.0).log2();
        assert!(cents.abs() < 30.0, "f0={} ({cents:+.1}c)", last.f0);
    }

    #[test]
    fn resample_all_preserves_pitch_and_length() {
        let x = sine(440.0, 44_100, 44_100); // 1초
        let y = resample_all(&x, 44_100, 48_000).unwrap();
        let expected = 48_000;
        assert!(
            (y.len() as isize - expected).abs() < 2048,
            "len {} vs {expected}",
            y.len()
        );
        let settled = &y[y.len() / 4..y.len() / 4 + 8192];
        // 48k 신호의 피치 확인은 16k로 내린 뒤 ACF로.
        let mut d = crate::decimate::FirDecimator::new(48_000, 16_000).unwrap();
        let mut z = Vec::new();
        d.process(settled, &mut z);
        let est = AcfEngine::new().infer(&z).unwrap();
        let last = est.last().unwrap();
        let cents = 1200.0 * (last.f0 / 440.0).log2();
        assert!(cents.abs() < 30.0, "f0={} ({cents:+.1}c)", last.f0);
    }

    #[test]
    fn selects_fir_for_integer_ratios() {
        assert!(matches!(To16k::new(48_000).unwrap(), To16k::Fir(_)));
        assert!(matches!(To16k::new(96_000).unwrap(), To16k::Fir(_)));
        assert!(matches!(To16k::new(16_000).unwrap(), To16k::Passthrough));
        assert!(matches!(To16k::new(44_100).unwrap(), To16k::Rubato(_)));
    }
}

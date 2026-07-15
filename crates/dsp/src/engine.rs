//! 추론 엔진 추상화 (스펙 §3: 추론은 `InferenceEngine` trait 뒤에, ort 구현 주입).

use crate::{DspError, ANALYSIS_SR, FMAX, FMIN, HOP};

/// hop 하나에 대한 (f0, confidence) 추정.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchEstimate {
    pub f0: f32,
    pub confidence: f32,
}

/// 16kHz mono 오디오에서 hop(256)당 피치를 추정한다.
///
/// 계약: 입력 길이 N(≥HOP)에 대해 `N / HOP`개(내림)의 추정을 반환한다.
/// k번째 추정의 중심 시각은 대략 `(k·256 + 128) / 16000`초.
/// 구현은 스트리밍 상태를 가질 수 없다 — 같은 입력이면 같은 출력
/// (파이프라인이 슬라이딩 윈도우를 관리한다).
pub trait InferenceEngine: Send {
    fn infer(&mut self, audio_16k: &[f32]) -> Result<Vec<PitchEstimate>, DspError>;
}

/// 정규화 자기상관(ACF) 기준선 엔진.
///
/// SwiftF0의 대체가 아니라 (a) 모델 없이 파이프라인을 테스트하고
/// (b) A/B 검증의 보조 축으로 쓰는 참조 구현이다. 클린 톤에서만 신뢰 가능.
pub struct AcfEngine {
    window: usize,
}

impl AcfEngine {
    pub fn new() -> Self {
        Self { window: 1024 }
    }
}

impl Default for AcfEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl InferenceEngine for AcfEngine {
    fn infer(&mut self, audio_16k: &[f32]) -> Result<Vec<PitchEstimate>, DspError> {
        let n_hops = audio_16k.len() / HOP;
        let min_lag = (ANALYSIS_SR as f32 / FMAX).floor() as usize; // ≈7
        let max_lag = (ANALYSIS_SR as f32 / FMIN).ceil() as usize; // ≈342
        let mut out = Vec::with_capacity(n_hops);
        for k in 0..n_hops {
            let end = (k + 1) * HOP;
            let start = end.saturating_sub(self.window);
            let w = &audio_16k[start..end];
            out.push(acf_estimate(w, min_lag, max_lag));
        }
        Ok(out)
    }
}

fn acf_estimate(w: &[f32], min_lag: usize, max_lag: usize) -> PitchEstimate {
    let n = w.len();
    if n < min_lag * 2 {
        return PitchEstimate { f0: 0.0, confidence: 0.0 };
    }
    let max_lag = max_lag.min(n / 2);
    let mut r_max = 0.0f32;
    let mut r_at = vec![0.0f32; max_lag + 2];
    for lag in min_lag..=max_lag {
        let m = n - lag;
        let mut num = 0.0f32;
        let mut e0 = 0.0f32;
        let mut e1 = 0.0f32;
        for i in 0..m {
            num += w[i] * w[i + lag];
            e0 += w[i] * w[i];
            e1 += w[i + lag] * w[i + lag];
        }
        let denom = (e0 * e1).sqrt();
        let r = if denom > 1e-12 { num / denom } else { 0.0 };
        r_at[lag] = r;
        if r > r_max {
            r_max = r;
        }
    }
    if r_max <= 0.0 {
        return PitchEstimate { f0: 0.0, confidence: 0.0 };
    }
    // 주기 신호는 배수 lag(서브하모닉)에서도 r≈max가 나온다. 전역 최대의
    // 90% 이상인 첫 국소 극대(=가장 짧은 주기)를 채택한다.
    let mut best_lag = 0usize;
    let mut best_r = 0.0f32;
    for lag in (min_lag + 1)..max_lag {
        let r = r_at[lag];
        if r >= 0.9 * r_max && r >= r_at[lag - 1] && r >= r_at[lag + 1] {
            best_lag = lag;
            best_r = r;
            break;
        }
    }
    if best_lag == 0 {
        return PitchEstimate { f0: 0.0, confidence: 0.0 };
    }
    // 파라볼릭 보간으로 서브샘플 lag 정밀화.
    let refined = if best_lag > min_lag && best_lag < max_lag {
        let (a, b, c) = (r_at[best_lag - 1], r_at[best_lag], r_at[best_lag + 1]);
        let denom = a - 2.0 * b + c;
        if denom.abs() > 1e-9 {
            best_lag as f32 + 0.5 * (a - c) / denom
        } else {
            best_lag as f32
        }
    } else {
        best_lag as f32
    };
    PitchEstimate {
        f0: ANALYSIS_SR as f32 / refined,
        confidence: best_r.clamp(0.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acf_finds_sine_pitch() {
        let mut e = AcfEngine::new();
        let sr = ANALYSIS_SR as f32;
        let x: Vec<f32> = (0..4096)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr).sin())
            .collect();
        let est = e.infer(&x).unwrap();
        assert_eq!(est.len(), 16);
        let last = est.last().unwrap();
        let cents = 1200.0 * (last.f0 / 440.0).log2();
        assert!(cents.abs() < 30.0, "f0={} ({cents:+.1}c)", last.f0);
        assert!(last.confidence > 0.9);
    }

    #[test]
    fn acf_silence_has_zero_confidence() {
        let mut e = AcfEngine::new();
        let est = e.infer(&vec![0.0; 2048]).unwrap();
        assert!(est.iter().all(|p| p.confidence == 0.0 && p.f0 == 0.0));
    }
}

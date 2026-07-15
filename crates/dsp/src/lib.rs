//! 실시간 보컬 DSP 파이프라인 (스펙 §2·§4·§5).
//!
//! 흐름: (48k mono) → biquad HPF 70Hz → 48k→16k 데시메이션 → hop(256) 단위
//! RMS 게이트 → SwiftF0 추론(`InferenceEngine`) → `PitchFrame` 조립.
//! 이 크레이트는 오디오 콜백이 아니라 전용 DSP 스레드에서 돌며, 게이트 미달
//! hop도 unvoiced 프레임을 반드시 방출한다 (62.5Hz 결번 금지).

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod decimate;
pub mod engine;
pub mod gate;
pub mod hpf;
pub mod ort_engine;
pub mod pipeline;
pub mod pitch;
pub mod resample;

pub use engine::{AcfEngine, InferenceEngine, PitchEstimate};
pub use ort_engine::OrtEngine;
pub use pipeline::{PipelineParams, PitchPipeline};

/// 분석 샘플레이트 (SwiftF0 요구).
pub const ANALYSIS_SR: u32 = 16_000;
/// hop 크기 @16k = 16ms = 62.5Hz.
pub const HOP: usize = 256;
pub const HOP_MS: u32 = 16;
/// SwiftF0 모델 피치 범위.
pub const FMIN: f32 = 46.875;
pub const FMAX: f32 = 2093.75;

#[derive(Debug, Error)]
pub enum DspError {
    #[error("unsupported input sample rate {0}")]
    UnsupportedSampleRate(u32),
    #[error("inference: {0}")]
    Inference(String),
    #[error("resample: {0}")]
    Resample(String),
    #[error("filter design: {0}")]
    FilterDesign(String),
}

/// Channel(62.5Hz)로 WebView에 보내는 프레임 (스펙 §5, serde).
/// 프론트 계약: src/lib/types.js 의 @typedef PitchFrame 와 동기 유지.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PitchFrame {
    /// 세션 시작 기준 ms (인덱스×16ms).
    pub t: u32,
    /// Hz. unvoiced(게이트 미달)면 0.
    pub f0: f32,
    /// MIDI 실수값 (A4=440 고정, 12평균율).
    pub midi: f32,
    /// 최근접 반음 대비 편차 [-50,+50).
    pub cents: f32,
    /// SwiftF0 confidence [0,1]. voicing 판단이 아님.
    pub confidence: f32,
    /// 입력 RMS dBFS ([-96,0]로 클램프 — JSON 직렬화에 -inf 금지).
    pub rms: f32,
    /// RMS 게이트 AND confidence 임계.
    pub voiced: bool,
}

impl PitchFrame {
    /// 게이트 미달 hop의 무성 프레임 (f0/midi/cents=0·confidence=0, rms 실측).
    pub fn unvoiced(t: u32, rms_dbfs: f32) -> Self {
        Self {
            t,
            f0: 0.0,
            midi: 0.0,
            cents: 0.0,
            confidence: 0.0,
            rms: rms_dbfs,
            voiced: false,
        }
    }
}

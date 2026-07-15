//! PitchFrame 조립 파이프라인: HPF → 16k 변환 → hop 게이트 → 추론 → 프레임.
//!
//! DSP 스레드 전용 (오디오 콜백 아님 — 여기서는 할당 허용). 모든 hop은
//! 프레임을 방출한다: 게이트 미달이어도 unvoiced 프레임 (62.5Hz 결번 금지).

use crate::engine::{InferenceEngine, PitchEstimate};
use crate::gate::rms_dbfs;
use crate::hpf::HighPass;
use crate::pitch::{f0_to_midi, midi_to_cents};
use crate::resample::To16k;
use crate::{DspError, PitchFrame, FMAX, FMIN, HOP, HOP_MS};

/// §4 파라미터 (설정화 항목은 configure 커맨드로 갱신).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PipelineParams {
    /// confidence 임계 (기본 0.9).
    pub conf_threshold: f32,
    /// RMS 게이트 임계 dBFS (기본 -45).
    pub gate_dbfs: f32,
    /// 추론 슬라이딩 윈도우 크기 (hop 수, 기본 12 = 3072 샘플 = 192ms).
    ///
    /// 실측(SwiftF0): 왼쪽 문맥이 짧으면 confidence가 임계(0.9) 아래로
    /// 떨어진다. 12홉에서 꼬리-1 프레임 conf ≈ 0.97.
    pub window_hops: usize,
}

impl Default for PipelineParams {
    fn default() -> Self {
        Self {
            conf_threshold: 0.9,
            gate_dbfs: -45.0,
            window_hops: 12,
        }
    }
}

/// 윈도우 마지막에서 몇 hop 물러난 프레임을 채택할지.
///
/// 실측: 마지막 프레임은 오른쪽 문맥이 그래프 내부 패딩이라 conf가
/// ~0.93까지 떨어진다. 1홉 백오프(+16ms 지연)로 conf ~0.97 확보.
/// E2E 지연 ≈ 캡처 10 + hop 16 + 프레임 중심 오프셋 24 + 추론/IPC/렌더
/// ≈ 60~70ms — §4 예산(60–80ms) 내.
const PICK_BACK: usize = 1;

pub struct PitchPipeline {
    hpf: HighPass,
    to16k: To16k,
    engine: Box<dyn InferenceEngine>,
    params: PipelineParams,
    /// HPF 적용용 입력 복사 버퍼.
    scratch_in: Vec<f32>,
    /// 16k 변환 결과 대기열.
    buf16k: Vec<f32>,
    /// 추론 슬라이딩 윈도우 (window_hops×HOP, 시작 시 0 채움).
    window: Vec<f32>,
    frame_idx: u64,
}

impl PitchPipeline {
    pub fn new(
        input_sr: u32,
        engine: Box<dyn InferenceEngine>,
        params: PipelineParams,
    ) -> Result<Self, DspError> {
        Ok(Self {
            hpf: HighPass::new(input_sr)?,
            to16k: To16k::new(input_sr)?,
            engine,
            params,
            scratch_in: Vec::with_capacity(8192),
            buf16k: Vec::with_capacity(8192),
            window: vec![0.0; params.window_hops.max(1) * HOP],
            frame_idx: 0,
        })
    }

    /// 지금까지 방출한 프레임 수.
    pub fn frames_emitted(&self) -> u64 {
        self.frame_idx
    }

    pub fn params(&self) -> PipelineParams {
        self.params
    }

    pub fn set_params(&mut self, p: PipelineParams) {
        if p.window_hops != self.params.window_hops {
            self.window = vec![0.0; p.window_hops.max(1) * HOP];
        }
        self.params = p;
    }

    /// mono 입력 청크를 처리하고 완성된 프레임들을 `out`에 push한다.
    pub fn process(&mut self, mono: &[f32], out: &mut Vec<PitchFrame>) -> Result<(), DspError> {
        self.scratch_in.clear();
        self.scratch_in.extend_from_slice(mono);
        self.hpf.process(&mut self.scratch_in);
        // borrow 분리를 위해 잠시 소유권 이동 없이 직접 호출.
        {
            let scratch = std::mem::take(&mut self.scratch_in);
            let r = self.to16k.process(&scratch, &mut self.buf16k);
            self.scratch_in = scratch;
            r?;
        }

        let mut consumed = 0;
        while self.buf16k.len() - consumed >= HOP {
            let hop = &self.buf16k[consumed..consumed + HOP];
            let t = (self.frame_idx * HOP_MS as u64) as u32;
            let rms = rms_dbfs(hop);

            // 윈도우 시프트 + hop 추가.
            let w = self.window.len();
            self.window.copy_within(HOP..w, 0);
            self.window[w - HOP..].copy_from_slice(hop);

            if rms < self.params.gate_dbfs {
                // 게이트 미달: 추론 생략 가능. 프레임은 반드시 방출.
                out.push(PitchFrame::unvoiced(t, rms));
            } else {
                let est = self.engine.infer(&self.window)?;
                let picked = est
                    .len()
                    .checked_sub(1 + PICK_BACK)
                    .and_then(|i| est.get(i))
                    .or_else(|| est.last())
                    .copied()
                    .unwrap_or(PitchEstimate { f0: 0.0, confidence: 0.0 });
                out.push(assemble(t, rms, picked, &self.params));
            }
            self.frame_idx += 1;
            consumed += HOP;
        }
        self.buf16k.drain(..consumed);
        Ok(())
    }
}

/// 게이트 통과 hop의 프레임 조립 (§5): confidence 미달이어도 추론값을
/// 채우되 voiced=false. f0가 0 이하이거나 모델 범위 밖이면 voiced=false.
fn assemble(t: u32, rms: f32, est: PitchEstimate, params: &PipelineParams) -> PitchFrame {
    let usable = est.f0.is_finite() && est.f0 > 0.0;
    let (f0, midi, cents) = if usable {
        let m = f0_to_midi(est.f0);
        (est.f0, m, midi_to_cents(m))
    } else {
        (0.0, 0.0, 0.0)
    };
    let in_range = (FMIN..=FMAX).contains(&f0);
    PitchFrame {
        t,
        f0,
        midi,
        cents,
        confidence: est.confidence,
        rms,
        voiced: usable && in_range && est.confidence >= params.conf_threshold,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::AcfEngine;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn sine(freq: f32, sr: u32, n: usize, amp: f32) -> Vec<f32> {
        let w = 2.0 * std::f64::consts::PI * freq as f64 / sr as f64;
        (0..n)
            .map(|i| amp * ((w * i as f64) % (2.0 * std::f64::consts::PI)).sin() as f32)
            .collect()
    }

    /// 호출 횟수를 세고 고정값을 돌려주는 스텁 엔진.
    struct StubEngine {
        calls: Arc<AtomicUsize>,
        f0: f32,
        conf: f32,
    }

    impl InferenceEngine for StubEngine {
        fn infer(&mut self, audio: &[f32]) -> Result<Vec<PitchEstimate>, DspError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![
                PitchEstimate {
                    f0: self.f0,
                    confidence: self.conf
                };
                audio.len() / HOP
            ])
        }
    }

    #[test]
    fn emits_frame_every_hop_without_gaps() {
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = StubEngine { calls, f0: 220.0, conf: 1.0 };
        let mut p = PitchPipeline::new(48_000, Box::new(engine), PipelineParams::default()).unwrap();
        let x = sine(220.0, 48_000, 48_000, 0.5); // 1초
        let mut frames = Vec::new();
        // 불규칙 청크로 공급해도 프레임 시간축은 결번 없이 이어져야 한다.
        for chunk in x.chunks(777) {
            p.process(chunk, &mut frames).unwrap();
        }
        assert!(frames.len() >= 60, "got {}", frames.len());
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(f.t, i as u32 * 16, "gap at {i}");
        }
    }

    #[test]
    fn gate_skips_inference_but_still_emits_unvoiced() {
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = StubEngine { calls: calls.clone(), f0: 440.0, conf: 1.0 };
        let mut p = PitchPipeline::new(48_000, Box::new(engine), PipelineParams::default()).unwrap();
        let x = vec![0.0f32; 48_000]; // 1초 무음
        let mut frames = Vec::new();
        p.process(&x, &mut frames).unwrap();
        assert!(frames.len() >= 60);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "게이트 미달인데 추론 호출됨");
        for f in &frames {
            assert!(!f.voiced);
            assert_eq!(f.f0, 0.0);
            assert_eq!(f.confidence, 0.0);
            assert_eq!(f.rms, -96.0);
        }
    }

    #[test]
    fn low_confidence_fills_values_but_not_voiced() {
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = StubEngine { calls, f0: 500.0, conf: 0.5 };
        let mut p = PitchPipeline::new(48_000, Box::new(engine), PipelineParams::default()).unwrap();
        let x = sine(500.0, 48_000, 48_000, 0.5);
        let mut frames = Vec::new();
        p.process(&x, &mut frames).unwrap();
        let f = frames.last().unwrap();
        assert!(!f.voiced);
        assert_eq!(f.f0, 500.0);
        // 500Hz = midi 71.213 (B4 +21.3c)
        assert!((f.midi - 71.213).abs() < 0.01, "midi {}", f.midi);
        assert!((f.cents - 21.3).abs() < 1.0, "cents {}", f.cents);
        assert_eq!(f.confidence, 0.5);
    }

    #[test]
    fn out_of_range_f0_is_not_voiced() {
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = StubEngine { calls, f0: 4000.0, conf: 0.99 };
        let mut p = PitchPipeline::new(48_000, Box::new(engine), PipelineParams::default()).unwrap();
        let x = sine(440.0, 48_000, 24_000, 0.5);
        let mut frames = Vec::new();
        p.process(&x, &mut frames).unwrap();
        assert!(frames.iter().all(|f| !f.voiced));
    }

    #[test]
    fn end_to_end_sine_with_acf_baseline() {
        let mut p = PitchPipeline::new(
            48_000,
            Box::new(AcfEngine::new()),
            PipelineParams { conf_threshold: 0.8, ..Default::default() },
        )
        .unwrap();
        let x = sine(440.0, 48_000, 96_000, 0.1414); // 2초, ≈-20dBFS
        let mut frames = Vec::new();
        p.process(&x, &mut frames).unwrap();
        let settled = &frames[20..];
        let voiced_count = settled.iter().filter(|f| f.voiced).count();
        assert!(voiced_count * 10 >= settled.len() * 9, "{voiced_count}/{}", settled.len());
        for f in settled.iter().filter(|f| f.voiced) {
            assert!((f.midi - 69.0).abs() < 0.5, "midi {} at t={}", f.midi, f.t);
        }
    }

    #[test]
    fn gate_threshold_update_applies() {
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = StubEngine { calls: calls.clone(), f0: 440.0, conf: 1.0 };
        let mut p = PitchPipeline::new(48_000, Box::new(engine), PipelineParams::default()).unwrap();
        let x = sine(440.0, 48_000, 48_000, 0.001); // ≈-63dBFS → 기본 게이트 미달
        let mut frames = Vec::new();
        p.process(&x, &mut frames).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // 게이트를 -80으로 낮추면 추론이 돌기 시작한다.
        p.set_params(PipelineParams { gate_dbfs: -80.0, ..Default::default() });
        p.process(&x, &mut frames).unwrap();
        assert!(calls.load(Ordering::SeqCst) > 0);
    }
}

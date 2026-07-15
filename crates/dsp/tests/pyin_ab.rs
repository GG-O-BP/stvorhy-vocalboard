//! pYIN A/B 기준선 (스펙 §3: 검증용, viterbi 스무딩 포함 보컬 F0 레퍼런스).
//!
//! 목적:
//! 1. 캡처 전처리(HPF 70Hz + 48k→16k 데시메이션)가 피치를 옮기지 않음을
//!    pyin으로 ±3 cents 수준에서 고정 — SwiftF0 통합 테스트의 완화 허용치
//!    (±25c)가 모델 고유 편향이지 파이프라인 결함이 아님을 입증.
//! 2. 같은 신호에 대한 SwiftF0ㅡpyin 편차를 A/B로 기록.

use pyin::{Framing, PYINExecutor, PadMode};
use vocalboard_dsp::hpf::HighPass;
use vocalboard_dsp::resample::To16k;

fn sine(freq: f64, sr: u32, n: usize, amp: f32) -> Vec<f32> {
    let w = 2.0 * std::f64::consts::PI * freq / sr as f64;
    (0..n)
        .map(|i| amp * ((w * i as f64) % (2.0 * std::f64::consts::PI)).sin() as f32)
        .collect()
}

fn cents(f0: f64, reference: f64) -> f64 {
    1200.0 * (f0 / reference).log2()
}

/// 캡처 전처리를 통과시킨 16k 신호.
fn preprocess_48k_to_16k(input: &[f32]) -> Vec<f32> {
    let mut hpf = HighPass::new(48_000).unwrap();
    let mut x = input.to_vec();
    hpf.process(&mut x);
    let mut to16k = To16k::new(48_000).unwrap();
    let mut y = Vec::new();
    to16k.process(&x, &mut y).unwrap();
    y
}

fn pyin_median_f0(signal_16k: &[f32]) -> f64 {
    let mut ex = PYINExecutor::<f32>::new(60.0, 1200.0, 16_000, 2048, None, None, None);
    let (_ts, f0, voiced, _prob) =
        ex.pyin(signal_16k, f32::NAN, Framing::Center(PadMode::Constant(0.0)));
    let mut voiced_f0: Vec<f64> = f0
        .iter()
        .zip(voiced.iter())
        .filter(|(_, v)| **v)
        .map(|(f, _)| *f as f64)
        .collect();
    assert!(
        voiced_f0.len() * 2 > f0.len(),
        "pyin voiced 프레임이 과반이어야 함 ({}/{})",
        voiced_f0.len(),
        f0.len()
    );
    voiced_f0.sort_by(|a, b| a.total_cmp(b));
    voiced_f0[voiced_f0.len() / 2]
}

/// HPF+데시메이터가 피치를 보존한다 (±3 cents).
#[test]
fn preprocessing_is_pitch_transparent_by_pyin() {
    for freq in [110.0f64, 220.0, 440.0, 880.0] {
        let x = sine(freq, 48_000, 96_000, 0.25);
        let y = preprocess_48k_to_16k(&x);
        // 필터 정착 구간 제외.
        let settled = &y[8_000..];
        let f0 = pyin_median_f0(settled);
        let dev = cents(f0, freq);
        assert!(
            dev.abs() <= 3.0,
            "{freq}Hz: pyin {f0:.3}Hz ({dev:+.2}c) — 전처리가 피치를 옮김"
        );
    }
}

/// SwiftF0 vs pyin A/B — 동일 16k 신호에 대한 편차 기록.
/// (pyin이 ±3c 안이면 SwiftF0-pyin 차이는 곧 모델 편향이다.)
#[test]
fn swiftf0_vs_pyin_ab_on_clean_sine() {
    let model = {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../models/swift_f0.onnx");
        if !p.exists() {
            eprintln!("SKIP: SwiftF0 model not found — pyin 단독 결과만 검증됨");
            return;
        }
        p
    };
    use vocalboard_dsp::engine::InferenceEngine;
    let mut engine = vocalboard_dsp::OrtEngine::from_file(&model).unwrap();

    for freq in [220.0f64, 440.0, 880.0] {
        let x = sine(freq, 48_000, 96_000, 0.25);
        let y = preprocess_48k_to_16k(&x);
        let settled = &y[8_000..];

        let pyin_f0 = pyin_median_f0(settled);
        let est = engine.infer(settled).unwrap();
        let mid = &est[est.len() / 4..est.len() * 3 / 4];
        let swift_f0 = (mid.iter().map(|p| p.f0 as f64).sum::<f64>() / mid.len() as f64).max(1e-9);

        let pyin_dev = cents(pyin_f0, freq);
        let swift_dev = cents(swift_f0, freq);
        eprintln!(
            "A/B {freq:>6.1}Hz: pyin {pyin_f0:>8.2}Hz ({pyin_dev:+.2}c) | swiftf0 {swift_f0:>8.2}Hz ({swift_dev:+.2}c)"
        );
        assert!(pyin_dev.abs() <= 3.0, "pyin 기준선 이탈: {pyin_dev:+.2}c");
        // 모델 편향 실측 상한 (±13c 관측) + 여유.
        assert!(
            (swift_dev - pyin_dev).abs() <= 25.0,
            "SwiftF0-pyin 편차 과대: {:.2}c",
            swift_dev - pyin_dev
        );
    }
}

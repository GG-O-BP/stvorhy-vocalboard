//! 정수비 FIR 데시메이터 (48k→16k 등). Kaiser 윈도 sinc 설계.
//!
//! 통과대역 0–7kHz, 저지대역 8kHz(출력 나이퀴스트)부터, 저지 감쇠 ~70dB.
//! 비정수 비율은 [`crate::resample`]의 rubato 경로를 쓴다.

use crate::DspError;

/// 0차 제1종 변형 베셀 함수 (Kaiser 윈도용, 급수 전개).
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let half_x = x / 2.0;
    for k in 1..=30 {
        term *= (half_x / k as f64) * (half_x / k as f64);
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
    }
    sum
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
    }
}

/// Kaiser 윈도 sinc 저역통과 탭 설계.
/// `cutoff`는 입력 SR 대비 정규화 주파수 (0..0.5), `atten_db`는 저지 감쇠,
/// `trans`는 정규화 천이 대역폭.
fn design_lowpass(cutoff: f64, atten_db: f64, trans: f64) -> Vec<f32> {
    let beta = if atten_db > 50.0 {
        0.1102 * (atten_db - 8.7)
    } else if atten_db >= 21.0 {
        0.5842 * (atten_db - 21.0).powf(0.4) + 0.07886 * (atten_db - 21.0)
    } else {
        0.0
    };
    let delta_omega = 2.0 * std::f64::consts::PI * trans;
    let mut n = ((atten_db - 7.95) / (2.285 * delta_omega)).ceil() as usize + 1;
    if n % 2 == 0 {
        n += 1; // 선형 위상 대칭을 위해 홀수 탭
    }
    let m = (n - 1) as f64;
    let i0_beta = bessel_i0(beta);
    let mut taps = Vec::with_capacity(n);
    for i in 0..n {
        let x = i as f64 - m / 2.0;
        let w = bessel_i0(beta * (1.0 - (2.0 * i as f64 / m - 1.0).powi(2)).max(0.0).sqrt()) / i0_beta;
        taps.push(2.0 * cutoff * sinc(2.0 * cutoff * x) * w);
    }
    // DC 이득 1로 정규화.
    let sum: f64 = taps.iter().sum();
    taps.iter().map(|t| (t / sum) as f32).collect()
}

/// 스트리밍 정수비 데시메이터.
pub struct FirDecimator {
    taps: Vec<f32>,
    ratio: usize,
    /// 미처리 입력 (탭 길이-1 만큼의 이력 + 나머지).
    buf: Vec<f32>,
}

impl FirDecimator {
    pub fn new(in_sr: u32, out_sr: u32) -> Result<Self, DspError> {
        if in_sr == 0 || out_sr == 0 || in_sr % out_sr != 0 || in_sr == out_sr {
            return Err(DspError::UnsupportedSampleRate(in_sr));
        }
        let ratio = (in_sr / out_sr) as usize;
        // 통과 7kHz / 저지 8kHz를 입력 SR로 정규화.
        let cutoff = 7_500.0 / in_sr as f64;
        let trans = 1_000.0 / in_sr as f64;
        let taps = design_lowpass(cutoff, 70.0, trans);
        Ok(Self {
            taps,
            ratio,
            buf: Vec::with_capacity(8192),
        })
    }

    pub fn ratio(&self) -> usize {
        self.ratio
    }

    /// 설계된 탭 (주파수 응답 검증용).
    pub fn taps(&self) -> &[f32] {
        &self.taps
    }

    /// 입력을 소비하고 데시메이트된 샘플을 `out`에 push한다.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.buf.extend_from_slice(input);
        let n = self.taps.len();
        if self.buf.len() < n {
            return;
        }
        let count = (self.buf.len() - n) / self.ratio + 1;
        for k in 0..count {
            let base = k * self.ratio;
            let mut acc = 0.0f32;
            for (j, t) in self.taps.iter().enumerate() {
                acc += t * self.buf[base + j];
            }
            out.push(acc);
        }
        self.buf.drain(..count * self.ratio);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// f64 위상 누적 사인 (f32 위상은 수만 라디안에서 -50dB대 노이즈를 만든다).
    fn sine(freq: f32, sr: u32, n: usize) -> Vec<f32> {
        let w = 2.0 * std::f64::consts::PI * freq as f64 / sr as f64;
        (0..n)
            .map(|i| ((w * i as f64) % (2.0 * std::f64::consts::PI)).sin() as f32)
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// 데시메이터 출력의 정착 구간 RMS 이득(dB, 사인 진폭 1 기준).
    fn decimated_gain_db(freq: f32) -> f32 {
        let mut d = FirDecimator::new(48_000, 16_000).unwrap();
        let x = sine(freq, 48_000, 48_000);
        let mut y = Vec::new();
        // 스트리밍 경계 검증을 겸해 홀수 크기 청크로 나눠 공급.
        for chunk in x.chunks(1234) {
            d.process(chunk, &mut y);
        }
        let tail = &y[y.len() / 2..];
        20.0 * (rms(tail) / std::f32::consts::FRAC_1_SQRT_2).log10()
    }

    #[test]
    fn passband_is_flat() {
        for freq in [200.0, 1000.0, 3000.0, 6000.0] {
            let g = decimated_gain_db(freq);
            assert!(g.abs() < 0.3, "{freq}Hz gain {g} dB");
        }
    }

    #[test]
    fn stopband_aliases_are_suppressed() {
        // 16k 나이퀴스트(8k) 초과 성분은 데시메이션 후 앨리어스로 접힌다.
        for freq in [9000.0, 12000.0, 17000.0] {
            let g = decimated_gain_db(freq);
            assert!(g < -60.0, "{freq}Hz alias gain {g} dB");
        }
    }

    /// 탭 DFT로 설계 스펙 직접 검증: 통과대역 평탄, 8kHz부터 -65dB 이하.
    #[test]
    fn designed_response_meets_spec() {
        let d = FirDecimator::new(48_000, 16_000).unwrap();
        let response_db = |f: f64| {
            let w = 2.0 * std::f64::consts::PI * f / 48_000.0;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (j, t) in d.taps().iter().enumerate() {
                re += *t as f64 * (w * j as f64).cos();
                im -= *t as f64 * (w * j as f64).sin();
            }
            20.0 * (re * re + im * im).sqrt().log10()
        };
        for f in [100.0, 1000.0, 4000.0, 7000.0] {
            let g = response_db(f);
            assert!(g.abs() < 0.1, "passband {f}Hz: {g:.2} dB");
        }
        let mut f = 8000.0;
        while f <= 24_000.0 {
            let g = response_db(f);
            assert!(g < -65.0, "stopband {f}Hz: {g:.2} dB");
            f += 250.0;
        }
    }

    #[test]
    fn output_length_is_one_third() {
        let mut d = FirDecimator::new(48_000, 16_000).unwrap();
        let x = sine(440.0, 48_000, 48_000 * 2);
        let mut y = Vec::new();
        d.process(&x, &mut y);
        let expected = 48_000 * 2 / 3;
        assert!(
            (y.len() as i64 - expected as i64).abs() < d.taps.len() as i64,
            "len {} vs {expected}",
            y.len()
        );
    }

    #[test]
    fn streaming_equals_batch() {
        let x = sine(1000.0, 48_000, 9600);
        let mut d1 = FirDecimator::new(48_000, 16_000).unwrap();
        let mut batch = Vec::new();
        d1.process(&x, &mut batch);
        let mut d2 = FirDecimator::new(48_000, 16_000).unwrap();
        let mut streamed = Vec::new();
        for chunk in x.chunks(371) {
            d2.process(chunk, &mut streamed);
        }
        assert_eq!(batch.len(), streamed.len());
        for (a, b) in batch.iter().zip(&streamed) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn rejects_non_integer_ratio() {
        assert!(FirDecimator::new(44_100, 16_000).is_err());
        assert!(FirDecimator::new(16_000, 16_000).is_err());
    }

    #[test]
    fn supports_96k() {
        let mut d = FirDecimator::new(96_000, 16_000).unwrap();
        assert_eq!(d.ratio(), 6);
        let x = sine(1000.0, 96_000, 96_000);
        let mut y = Vec::new();
        d.process(&x, &mut y);
        let tail = &y[y.len() / 2..];
        let g = 20.0 * (rms(tail) / std::f32::consts::FRAC_1_SQRT_2).log10();
        assert!(g.abs() < 0.3, "96k 1kHz gain {g} dB");
    }
}

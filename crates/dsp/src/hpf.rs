//! biquad HPF 70Hz (Butterworth Q) — 럼블/근접효과 저역 제거.

use biquad::{Biquad, Coefficients, DirectForm2Transposed, ToHertz, Type, Q_BUTTERWORTH_F32};

use crate::DspError;

pub const HPF_CUTOFF_HZ: f32 = 70.0;

pub struct HighPass {
    filt: DirectForm2Transposed<f32>,
}

impl HighPass {
    pub fn new(sample_rate: u32) -> Result<Self, DspError> {
        let coeffs = Coefficients::<f32>::from_params(
            Type::HighPass,
            (sample_rate as f32).hz(),
            HPF_CUTOFF_HZ.hz(),
            Q_BUTTERWORTH_F32,
        )
        .map_err(|e| DspError::FilterDesign(format!("{e:?}")))?;
        Ok(Self {
            filt: DirectForm2Transposed::<f32>::new(coeffs),
        })
    }

    /// 인플레이스 필터링.
    pub fn process(&mut self, samples: &mut [f32]) {
        for s in samples {
            *s = self.filt.run(*s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 정착 후 구간의 RMS 이득(dB)을 잰다.
    fn gain_db(freq: f32, sr: u32) -> f32 {
        let mut hpf = HighPass::new(sr).unwrap();
        let n = sr as usize; // 1초
        let w = 2.0 * std::f64::consts::PI * freq as f64 / sr as f64;
        let mut x: Vec<f32> = (0..n)
            .map(|i| ((w * i as f64) % (2.0 * std::f64::consts::PI)).sin() as f32)
            .collect();
        hpf.process(&mut x);
        let tail = &x[n / 2..];
        let rms = (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt();
        20.0 * (rms / std::f32::consts::FRAC_1_SQRT_2).log10()
    }

    #[test]
    fn passes_voice_band() {
        let g = gain_db(1000.0, 48_000);
        assert!(g.abs() < 0.1, "1kHz gain {g} dB");
        let g = gain_db(200.0, 48_000);
        assert!(g.abs() < 0.5, "200Hz gain {g} dB");
    }

    #[test]
    fn attenuates_rumble() {
        let g = gain_db(20.0, 48_000);
        assert!(g < -20.0, "20Hz gain {g} dB");
        let g = gain_db(50.0, 48_000);
        assert!(g < -4.0, "50Hz gain {g} dB");
    }
}

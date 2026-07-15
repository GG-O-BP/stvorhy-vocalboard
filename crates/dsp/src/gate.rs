//! hop 단위 RMS 측정과 게이트 (스펙 §4: 기본 -45 dBFS).
//!
//! SwiftF0 confidence는 voicing 판단이 아니므로 이 게이트를 항상 병행한다.

/// dBFS 하한 (무음 클램프; F0C1 rms 인코딩 하한과 일치).
pub const RMS_FLOOR_DBFS: f32 = -96.0;

/// 샘플 블록의 RMS를 dBFS로 반환한다 ([-96, 0]로 클램프).
pub fn rms_dbfs(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return RMS_FLOOR_DBFS;
    }
    let mean_sq = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    if mean_sq <= 0.0 {
        return RMS_FLOOR_DBFS;
    }
    (10.0 * mean_sq.log10()).clamp(RMS_FLOOR_DBFS, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(amp: f32, n: usize) -> Vec<f32> {
        let w = 2.0 * std::f64::consts::PI * 440.0 / 16_000.0;
        (0..n)
            .map(|i| amp * ((w * i as f64) % (2.0 * std::f64::consts::PI)).sin() as f32)
            .collect()
    }

    #[test]
    fn full_scale_sine_is_minus_3dbfs() {
        let x = sine(1.0, 16_000);
        let db = rms_dbfs(&x);
        assert!((db + 3.01).abs() < 0.05, "{db}");
    }

    #[test]
    fn amplitude_scales_linearly_in_db() {
        let loud = rms_dbfs(&sine(0.1, 16_000));
        let quiet = rms_dbfs(&sine(0.001, 16_000));
        assert!((loud - quiet - 40.0).abs() < 0.1, "{loud} vs {quiet}");
    }

    #[test]
    fn silence_clamps_to_floor() {
        assert_eq!(rms_dbfs(&vec![0.0; 256]), RMS_FLOOR_DBFS);
        assert_eq!(rms_dbfs(&[]), RMS_FLOOR_DBFS);
        // 극미세 신호도 -96 밑으로 내려가지 않는다 (JSON -inf 방지).
        assert_eq!(rms_dbfs(&vec![1e-9; 256]), RMS_FLOOR_DBFS);
    }

    #[test]
    fn gate_threshold_semantics() {
        // -45 dBFS 문턱: RMS -40 사인은 통과, -50 사인은 차단.
        let pass = rms_dbfs(&sine(0.01414, 4096)); // ≈ -40 dBFS RMS
        let block = rms_dbfs(&sine(0.00447, 4096)); // ≈ -50 dBFS RMS
        assert!(pass > -45.0, "{pass}");
        assert!(block < -45.0, "{block}");
    }
}

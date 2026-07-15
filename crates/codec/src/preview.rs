//! preview BLOB (스펙 §5): `[u8 버전=1][u8×256]`.
//!
//! 전체 길이를 256 버킷으로 등분, 버킷당 voiced 프레임 midi 중앙값을
//! `clamp(round((midi−20)×2), 1, 255)`로 양자화, 무성 버킷은 0.
//! sessions.preview / tracks.preview 공통 (목록 썸네일용).

use crate::F0Frame;

pub const PREVIEW_VERSION: u8 = 1;
pub const PREVIEW_BUCKETS: usize = 256;
pub const PREVIEW_LEN: usize = 1 + PREVIEW_BUCKETS;

/// midi 실수값을 preview 바이트로 양자화한다.
pub fn quantize_midi(midi: f32) -> u8 {
    (((midi - 20.0) * 2.0).round() as i64).clamp(1, 255) as u8
}

/// preview 바이트를 midi로 복원한다 (0 = 무성 버킷).
pub fn dequantize_midi(b: u8) -> Option<f32> {
    if b == 0 {
        None
    } else {
        Some(b as f32 / 2.0 + 20.0)
    }
}

/// 프레임 열에서 preview BLOB을 계산한다.
pub fn compute_preview(frames: &[F0Frame]) -> Vec<u8> {
    let mut out = vec![0u8; PREVIEW_LEN];
    out[0] = PREVIEW_VERSION;
    let n = frames.len();
    if n == 0 {
        return out;
    }
    for bucket in 0..PREVIEW_BUCKETS {
        let lo = bucket * n / PREVIEW_BUCKETS;
        let hi = (bucket + 1) * n / PREVIEW_BUCKETS;
        let mut voiced: Vec<f32> = frames[lo..hi]
            .iter()
            .filter(|f| f.voiced())
            .map(|f| f.midi())
            .collect();
        if voiced.is_empty() {
            continue;
        }
        voiced.sort_by(|a, b| a.total_cmp(b));
        let m = voiced.len();
        let median = if m % 2 == 1 {
            voiced[m / 2]
        } else {
            (voiced[m / 2 - 1] + voiced[m / 2]) / 2.0
        };
        out[1 + bucket] = quantize_midi(median);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voiced(midi: f32) -> F0Frame {
        F0Frame::quantize(midi, true, 1.0, -20.0)
    }

    fn unvoiced() -> F0Frame {
        F0Frame::quantize(0.0, false, 0.0, -80.0)
    }

    #[test]
    fn empty_input_is_all_zero() {
        let p = compute_preview(&[]);
        assert_eq!(p.len(), PREVIEW_LEN);
        assert_eq!(p[0], PREVIEW_VERSION);
        assert!(p[1..].iter().all(|&b| b == 0));
    }

    #[test]
    fn quantization_formula() {
        assert_eq!(quantize_midi(69.0), 98); // (69-20)*2
        assert_eq!(quantize_midi(20.0), 1); // 0 → clamp 1
        assert_eq!(quantize_midi(0.0), 1);
        assert_eq!(quantize_midi(200.0), 255);
        assert_eq!(dequantize_midi(98), Some(69.0));
        assert_eq!(dequantize_midi(0), None);
    }

    #[test]
    fn bucket_median_and_unvoiced_buckets() {
        // 512 프레임 → 버킷당 2프레임.
        let mut frames = Vec::new();
        for i in 0..256 {
            if i % 2 == 0 {
                frames.push(voiced(60.0));
                frames.push(voiced(62.0)); // 짝수 개수 중앙값 = 61.0
            } else {
                frames.push(unvoiced());
                frames.push(unvoiced());
            }
        }
        let p = compute_preview(&frames);
        for i in 0..256 {
            if i % 2 == 0 {
                assert_eq!(p[1 + i], quantize_midi(61.0), "bucket {i}");
            } else {
                assert_eq!(p[1 + i], 0, "bucket {i}");
            }
        }
    }

    #[test]
    fn short_input_leaves_empty_buckets_zero() {
        // 4 프레임: 앞 4개 버킷 경계에만 매핑, 나머지는 0이어야 한다.
        let frames = vec![voiced(50.0), voiced(52.0), unvoiced(), voiced(54.0)];
        let p = compute_preview(&frames);
        let nonzero = p[1..].iter().filter(|&&b| b != 0).count();
        assert_eq!(nonzero, 3);
    }

    #[test]
    fn median_ignores_unvoiced_within_bucket() {
        // 단일 버킷에 voiced 3 + unvoiced 다수.
        let mut frames = vec![voiced(40.0), voiced(80.0), voiced(41.0)];
        frames.extend(std::iter::repeat_with(unvoiced).take(253));
        // 256 프레임 → 버킷당 1개. 첫 3개 버킷만 voiced.
        let p = compute_preview(&frames);
        assert_eq!(p[1], quantize_midi(40.0));
        assert_eq!(p[2], quantize_midi(80.0));
        assert_eq!(p[3], quantize_midi(41.0));
    }
}

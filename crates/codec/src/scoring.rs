//! 채점 (스펙 §5).
//!
//! 사용자·레퍼런스 프레임을 동일 16ms 그리드로 정렬(인덱스 =
//! floor(플레이헤드 ms / 16)). Δcents = 1200·log2(f0_u/f0_r) — F0C1
//! 도메인에서는 (midi_u − midi_r)×100 과 동치. 옥타브 불변 세션은
//! Δ ← ((Δ+600) mod 1200) − 600 으로 접는다.
//!
//! 프레임 점수 = 100 × clamp(1 − max(0, |Δ| − 20) / 80, 0, 1)
//! — ±20c까지 만점, 반음(100c) 이상 0점.
//!
//! 집계 대상 = 레퍼런스 voiced 프레임 전체. 사용자 unvoiced면 0점
//! (안 부른 구간 감점), 사용자만 voiced인 프레임(애드리브)은 무패널티
//! 제외. mean_score = 대상 프레임 평균, mean_abs_cents = 양쪽 voiced
//! 교집합의 |Δ| 평균 (옥타브 불변 세션은 접은 값 기준).

use crate::F0Frame;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoreSummary {
    /// 레퍼런스 voiced 프레임 평균 점수 [0,100]. 대상 없으면 None.
    pub mean_score: Option<f32>,
    /// 양쪽 voiced 교집합 |Δcents| 평균. 교집합 없으면 None.
    pub mean_abs_cents: Option<f32>,
    /// 집계 대상(레퍼런스 voiced) 프레임 수.
    pub scored_frames: u32,
    /// 양쪽 voiced 교집합 프레임 수.
    pub overlap_frames: u32,
}

/// Δcents를 옥타브 불변으로 접는다: ((Δ+600) mod 1200) − 600.
pub fn fold_octave(delta_cents: f32) -> f32 {
    let m = (delta_cents + 600.0).rem_euclid(1200.0);
    m - 600.0
}

/// 프레임 점수 (§5 공식).
pub fn frame_score(abs_delta_cents: f32) -> f32 {
    100.0 * (1.0 - ((abs_delta_cents - 20.0).max(0.0) / 80.0)).clamp(0.0, 1.0)
}

/// 세션 채점. `user_offset_frames`: 사용자 프레임 인덱스 보정
/// (지연 캘리브레이션 ms / 16, 반올림). 레퍼런스 인덱스 i에 대해
/// 사용자 인덱스 i + offset 을 본다.
pub fn score_session(
    user: &[F0Frame],
    reference: &[F0Frame],
    octave_invariant: bool,
    user_offset_frames: i32,
) -> ScoreSummary {
    let mut score_sum = 0.0f64;
    let mut scored = 0u32;
    let mut abs_sum = 0.0f64;
    let mut overlap = 0u32;

    for (i, r) in reference.iter().enumerate() {
        if !r.voiced() {
            continue;
        }
        let ui = i as i64 + user_offset_frames as i64;
        // 세션이 레퍼런스보다 짧으면 그 구간은 "안 부른 구간"으로 0점.
        let u = if ui >= 0 { user.get(ui as usize) } else { None };
        scored += 1;
        match u {
            Some(u) if u.voiced() => {
                let mut delta = (u.midi() - r.midi()) * 100.0;
                if octave_invariant {
                    delta = fold_octave(delta);
                }
                score_sum += frame_score(delta.abs()) as f64;
                abs_sum += delta.abs() as f64;
                overlap += 1;
            }
            _ => {
                // 사용자 unvoiced → 0점 가산 (합계에 0을 더함).
            }
        }
    }

    ScoreSummary {
        mean_score: (scored > 0).then(|| (score_sum / scored as f64) as f32),
        mean_abs_cents: (overlap > 0).then(|| (abs_sum / overlap as f64) as f32),
        scored_frames: scored,
        overlap_frames: overlap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voiced(midi: f32) -> F0Frame {
        F0Frame::quantize(midi, true, 0.95, -20.0)
    }

    fn unvoiced() -> F0Frame {
        F0Frame::quantize(0.0, false, 0.0, -80.0)
    }

    #[test]
    fn perfect_match_scores_100() {
        let r: Vec<_> = (0..100).map(|_| voiced(60.0)).collect();
        let s = score_session(&r, &r, false, 0);
        assert_eq!(s.mean_score, Some(100.0));
        assert_eq!(s.mean_abs_cents, Some(0.0));
        assert_eq!(s.scored_frames, 100);
        assert_eq!(s.overlap_frames, 100);
    }

    #[test]
    fn frame_score_formula_edges() {
        assert_eq!(frame_score(0.0), 100.0);
        assert_eq!(frame_score(20.0), 100.0); // ±20c까지 만점
        assert!((frame_score(25.0) - 93.75).abs() < 1e-4);
        assert_eq!(frame_score(60.0), 50.0);
        assert_eq!(frame_score(100.0), 0.0); // 반음부터 0점
        assert_eq!(frame_score(500.0), 0.0);
    }

    #[test]
    fn quarter_tone_offset() {
        // +25c 편차: F0C1 양자화(1c 단위)로 정확히 표현됨.
        let r: Vec<_> = (0..50).map(|_| voiced(60.0)).collect();
        let u: Vec<_> = (0..50).map(|_| voiced(60.25)).collect();
        let s = score_session(&u, &r, false, 0);
        assert!((s.mean_score.unwrap() - 93.75).abs() < 0.01);
        assert!((s.mean_abs_cents.unwrap() - 25.0).abs() < 0.01);
    }

    #[test]
    fn octave_invariant_folds() {
        let r: Vec<_> = (0..50).map(|_| voiced(48.0)).collect();
        let u: Vec<_> = (0..50).map(|_| voiced(60.0)).collect(); // +1200c
        let strict = score_session(&u, &r, false, 0);
        assert_eq!(strict.mean_score, Some(0.0));
        assert_eq!(strict.mean_abs_cents, Some(1200.0));
        let folded = score_session(&u, &r, true, 0);
        assert_eq!(folded.mean_score, Some(100.0));
        assert_eq!(folded.mean_abs_cents, Some(0.0));
        // +1250c → 접으면 +50c.
        let u2: Vec<_> = (0..50).map(|_| voiced(60.5)).collect();
        let s2 = score_session(&u2, &r, true, 0);
        assert!((s2.mean_abs_cents.unwrap() - 50.0).abs() < 0.01);
    }

    #[test]
    fn fold_octave_range() {
        assert_eq!(fold_octave(0.0), 0.0);
        assert_eq!(fold_octave(1200.0), 0.0);
        assert_eq!(fold_octave(-1200.0), 0.0);
        assert_eq!(fold_octave(600.0), -600.0); // 반옥타브 경계는 [-600,600)
        assert_eq!(fold_octave(650.0), -550.0);
        assert_eq!(fold_octave(-650.0), 550.0);
    }

    #[test]
    fn user_unvoiced_penalized_adlib_excluded() {
        // 레퍼런스: 앞 50 voiced, 뒤 50 unvoiced.
        let mut r: Vec<_> = (0..50).map(|_| voiced(60.0)).collect();
        r.extend((0..50).map(|_| unvoiced()));
        // 사용자: 앞 25만 부름(정확), 나머지 무성, 뒤 50은 애드리브로 voiced.
        let mut u: Vec<_> = (0..25).map(|_| voiced(60.0)).collect();
        u.extend((0..25).map(|_| unvoiced()));
        u.extend((0..50).map(|_| voiced(72.0)));
        let s = score_session(&u, &r, false, 0);
        // 대상 50 (레퍼런스 voiced), 그중 25 만점 + 25 0점 → 50.
        assert_eq!(s.scored_frames, 50);
        assert_eq!(s.overlap_frames, 25);
        assert!((s.mean_score.unwrap() - 50.0).abs() < 1e-4);
        assert_eq!(s.mean_abs_cents, Some(0.0));
    }

    #[test]
    fn offset_alignment() {
        // 사용자가 2프레임 늦게 (지연) 같은 멜로디를 불렀다: offset +2로 정렬.
        let r: Vec<_> = (0..10).map(|i| voiced(60.0 + i as f32)).collect();
        let mut u = vec![unvoiced(), unvoiced()];
        u.extend((0..10).map(|i| voiced(60.0 + i as f32)));
        let misaligned = score_session(&u, &r, false, 0);
        let aligned = score_session(&u, &r, false, 2);
        assert_eq!(aligned.mean_score, Some(100.0));
        assert!(misaligned.mean_score.unwrap() < aligned.mean_score.unwrap());
    }

    #[test]
    fn no_reference_voiced_means_none() {
        let r: Vec<_> = (0..10).map(|_| unvoiced()).collect();
        let u: Vec<_> = (0..10).map(|_| voiced(60.0)).collect();
        let s = score_session(&u, &r, false, 0);
        assert_eq!(s.mean_score, None);
        assert_eq!(s.mean_abs_cents, None);
        assert_eq!(s.scored_frames, 0);
    }
}

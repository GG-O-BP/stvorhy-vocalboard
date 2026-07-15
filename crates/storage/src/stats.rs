//! 세션 요약 통계 (§5 sessions 컬럼).

use vocalboard_codec::{F0Frame, HOP_MS};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionStats {
    pub frame_count: u32,
    pub duration_ms: u32,
    pub voiced_ratio: f32,
    pub midi_min: Option<f32>,
    pub midi_max: Option<f32>,
}

impl SessionStats {
    pub fn from_frames(frames: &[F0Frame]) -> Self {
        let frame_count = frames.len() as u32;
        let mut voiced = 0u32;
        let mut midi_min = f32::INFINITY;
        let mut midi_max = f32::NEG_INFINITY;
        for f in frames {
            if f.voiced() {
                voiced += 1;
                let m = f.midi();
                midi_min = midi_min.min(m);
                midi_max = midi_max.max(m);
            }
        }
        Self {
            frame_count,
            duration_ms: frame_count * HOP_MS as u32,
            voiced_ratio: if frame_count == 0 { 0.0 } else { voiced as f32 / frame_count as f32 },
            midi_min: (voiced > 0).then_some(midi_min),
            midi_max: (voiced > 0).then_some(midi_max),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_over_mixed_frames() {
        let mut frames = Vec::new();
        for _ in 0..75 {
            frames.push(F0Frame::quantize(60.0, true, 0.95, -20.0));
        }
        for _ in 0..25 {
            frames.push(F0Frame::quantize(0.0, false, 0.0, -80.0));
        }
        frames.push(F0Frame::quantize(72.5, true, 0.99, -18.0));
        let s = SessionStats::from_frames(&frames);
        assert_eq!(s.frame_count, 101);
        assert_eq!(s.duration_ms, 101 * 16);
        assert!((s.voiced_ratio - 76.0 / 101.0).abs() < 1e-6);
        assert_eq!(s.midi_min, Some(60.0));
        assert_eq!(s.midi_max, Some(72.5));
    }

    #[test]
    fn stats_empty_and_all_unvoiced() {
        let s = SessionStats::from_frames(&[]);
        assert_eq!(s.frame_count, 0);
        assert_eq!(s.voiced_ratio, 0.0);
        assert_eq!(s.midi_min, None);

        let frames = vec![F0Frame::quantize(0.0, false, 0.0, -80.0); 10];
        let s = SessionStats::from_frames(&frames);
        assert_eq!(s.voiced_ratio, 0.0);
        assert_eq!(s.midi_max, None);
    }
}

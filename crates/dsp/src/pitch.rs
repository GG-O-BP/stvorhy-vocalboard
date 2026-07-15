//! f0 → midi → cents 변환 (12평균율, A4=440Hz 고정 — 스펙 §5).

/// f0(Hz) → MIDI 실수값.
pub fn f0_to_midi(f0: f32) -> f32 {
    69.0 + 12.0 * (f0 / 440.0).log2()
}

/// MIDI 실수값 → 최근접 반음 대비 cents 편차 [-50, +50).
pub fn midi_to_cents(midi: f32) -> f32 {
    (midi - midi.round()) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a440_is_midi_69() {
        assert!((f0_to_midi(440.0) - 69.0).abs() < 1e-5);
        assert_eq!(midi_to_cents(f0_to_midi(440.0)), 0.0);
    }

    #[test]
    fn octaves_are_12_semitones() {
        assert!((f0_to_midi(220.0) - 57.0).abs() < 1e-5);
        assert!((f0_to_midi(880.0) - 81.0).abs() < 1e-5);
    }

    #[test]
    fn cents_range_is_half_open() {
        // 정확히 반음 중간(+50c)은 위 반음의 -50c로 정규화된다.
        assert_eq!(midi_to_cents(69.5), -50.0);
        assert_eq!(midi_to_cents(68.5), -50.0);
        assert!((midi_to_cents(69.499) - 49.9).abs() < 0.11);
        assert!((midi_to_cents(69.25) - 25.0).abs() < 1e-4);
        assert!((midi_to_cents(68.75) + 25.0).abs() < 1e-4);
    }
}

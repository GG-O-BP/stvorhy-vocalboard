/**
 * round(midi) → 음이름 테이블 조회 (스펙 §5: 조회는 렌더로 간주).
 * @file
 */

const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/**
 * MIDI 실수값을 최근접 반음의 음이름으로 변환한다.
 * @param {number} midi MIDI 노트 번호 (실수)
 * @returns {string} 예: 69 → "A4", 60.4 → "C4"
 */
export function midiToNoteName(midi) {
  const n = Math.round(midi);
  const octave = Math.floor(n / 12) - 1;
  return NAMES[((n % 12) + 12) % 12] + String(octave);
}

/**
 * cents 편차 표시 문자열 (부호 포함 정수).
 * @param {number} cents [-50,+50)
 * @returns {string} 예: +12, -3, ±0
 */
export function formatCents(cents) {
  const c = Math.round(cents);
  if (c === 0) return "±0";
  return c > 0 ? `+${c}` : String(c);
}

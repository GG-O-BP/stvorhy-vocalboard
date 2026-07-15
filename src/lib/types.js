/**
 * IPC 계약 단일 정의. Rust 구조체(serde) 변경 시 반드시 함께 갱신할 것.
 * @file
 */

/**
 * DSP 스레드가 62.5Hz로 방출하는 피치 프레임 (Rust `PitchFrame`, src-tauri).
 * 게이트 미달 hop도 결번 없이 방출된다 (f0/midi/cents=0, confidence=0,
 * voiced=false, rms는 실측값).
 *
 * @typedef {Object} PitchFrame
 * @property {number} t 세션 시작 기준 ms (u32, 인덱스×16ms)
 * @property {number} f0 기본 주파수 Hz (unvoiced면 0)
 * @property {number} midi MIDI 노트 실수값 (A4=440 기준, unvoiced면 0)
 * @property {number} cents 최근접 반음 대비 편차 [-50,+50) (unvoiced면 0)
 * @property {number} confidence SwiftF0 신뢰도 [0,1] (voicing 판단 아님)
 * @property {number} rms 입력 RMS dBFS (음수, 예: -45.2)
 * @property {boolean} voiced RMS 게이트 AND confidence 임계 통과 여부
 */

export {};

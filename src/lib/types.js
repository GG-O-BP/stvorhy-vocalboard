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

/**
 * 세션 목록 항목 (Rust `SessionListItem`, crates/storage queries).
 * @typedef {Object} SessionListItem
 * @property {string} id
 * @property {number} started_at unix epoch ms
 * @property {number} duration_ms
 * @property {string | null} track_id
 * @property {string | null} track_title
 * @property {number} frame_count
 * @property {number} voiced_ratio [0,1]
 * @property {number | null} mean_score
 * @property {boolean} has_recording
 * @property {number[] | null} preview [버전u8, ...256 buckets] (§5)
 */

/**
 * 세션 상세 (Rust `SessionDetail`).
 * @typedef {Object} SessionDetail
 * @property {string} id
 * @property {number} started_at
 * @property {number} duration_ms
 * @property {string | null} track_id
 * @property {string | null} track_title
 * @property {number} frame_count
 * @property {number} voiced_ratio
 * @property {number | null} midi_min
 * @property {number | null} midi_max
 * @property {number | null} mean_abs_cents
 * @property {number | null} mean_score
 * @property {boolean} octave_invariant
 * @property {string | null} codec
 * @property {string | null} recording_path
 */

/**
 * min/max 데시메이션 포인트 (Rust `SeriesPoint`). 무성 버킷은 null.
 * @typedef {Object} SeriesPoint
 * @property {number} t 버킷 시작 ms
 * @property {number | null} min voiced midi 최소
 * @property {number | null} max voiced midi 최대
 */

/**
 * 세션 피치 시리즈 (Rust `SessionSeries`).
 * @typedef {Object} SessionSeries
 * @property {number} frame_count
 * @property {number} hop_ms
 * @property {number} duration_ms
 * @property {SeriesPoint[]} points
 */

/**
 * 재생 플레이헤드 이벤트 (Rust `PlayheadEvent`, ~20Hz).
 * @typedef {Object} PlayheadEvent
 * @property {number} t 재생 위치 ms
 * @property {boolean} playing
 * @property {boolean} done
 * @property {number} duration_ms
 */

/**
 * 세션 종료 요약 (Rust `FinalizedSession`).
 * @typedef {Object} FinalizedSession
 * @property {string} id
 * @property {number} started_at
 * @property {number} duration_ms
 * @property {number} frame_count
 * @property {number} voiced_ratio
 * @property {number | null} midi_min
 * @property {number | null} midi_max
 * @property {number | null} mean_abs_cents
 * @property {number | null} mean_score
 * @property {string | null} recording_path
 */

export {};

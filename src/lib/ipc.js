/**
 * 백엔드 IPC 래퍼. 커맨드 이름/페이로드의 단일 사용처.
 * @file
 */
import { Channel, invoke } from "@tauri-apps/api/core";

/** @typedef {import("./types.js").PitchFrame} PitchFrame */

/**
 * 캡처 시작 정보.
 * @typedef {Object} CaptureInfo
 * @property {number} sample_rate
 * @property {number} channels
 * @property {boolean} simulated
 */

/**
 * 캡처를 시작한다. 프레임은 62.5Hz로 `onFrame`에 직접 전달된다
 * (Solid 반응성 금지 경로 — 콜백에서 PixiJS 직행).
 * @param {(frame: PitchFrame) => void} onFrame
 * @returns {Promise<CaptureInfo>}
 */
export async function startCapture(onFrame) {
  /** @type {Channel<PitchFrame>} */
  const channel = new Channel();
  channel.onmessage = onFrame;
  return await invoke("start_capture", { channel });
}

/** @returns {Promise<void>} */
export function stopCapture() {
  return invoke("stop_capture");
}

/**
 * DSP 관련 설정을 백엔드에 동기화한다 (실행 중 캡처에 즉시 적용).
 * @param {import("./settings.js").AppConfig} config
 * @returns {Promise<void>}
 */
export function configure(config) {
  return invoke("configure", { config });
}

/** @typedef {import("./types.js").SessionListItem} SessionListItem */
/** @typedef {import("./types.js").SessionDetail} SessionDetail */
/** @typedef {import("./types.js").SessionSeries} SessionSeries */
/** @typedef {import("./types.js").PlayheadEvent} PlayheadEvent */
/** @typedef {import("./types.js").FinalizedSession} FinalizedSession */

/** @returns {Promise<SessionListItem[]>} */
export function listSessions() {
  return invoke("list_sessions");
}

/**
 * @param {string} id
 * @returns {Promise<SessionDetail>}
 */
export function sessionDetail(id) {
  return invoke("session_detail", { id });
}

/**
 * @param {string} id
 * @param {number} maxPoints
 * @returns {Promise<SessionSeries>}
 */
export function sessionSeries(id, maxPoints) {
  return invoke("session_series", { id, maxPoints });
}

/**
 * 세션 녹음 재생. 플레이헤드는 ~20Hz로 콜백에 온다. 반환: duration_ms.
 * @param {string} sessionId
 * @param {number} startMs
 * @param {(e: PlayheadEvent) => void} onPlayhead
 * @returns {Promise<number>}
 */
export async function playRecording(sessionId, startMs, onPlayhead) {
  /** @type {Channel<PlayheadEvent>} */
  const channel = new Channel();
  channel.onmessage = onPlayhead;
  return await invoke("play_recording", { sessionId, startMs, channel });
}

/** @returns {Promise<void>} */
export function playbackPause() {
  return invoke("playback_pause");
}

/** @returns {Promise<void>} */
export function playbackResume() {
  return invoke("playback_resume");
}

/**
 * @param {number} tMs
 * @returns {Promise<void>}
 */
export function playbackSeek(tMs) {
  return invoke("playback_seek", { tMs });
}

/** @returns {Promise<void>} */
export function playbackStop() {
  return invoke("playback_stop");
}

/** @typedef {import("./types.js").TrackListItem} TrackListItem */
/** @typedef {import("./types.js").SepProgress} SepProgress */
/** @typedef {import("./types.js").ImportedTrack} ImportedTrack */
/** @typedef {import("./types.js").PracticeInfo} PracticeInfo */

/**
 * @param {string} srcPath
 * @returns {Promise<ImportedTrack>}
 */
export function importTrack(srcPath) {
  return invoke("import_track", { srcPath });
}

/**
 * 분리 잡 시작 (백그라운드). 진행은 콜백으로 (download/decode/separate/
 * save/extract/done/error).
 * @param {string} trackId
 * @param {(p: SepProgress) => void} onProgress
 * @returns {Promise<void>}
 */
export async function separateTrack(trackId, onProgress) {
  /** @type {Channel<SepProgress>} */
  const channel = new Channel();
  channel.onmessage = onProgress;
  return await invoke("separate_track", { trackId, channel });
}

/** @returns {Promise<TrackListItem[]>} */
export function listTracks() {
  return invoke("list_tracks");
}

/**
 * @param {string} id
 * @param {number} maxPoints
 * @returns {Promise<import("./types.js").SessionSeries>}
 */
export function trackSeries(id, maxPoints) {
  return invoke("track_series", { id, maxPoints });
}

/**
 * @param {string} id
 * @returns {Promise<void>}
 */
export function deleteTrack(id) {
  return invoke("delete_track", { id });
}

/**
 * 연습 시작 (반주 재생 여부 선택). HEADPHONE_GATE: 접두 오류 시 사용자
 * 확인 후 headphoneOverride=true로 재시도.
 * @param {string} trackId
 * @param {boolean} playback
 * @param {boolean} headphoneOverride
 * @param {(f: import("./types.js").PitchFrame) => void} onFrame
 * @param {(e: import("./types.js").PlayheadEvent) => void} onPlayhead
 * @returns {Promise<PracticeInfo>}
 */
export async function practiceStart(trackId, playback, headphoneOverride, onFrame, onPlayhead) {
  /** @type {Channel<import("./types.js").PitchFrame>} */
  const pitch = new Channel();
  pitch.onmessage = onFrame;
  /** @type {Channel<import("./types.js").PlayheadEvent>} */
  const playhead = new Channel();
  playhead.onmessage = onPlayhead;
  return await invoke("practice_start", {
    trackId,
    playback,
    headphoneOverride,
    pitch,
    playhead,
  });
}

/** @returns {Promise<import("./types.js").FinalizedSession | null>} */
export function practiceStop() {
  return invoke("practice_stop");
}

/**
 * 노래방 (반주만 재생). 반환: duration_ms.
 * @param {string} trackId
 * @param {(e: import("./types.js").PlayheadEvent) => void} onPlayhead
 * @returns {Promise<number>}
 */
export async function karaokeStart(trackId, onPlayhead) {
  /** @type {Channel<import("./types.js").PlayheadEvent>} */
  const playhead = new Channel();
  playhead.onmessage = onPlayhead;
  return await invoke("karaoke_start", { trackId, playhead });
}

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

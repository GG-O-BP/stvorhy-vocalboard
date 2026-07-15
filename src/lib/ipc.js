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

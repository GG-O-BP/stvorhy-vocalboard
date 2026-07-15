/**
 * 설정 (스펙 §3): tauri-plugin-store 영속화 + configure 커맨드 백엔드 동기화.
 * 키 계약은 src-tauri/src/config.rs 의 AppConfig 와 동기 유지.
 * @file
 */
import { load } from "@tauri-apps/plugin-store";
import { configure } from "./ipc.js";

/**
 * @typedef {Object} AppConfig
 * @property {number} gate_dbfs RMS 게이트 임계 dBFS (기본 -45)
 * @property {number} conf_threshold confidence 임계 (기본 0.9)
 * @property {boolean} recording_enabled 마이크 드라이 녹음 (Phase 2)
 * @property {number} recording_keep_last 녹음 보존 개수 (Phase 2)
 * @property {boolean} octave_invariant 채점 옥타브 불변 (Phase 3.5)
 * @property {number} latency_calib_ms 재생 지연 캘리브레이션 ms (Phase 3.5)
 * @property {boolean} separation_quality 분리 품질 모드 (Phase 3.5)
 */

/** @type {AppConfig} */
export const DEFAULTS = Object.freeze({
  gate_dbfs: -45,
  conf_threshold: 0.9,
  recording_enabled: true,
  recording_keep_last: 20,
  octave_invariant: false,
  latency_calib_ms: 0,
  separation_quality: false,
});

/**
 * 저장된 부분 설정을 기본값과 병합한다 (미지의 키는 버린다).
 * @param {unknown} saved
 * @returns {AppConfig}
 */
export function mergeWithDefaults(saved) {
  /** @type {Record<string, unknown>} */
  const out = { ...DEFAULTS };
  if (saved && typeof saved === "object") {
    for (const key of Object.keys(DEFAULTS)) {
      const v = /** @type {Record<string, unknown>} */ (saved)[key];
      if (v !== undefined && typeof v === typeof out[key]) {
        out[key] = v;
      }
    }
  }
  return /** @type {AppConfig} */ (out);
}

const STORE_FILE = "settings.json";
const KEY = "config";

/** @returns {Promise<AppConfig>} */
export async function loadSettings() {
  const store = await load(STORE_FILE, { autoSave: true, defaults: {} });
  return mergeWithDefaults(await store.get(KEY));
}

/**
 * 저장 + 백엔드 동기화.
 * @param {AppConfig} cfg
 */
export async function saveSettings(cfg) {
  const store = await load(STORE_FILE, { autoSave: true, defaults: {} });
  await store.set(KEY, cfg);
  await configure(cfg);
}

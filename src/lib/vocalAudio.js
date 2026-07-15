/**
 * vocal-audio 플러그인 JS 래퍼 (권한/세션/화면유지/라우트/포그라운드).
 * 데스크톱에서는 스텁 구현이 응답한다 (라우트 headphones=null).
 * @file
 */
import { addPluginListener, invoke } from "@tauri-apps/api/core";

const PLUGIN = "vocal-audio";

/**
 * @typedef {Object} PermissionStatus
 * @property {"granted" | "denied" | "prompt"} microphone
 */

/**
 * @typedef {Object} RouteInfo
 * @property {boolean | null} headphones 헤드폰 경유 여부 (데스크톱: null)
 * @property {string} description
 */

/** @returns {Promise<PermissionStatus>} */
export function checkPermissions() {
  return invoke(`plugin:${PLUGIN}|check_permissions`);
}

/** @returns {Promise<PermissionStatus>} */
export function requestPermissions() {
  return invoke(`plugin:${PLUGIN}|request_permissions`);
}

/**
 * 오디오 세션 구성 (iOS: .playAndRecord+.measurement 활성화, Android: 포커스).
 * @param {{ playback: boolean }} config
 * @returns {Promise<void>}
 */
export function configureSession(config) {
  return invoke(`plugin:${PLUGIN}|configure_session`, { config });
}

/** @returns {Promise<void>} */
export function releaseSession() {
  return invoke(`plugin:${PLUGIN}|release_session`);
}

/**
 * @param {boolean} enabled
 * @returns {Promise<void>}
 */
export function setKeepScreenOn(enabled) {
  return invoke(`plugin:${PLUGIN}|set_keep_screen_on`, { enabled });
}

/** @returns {Promise<RouteInfo>} */
export function currentRoute() {
  return invoke(`plugin:${PLUGIN}|current_route`);
}

/** @returns {Promise<void>} */
export function startForegroundService() {
  return invoke(`plugin:${PLUGIN}|start_foreground_service`);
}

/** @returns {Promise<void>} */
export function stopForegroundService() {
  return invoke(`plugin:${PLUGIN}|stop_foreground_service`);
}

/**
 * @param {string[]} paths
 * @returns {Promise<void>}
 */
export function excludeFromBackup(paths) {
  return invoke(`plugin:${PLUGIN}|exclude_from_backup`, { paths });
}

/**
 * 인터럽션 이벤트 (iOS 전화 수신 등).
 * @param {(e: { phase: "began" | "ended", shouldResume: boolean }) => void} handler
 */
export function onInterruption(handler) {
  return addPluginListener(PLUGIN, "interruption", handler);
}

/**
 * 라우트 변경 이벤트 (헤드폰 연결/해제 등).
 * @param {(e: { headphones: boolean, description: string }) => void} handler
 */
export function onRouteChanged(handler) {
  return addPluginListener(PLUGIN, "routeChanged", handler);
}

/**
 * Android 오디오 포커스 변경 이벤트.
 * @param {(e: { state: "gain" | "loss" | "lossTransient" | "lossTransientCanDuck" }) => void} handler
 */
export function onFocusChanged(handler) {
  return addPluginListener(PLUGIN, "focusChanged", handler);
}

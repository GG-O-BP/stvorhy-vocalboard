/**
 * 판독값 스로틀 (스펙 §4: DOM 판독값 10–15Hz).
 * 62.5Hz 스트림을 Solid 신호에 직접 태우지 않기 위한 유틸.
 * @file
 */

/**
 * 최신 값을 보관했다가 고정 주기로만 `emit`을 호출하는 스로틀러.
 * 값이 없으면 emit하지 않는다. `stop()`으로 타이머를 정리한다.
 *
 * @template T
 * @param {(value: T) => void} emit
 * @param {number} [intervalMs=80] 기본 80ms ≈ 12.5Hz
 * @param {{ setInterval?: typeof setInterval, clearInterval?: typeof clearInterval }} [timers]
 *   테스트 주입용 타이머
 * @returns {{ push: (value: T) => void, stop: () => void }}
 */
export function createReadoutThrottle(emit, intervalMs = 80, timers = {}) {
  const si = timers.setInterval ?? setInterval;
  const ci = timers.clearInterval ?? clearInterval;
  /** @type {T | undefined} */
  let latest;
  let hasNew = false;
  const id = si(() => {
    if (hasNew && latest !== undefined) {
      hasNew = false;
      emit(latest);
    }
  }, intervalMs);
  return {
    push(value) {
      latest = value;
      hasNew = true;
    },
    stop() {
      ci(id);
    },
  };
}

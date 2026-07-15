/**
 * 모바일 경화 (Phase 4): 인터럽션/라우트 변경/오디오 포커스 이벤트를
 * 캡처·연습 수명주기에 배선한다. 스트림 재구성은 "정지 → 재시작"으로
 * 구현된다 (백엔드 start_capture가 교체 시맨틱이므로 안전).
 *
 * 데스크톱에서는 이벤트가 오지 않으므로 no-op이다.
 * @file
 */
import { onFocusChanged, onInterruption, onRouteChanged } from "./vocalAudio.js";

/**
 * @param {Object} hooks
 * @param {() => boolean} hooks.isActive 현재 캡처/연습 중인가
 * @param {() => Promise<void>} hooks.restart 스트림 재구성 (라우트 변경)
 * @param {(reason: string) => Promise<void>} hooks.suspend 중단 (인터럽션/포커스 손실)
 * @param {(reason: string) => void} [hooks.notify] 사용자 안내 문구
 * @param {boolean} [hooks.playbackActive] 반주 동시 모드 여부 (스피커 전환 시 차단)
 * @returns {Promise<() => void>} detach
 */
export async function attachMobileHandlers(hooks) {
  const notify = hooks.notify ?? (() => {});
  /** @type {Array<{ unregister: () => void } | undefined>} */
  const registrations = [];

  try {
    registrations.push(
      await onInterruption(async (e) => {
        if (!hooks.isActive()) return;
        if (e.phase === "began") {
          notify("오디오 인터럽션 — 세션을 일시 중단합니다");
          await hooks.suspend("interruption");
        } else if (e.phase === "ended" && e.shouldResume) {
          notify("인터럽션 종료 — 재시작합니다");
          await hooks.restart();
        }
      }),
    );
    registrations.push(
      await onRouteChanged(async (e) => {
        if (!hooks.isActive()) return;
        if (hooks.playbackActive && e.headphones === false) {
          // 가드레일: 동시 모드 중 스피커로 빠지면 즉시 차단.
          notify(`스피커로 전환됨(${e.description}) — 동시 모드를 중단합니다`);
          await hooks.suspend("route-speaker");
          return;
        }
        // 장치가 바뀌면 SR/버퍼가 달라질 수 있으므로 스트림 재구성.
        notify(`라우트 변경(${e.description}) — 스트림 재구성`);
        await hooks.restart();
      }),
    );
    registrations.push(
      await onFocusChanged(async (e) => {
        if (!hooks.isActive()) return;
        if (e.state === "loss") {
          notify("오디오 포커스 상실 — 세션을 중단합니다");
          await hooks.suspend("focus-loss");
        } else if (e.state === "lossTransient") {
          notify("일시적 포커스 상실 — 세션을 일시 중단합니다");
          await hooks.suspend("focus-transient");
        }
        // lossTransientCanDuck: 캡처는 유지 (덕킹은 재생에만 의미).
      }),
    );
  } catch {
    // 플러그인 이벤트 미지원 환경(데스크톱 webview 일부): 무시.
  }

  return () => {
    for (const r of registrations) {
      try {
        r?.unregister();
      } catch {
        // already gone
      }
    }
  };
}

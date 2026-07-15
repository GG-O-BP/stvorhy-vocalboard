import { createSignal, onCleanup, Show } from "solid-js";
import { startCapture, stopCapture } from "../lib/ipc.js";
import { formatCents, midiToNoteName } from "../lib/noteNames.js";
import { createReadoutThrottle } from "../lib/throttle.js";
import { createPianoRoll } from "../render/pianoRoll.js";

/** @typedef {import("../lib/types.js").PitchFrame} PitchFrame */

/**
 * 라이브 피치 모니터 뷰. 컴포넌트는 얇게: 렌더 로직은 render/pianoRoll.js,
 * IPC는 lib/ipc.js. 62.5Hz 프레임은 Channel 콜백에서 PixiJS로 직행하고,
 * Solid 신호에는 12.5Hz 스로틀된 판독값만 태운다 (가드레일).
 */
export default function LiveView() {
  const [running, setRunning] = createSignal(false);
  const [status, setStatus] = createSignal("대기");
  /** @type {ReturnType<typeof createSignal<PitchFrame | null>>} */
  const [readout, setReadout] = createSignal(/** @type {PitchFrame | null} */ (null));

  /** @type {HTMLDivElement | undefined} */
  let host;
  /** @type {Awaited<ReturnType<typeof createPianoRoll>> | null} */
  let roll = null;
  /** @type {ReturnType<typeof createReadoutThrottle<PitchFrame>> | null} */
  let throttle = null;
  let busy = false;

  async function start() {
    if (running() || busy) return;
    busy = true;
    try {
      if (!roll && host) {
        roll = await createPianoRoll(host);
      }
      throttle = createReadoutThrottle((f) => setReadout(f), 80);
      const info = await startCapture((frame) => {
        roll?.pushFrame(frame);
        throttle?.push(frame);
      });
      setStatus(
        `${info.sample_rate}Hz · ${info.channels}ch · ${roll?.rendererType()}` +
          (info.simulated ? " · 시뮬레이터" : ""),
      );
      setRunning(true);
    } catch (e) {
      setStatus(String(e));
    } finally {
      busy = false;
    }
  }

  async function stop() {
    if (!running() || busy) return;
    busy = true;
    try {
      await stopCapture();
    } catch (e) {
      setStatus(String(e));
    } finally {
      throttle?.stop();
      throttle = null;
      setRunning(false);
      setStatus("정지");
      busy = false;
    }
  }

  onCleanup(() => {
    if (running()) {
      stopCapture().catch(() => {});
      throttle?.stop();
    }
    roll?.destroy();
    roll = null;
  });

  return (
    <div class="live-view">
      <div class="toolbar">
        <button class="primary" onClick={() => (running() ? stop() : start())}>
          {running() ? "정지" : "시작"}
        </button>
        <span class="status">{status()}</span>
        <div class="readout">
          <Show
            when={readout()?.voiced && readout()}
            fallback={<span class="note muted">—</span>}
          >
            {(f) => (
              <>
                <span class="note">{midiToNoteName(f().midi)}</span>
                <span class="cents">{formatCents(f().cents)}c</span>
              </>
            )}
          </Show>
          <Show when={readout()}>
            {(f) => (
              <span class="meta">
                {f().rms.toFixed(1)} dBFS · conf {f().confidence.toFixed(2)}
              </span>
            )}
          </Show>
        </div>
      </div>
      <div class="roll-host" ref={host} />
    </div>
  );
}

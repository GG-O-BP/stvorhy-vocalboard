import { createSignal, onCleanup, Show } from "solid-js";
import { practiceStart, practiceStop, trackSeries } from "../lib/ipc.js";
import { attachMobileHandlers } from "../lib/mobileHardening.js";
import { formatCents, midiToNoteName } from "../lib/noteNames.js";
import { createReadoutThrottle } from "../lib/throttle.js";
import { createPracticeRoll } from "../render/practiceRoll.js";

/** @typedef {import("../lib/types.js").TrackListItem} TrackListItem */
/** @typedef {import("../lib/types.js").PitchFrame} PitchFrame */
/** @typedef {import("../lib/types.js").FinalizedSession} FinalizedSession */

/**
 * 연습 화면: 레퍼런스 컨투어+노트 오버레이 위에 사용자 피치를 실시간
 * 표시하고, 종료 시 채점 요약을 보여준다.
 * @param {{ track: TrackListItem, onBack: () => void }} props
 */
export default function PracticeView(props) {
  const [running, setRunning] = createSignal(false);
  const [status, setStatus] = createSignal("");
  const [readout, setReadout] = createSignal(/** @type {PitchFrame | null} */ (null));
  const [summary, setSummary] = createSignal(/** @type {FinalizedSession | null} */ (null));
  const [needConfirm, setNeedConfirm] = createSignal(/** @type {string | null} */ (null));

  /** @type {HTMLDivElement | undefined} */
  let host;
  /** @type {Awaited<ReturnType<typeof createPracticeRoll>> | null} */
  let roll = null;
  /** @type {ReturnType<typeof createReadoutThrottle<PitchFrame>> | null} */
  let throttle = null;
  let pendingPlayback = true;
  /** @type {(() => void) | null} */
  let detachMobile = null;

  // Phase 4: 연습 중 인터럽션/라우트 이탈은 세션 종료(채점 저장)로 처리.
  attachMobileHandlers({
    isActive: () => running(),
    playbackActive: true,
    restart: async () => {
      await stop();
    },
    suspend: async () => {
      await stop();
    },
    notify: (m) => setStatus(m),
  }).then((d) => {
    detachMobile = d;
  });

  async function ensureRoll() {
    if (roll || !host) return;
    roll = await createPracticeRoll(host);
    // 풀 해상도 레퍼런스 (프레임당 1포인트) + 노트 블록.
    const maxPoints = Math.max(1000, Math.ceil(props.track.duration_ms / 16) + 8);
    const series = await trackSeries(props.track.id, maxPoints);
    /** @type {{s:number,e:number,m:number}[]} */
    const notes = props.track.notes_json ? JSON.parse(props.track.notes_json) : [];
    roll.setReference(series.points, notes);
  }

  /**
   * @param {boolean} playback
   * @param {boolean} override
   */
  async function start(playback, override = false) {
    if (running()) return;
    pendingPlayback = playback;
    setSummary(null);
    setNeedConfirm(null);
    try {
      await ensureRoll();
      throttle = createReadoutThrottle((f) => setReadout(f), 80);
      const info = await practiceStart(
        props.track.id,
        playback,
        override,
        (frame) => {
          roll?.pushFrame(frame);
          throttle?.push(frame);
        },
        (e) => {
          roll?.setPlayhead(e.t);
          if (e.done) {
            stop();
          }
        },
      );
      setStatus(
        `${info.capture.sample_rate}Hz · ${roll?.rendererType()}` +
          (playback ? ` · 반주 ${Math.round(info.duration_ms / 1000)}s` : " · 무반주"),
      );
      setRunning(true);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("HEADPHONE_GATE:")) {
        setNeedConfirm(msg.split("HEADPHONE_GATE:")[1] ?? msg);
      } else {
        setStatus(msg);
      }
    }
  }

  async function stop() {
    if (!running()) return;
    setRunning(false);
    throttle?.stop();
    throttle = null;
    try {
      const s = await practiceStop();
      setSummary(s);
      setStatus(s ? "세션 저장됨" : "저장할 프레임 없음");
    } catch (e) {
      setStatus(String(e));
    }
  }

  onCleanup(() => {
    detachMobile?.();
    if (running()) {
      practiceStop().catch(() => {});
      throttle?.stop();
    }
    roll?.destroy();
    roll = null;
  });

  queueMicrotask(() => requestAnimationFrame(() => ensureRoll().catch((e) => setStatus(String(e)))));

  return (
    <div class="practice">
      <div class="list-head">
        <button onClick={() => props.onBack()}>← 트랙</button>
        <h2>{props.track.title ?? props.track.id}</h2>
        <span class="status">{status()}</span>
        <div class="readout">
          <Show when={readout()?.voiced && readout()} fallback={<span class="note muted">—</span>}>
            {(f) => (
              <>
                <span class="note">{midiToNoteName(f().midi)}</span>
                <span class="cents">{formatCents(f().cents)}c</span>
              </>
            )}
          </Show>
        </div>
      </div>

      <Show when={needConfirm()}>
        <div class="gate-confirm">
          <p>{needConfirm()}</p>
          <button class="primary" onClick={() => start(pendingPlayback, true)}>
            헤드폰을 사용 중입니다 — 계속
          </button>
          <button onClick={() => setNeedConfirm(null)}>취소</button>
        </div>
      </Show>

      <div class="roll-host" ref={host} />

      <div class="toolbar">
        <Show
          when={running()}
          fallback={
            <>
              <button class="primary" onClick={() => start(true)}>
                반주 연습
              </button>
              <button class="primary" onClick={() => start(false)}>
                무반주 연습
              </button>
            </>
          }
        >
          <button class="primary" onClick={stop}>
            종료·채점
          </button>
        </Show>
        <Show when={summary()}>
          {(s) => (
            <span class="summary">
              점수{" "}
              <b>
                {s().mean_score !== null
                  ? Math.round(/** @type {number} */ (s().mean_score))
                  : "—"}
              </b>
              <Show when={s().mean_abs_cents !== null}>
                {" "}
                · 평균 |Δ| {Math.round(/** @type {number} */ (s().mean_abs_cents))}c
              </Show>{" "}
              · 유성 {Math.round(s().voiced_ratio * 100)}%
            </span>
          )}
        </Show>
      </div>
    </div>
  );
}

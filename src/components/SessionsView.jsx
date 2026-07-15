import { createSignal, For, onCleanup, Show } from "solid-js";
import {
  listSessions,
  playbackPause,
  playbackResume,
  playbackSeek,
  playbackStop,
  playRecording,
  sessionDetail,
  sessionSeries,
} from "../lib/ipc.js";
import { createReviewGraph, drawPreviewThumb } from "../render/reviewGraph.js";

/** @typedef {import("../lib/types.js").SessionListItem} SessionListItem */
/** @typedef {import("../lib/types.js").SessionDetail} SessionDetail */

/** @param {number} ms */
function fmtClock(ms) {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** @param {number} epochMs */
function fmtDate(epochMs) {
  const d = new Date(epochMs);
  const p = (/** @type {number} */ n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

export default function SessionsView() {
  const [items, setItems] = createSignal(/** @type {SessionListItem[]} */ ([]));
  const [error, setError] = createSignal("");
  const [detail, setDetail] = createSignal(/** @type {SessionDetail | null} */ (null));

  function refresh() {
    listSessions()
      .then((rows) => {
        setItems(rows);
        setError("");
      })
      .catch((e) => setError(String(e)));
  }
  refresh();

  return (
    <div class="sessions">
      <Show when={!detail()}>
        <div class="session-list">
          <div class="list-head">
            <h2>세션</h2>
            <button onClick={refresh}>새로고침</button>
          </div>
          <Show when={error()}>
            <p class="error">{error()}</p>
          </Show>
          <Show when={items().length === 0 && !error()}>
            <p class="placeholder">저장된 세션이 없습니다. 라이브 탭에서 녹음해 보세요.</p>
          </Show>
          <ul>
            <For each={items()}>
              {(item) => (
                <li
                  onClick={() =>
                    sessionDetail(item.id)
                      .then(setDetail)
                      .catch((e) => setError(String(e)))
                  }
                >
                  <canvas
                    width="128"
                    height="36"
                    class="thumb"
                    ref={(c) => queueMicrotask(() => drawPreviewThumb(c, item.preview))}
                  />
                  <div class="meta">
                    <span class="title">
                      {fmtDate(item.started_at)}
                      <Show when={item.track_title}> · {item.track_title}</Show>
                    </span>
                    <span class="sub">
                      {fmtClock(item.duration_ms)} · 유성 {Math.round(item.voiced_ratio * 100)}%
                      <Show when={item.mean_score !== null}>
                        {" "}
                        · 점수 {Math.round(/** @type {number} */ (item.mean_score))}
                      </Show>
                      <Show when={item.has_recording}> · 🎙</Show>
                    </span>
                  </div>
                </li>
              )}
            </For>
          </ul>
        </div>
      </Show>
      <Show when={detail()}>
        {(d) => <SessionDetailPane detail={d()} onBack={() => { setDetail(null); refresh(); }} />}
      </Show>
    </div>
  );
}

/**
 * @param {{ detail: SessionDetail, onBack: () => void }} props
 */
function SessionDetailPane(props) {
  const [status, setStatus] = createSignal("");
  const [playing, setPlaying] = createSignal(false);
  const [cursor, setCursor] = createSignal(/** @type {number | null} */ (null));

  /** @type {HTMLCanvasElement | undefined} */
  let canvas;
  /** @type {ReturnType<typeof createReviewGraph> | null} */
  let graph = null;
  let started = false;

  function initGraph() {
    if (!canvas) return;
    const host = canvas.parentElement;
    canvas.width = host ? host.clientWidth - 4 : 800;
    canvas.height = 320;
    sessionSeries(props.detail.id, Math.max(100, canvas.width))
      .then((series) => {
        graph = createReviewGraph(/** @type {HTMLCanvasElement} */ (canvas), series);
        graph.draw(null);
      })
      .catch((e) => setStatus(String(e)));
  }
  queueMicrotask(() => requestAnimationFrame(initGraph));

  async function togglePlay() {
    try {
      if (!props.detail.recording_path) {
        setStatus("이 세션에는 녹음이 없습니다");
        return;
      }
      if (!started) {
        await playRecording(props.detail.id, cursor() ?? 0, (e) => {
          setCursor(e.t);
          setPlaying(e.playing);
          graph?.draw(e.t);
        });
        started = true;
        setPlaying(true);
      } else if (playing()) {
        await playbackPause();
        setPlaying(false);
      } else {
        await playbackResume();
        setPlaying(true);
      }
    } catch (e) {
      setStatus(String(e));
    }
  }

  /** @param {MouseEvent} e */
  function seekAt(e) {
    if (!graph || !canvas) return;
    const rect = canvas.getBoundingClientRect();
    const ms = graph.msAtX(e.clientX - rect.left);
    setCursor(ms);
    graph.draw(ms);
    if (started) {
      playbackSeek(ms).catch(() => {});
    }
  }

  onCleanup(() => {
    playbackStop().catch(() => {});
  });

  const d = () => props.detail;

  return (
    <div class="session-detail">
      <div class="list-head">
        <button onClick={() => props.onBack()}>← 목록</button>
        <h2>{fmtDate(d().started_at)}</h2>
        <span class="sub">
          {fmtClock(d().duration_ms)} · 유성 {Math.round(d().voiced_ratio * 100)}%
          <Show when={d().midi_min !== null}>
            {" "}
            · 음역 {Math.round(/** @type {number} */ (d().midi_min))}–
            {Math.round(/** @type {number} */ (d().midi_max))}
          </Show>
          <Show when={d().mean_score !== null}>
            {" "}
            · 점수 {Math.round(/** @type {number} */ (d().mean_score))}
          </Show>
        </span>
      </div>
      <div class="graph-host">
        <canvas ref={canvas} onClick={seekAt} />
      </div>
      <div class="toolbar">
        <button class="primary" onClick={togglePlay} disabled={!d().recording_path}>
          {playing() ? "일시정지" : "재생"}
        </button>
        <Show when={cursor() !== null}>
          <span class="status">{fmtClock(/** @type {number} */ (cursor()))}</span>
        </Show>
        <Show when={!d().recording_path}>
          <span class="status">녹음 없음</span>
        </Show>
        <Show when={status()}>
          <span class="status error">{status()}</span>
        </Show>
      </div>
    </div>
  );
}

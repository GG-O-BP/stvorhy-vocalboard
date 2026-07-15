import { createSignal, For, Show } from "solid-js";
import { open } from "@tauri-apps/plugin-dialog";
import { deleteTrack, importTrack, karaokeStart, listTracks, playbackStop, separateTrack } from "../lib/ipc.js";
import { drawPreviewThumb } from "../render/reviewGraph.js";
import PracticeView from "./PracticeView.jsx";

/** @typedef {import("../lib/types.js").TrackListItem} TrackListItem */
/** @typedef {import("../lib/types.js").SepProgress} SepProgress */

const STAGE_LABEL = {
  download: "모델 다운로드",
  decode: "디코딩",
  separate: "보컬 분리",
  save: "스템 저장",
  extract: "피치 추출",
  done: "완료",
  error: "오류",
};

/** @param {number} ms */
function fmtClock(ms) {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export default function TracksView() {
  const [items, setItems] = createSignal(/** @type {TrackListItem[]} */ ([]));
  const [error, setError] = createSignal("");
  const [progress, setProgress] = createSignal(
    /** @type {Record<string, SepProgress>} */ ({}),
  );
  const [practice, setPractice] = createSignal(/** @type {TrackListItem | null} */ (null));
  const [karaoke, setKaraoke] = createSignal(/** @type {string | null} */ (null));

  function refresh() {
    listTracks()
      .then((rows) => {
        setItems(rows);
        setError("");
      })
      .catch((e) => setError(String(e)));
  }
  refresh();

  async function doImport() {
    try {
      const picked = await open({
        multiple: false,
        directory: false,
        filters: [
          { name: "오디오", extensions: ["mp3", "m4a", "aac", "flac", "wav", "ogg"] },
        ],
      });
      if (!picked) return;
      const path = typeof picked === "string" ? picked : String(picked);
      await importTrack(path);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  /** @param {TrackListItem} item */
  async function doSeparate(item) {
    try {
      await separateTrack(item.id, (p) => {
        setProgress((prev) => ({ ...prev, [item.id]: p }));
        if (p.stage === "done") {
          refresh();
        }
        if (p.stage === "error") {
          setError(p.message ?? "분리 실패");
        }
      });
    } catch (e) {
      setError(String(e));
    }
  }

  /** @param {TrackListItem} item */
  async function doKaraoke(item) {
    try {
      if (karaoke() === item.id) {
        await playbackStop();
        setKaraoke(null);
        return;
      }
      await karaokeStart(item.id, (e) => {
        if (e.done) setKaraoke(null);
      });
      setKaraoke(item.id);
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div class="tracks">
      <Show when={!practice()}>
        <div class="list-head">
          <h2>레퍼런스 트랙</h2>
          <button onClick={doImport}>파일 가져오기</button>
          <button onClick={refresh}>새로고침</button>
        </div>
        <Show when={error()}>
          <p class="error">{error()}</p>
        </Show>
        <Show when={items().length === 0 && !error()}>
          <p class="placeholder">
            오디오 파일을 가져오면 보컬 분리 → 레퍼런스 피치 추출 → 연습을 시작할 수 있습니다.
          </p>
        </Show>
        <ul class="track-list">
          <For each={items()}>
            {(item) => {
              const prog = () => progress()[item.id];
              const busy = () =>
                prog() && prog().stage !== "done" && prog().stage !== "error";
              return (
                <li>
                  <canvas
                    width="128"
                    height="36"
                    class="thumb"
                    ref={(c) => queueMicrotask(() => drawPreviewThumb(c, item.preview))}
                  />
                  <div class="meta">
                    <span class="title">{item.title ?? item.id}</span>
                    <span class="sub">
                      {fmtClock(item.duration_ms)}
                      <Show when={item.separated}> · 분리됨 ({item.sep_model})</Show>
                      <Show when={busy()}>
                        {" "}
                        · {STAGE_LABEL[/** @type {SepProgress} */ (prog()).stage]}{" "}
                        {Math.round(/** @type {SepProgress} */ (prog()).progress * 100)}%
                      </Show>
                    </span>
                  </div>
                  <div class="actions">
                    <Show when={!item.separated}>
                      <button disabled={busy()} onClick={() => doSeparate(item)}>
                        분리
                      </button>
                    </Show>
                    <Show when={item.separated}>
                      <button class="primary" onClick={() => setPractice(item)}>
                        연습
                      </button>
                      <button onClick={() => doKaraoke(item)}>
                        {karaoke() === item.id ? "노래방 정지" : "노래방"}
                      </button>
                      <button disabled={busy()} onClick={() => doSeparate(item)}>
                        재분리
                      </button>
                    </Show>
                    <button
                      onClick={() =>
                        deleteTrack(item.id)
                          .then(refresh)
                          .catch((e) => setError(String(e)))
                      }
                    >
                      삭제
                    </button>
                  </div>
                </li>
              );
            }}
          </For>
        </ul>
      </Show>
      <Show when={practice()}>
        {(t) => <PracticeView track={t()} onBack={() => { setPractice(null); refresh(); }} />}
      </Show>
    </div>
  );
}

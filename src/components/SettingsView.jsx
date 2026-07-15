import { createSignal, Show } from "solid-js";
import { DEFAULTS, loadSettings, saveSettings } from "../lib/settings.js";

/** @typedef {import("../lib/settings.js").AppConfig} AppConfig */

/**
 * 설정 탭. 로드/저장은 lib/settings.js, 뷰는 얇게.
 */
export default function SettingsView() {
  const [cfg, setCfg] = createSignal(/** @type {AppConfig} */ ({ ...DEFAULTS }));
  const [state, setState] = createSignal("로드 중…");

  loadSettings()
    .then((c) => {
      setCfg(c);
      setState("");
    })
    .catch((e) => setState(`로드 실패: ${e}`));

  /**
   * @param {Partial<AppConfig>} patch
   */
  function update(patch) {
    const next = { ...cfg(), ...patch };
    setCfg(next);
    setState("저장 중…");
    saveSettings(next)
      .then(() => setState("저장됨"))
      .catch((e) => setState(`저장 실패: ${e}`));
  }

  return (
    <div class="settings">
      <h2>설정</h2>
      <Show when={state()}>
        <p class="settings-state">{state()}</p>
      </Show>

      <section>
        <h3>DSP</h3>
        <label>
          RMS 게이트 임계 (dBFS)
          <input
            type="number"
            min="-96"
            max="0"
            step="1"
            value={cfg().gate_dbfs}
            onChange={(e) => update({ gate_dbfs: Number(e.currentTarget.value) })}
          />
        </label>
        <label>
          Confidence 임계
          <input
            type="number"
            min="0"
            max="1"
            step="0.05"
            value={cfg().conf_threshold}
            onChange={(e) => update({ conf_threshold: Number(e.currentTarget.value) })}
          />
        </label>
      </section>

      <section>
        <h3>녹음</h3>
        <label class="row">
          <input
            type="checkbox"
            checked={cfg().recording_enabled}
            onChange={(e) => update({ recording_enabled: e.currentTarget.checked })}
          />
          세션 중 마이크 녹음 (WAV)
        </label>
        <label>
          녹음 보존 개수 (최근 N개)
          <input
            type="number"
            min="1"
            max="1000"
            step="1"
            value={cfg().recording_keep_last}
            onChange={(e) => update({ recording_keep_last: Number(e.currentTarget.value) })}
          />
        </label>
      </section>

      <section>
        <h3>연습·채점</h3>
        <label class="row">
          <input
            type="checkbox"
            checked={cfg().octave_invariant}
            onChange={(e) => update({ octave_invariant: e.currentTarget.checked })}
          />
          옥타브 불변 채점 (mod 12)
        </label>
        <label>
          재생 지연 캘리브레이션 (ms)
          <input
            type="number"
            min="-500"
            max="500"
            step="5"
            value={cfg().latency_calib_ms}
            onChange={(e) => update({ latency_calib_ms: Number(e.currentTarget.value) })}
          />
        </label>
        <label class="row">
          <input
            type="checkbox"
            checked={cfg().separation_quality}
            onChange={(e) => update({ separation_quality: e.currentTarget.checked })}
          />
          보컬 분리 품질 모드 (HTDemucs — 느림, 기본은 고속 MDX)
        </label>
      </section>
    </div>
  );
}

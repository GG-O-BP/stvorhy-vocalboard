/**
 * 연습 모드 피아노롤 (PixiJS): 레퍼런스 컨투어 + 노트 블록 오버레이 +
 * 사용자 트레이스. now-line이 35% 지점에 있고 우측은 다가올 레퍼런스,
 * 좌측은 지나간 레퍼런스와 사용자 트레이스.
 *
 * 라이브 롤(스크롤 텍스처)과 달리 시간 창이 플레이헤드에 종속되므로
 * 레이어 단위 재드로우 방식을 쓴다 (가시 프리미티브 수백 개 수준 —
 * 62.5Hz 재드로우는 GPU에 사소).
 * @file
 */

import { Application, Container, Graphics, Text } from "pixi.js";
import { midiToY } from "./pianoRoll.js";

/** @typedef {import("../lib/types.js").PitchFrame} PitchFrame */
/** @typedef {import("../lib/types.js").SeriesPoint} SeriesPoint */
/** @typedef {{ s: number, e: number, m: number }} NoteBlock */

/** 시간 → 픽셀 (125px/s, 라이브 롤과 동일 스케일). */
export const PX_PER_MS = 0.125;
/** now-line 위치 (폭 비율). */
export const NOW_RATIO = 0.35;

/**
 * 플레이헤드 기준 시각 t의 x 좌표.
 * @param {number} t ms
 * @param {number} playhead ms
 * @param {number} width px
 */
export function timeToX(t, playhead, width) {
  return width * NOW_RATIO + (t - playhead) * PX_PER_MS;
}

const C = {
  bg: 0x14151a,
  gridOctave: 0x33364a,
  gridSemi: 0x1d1f29,
  nowLine: 0x3d4157,
  refNote: 0x8a5c2e,
  refNoteFill: 0x5d3f21,
  refContour: 0xffa94f,
  user: 0x4fc1ff,
  label: 0x8b8fa3,
};

/**
 * @param {HTMLElement} host
 * @param {{ midiLo?: number, midiHi?: number }} [opts]
 */
export async function createPracticeRoll(host, opts = {}) {
  const midiLo = opts.midiLo ?? 40; // E2
  const midiHi = opts.midiHi ?? 88; // E6
  const width = Math.max(320, host.clientWidth | 0);
  const height = Math.max(200, host.clientHeight | 0);

  const app = new Application();
  await app.init({
    width,
    height,
    preference: "webgpu",
    background: C.bg,
    antialias: true,
  });
  host.appendChild(app.canvas);

  // 그리드 (정적).
  const grid = new Graphics();
  for (let m = midiLo; m <= midiHi; m++) {
    const y = midiToY(m, midiLo, midiHi, height);
    const isC = m % 12 === 0;
    grid.moveTo(0, y).lineTo(width, y).stroke({
      width: 1,
      color: isC ? C.gridOctave : C.gridSemi,
      alpha: isC ? 1 : 0.6,
    });
  }
  app.stage.addChild(grid);
  for (let m = midiLo; m <= midiHi; m += 12) {
    if (m % 12 !== 0) continue;
    const label = new Text({
      text: `C${Math.floor(m / 12) - 1}`,
      style: { fontSize: 10, fill: C.label },
    });
    label.position.set(4, midiToY(m, midiLo, midiHi, height) - 12);
    app.stage.addChild(label);
  }

  const refLayer = new Graphics();
  const userLayer = new Graphics();
  app.stage.addChild(refLayer, userLayer);

  const nowLine = new Graphics();
  const nowX = width * NOW_RATIO;
  nowLine.moveTo(nowX, 0).lineTo(nowX, height).stroke({ width: 2, color: C.nowLine });
  app.stage.addChild(nowLine);

  /** @type {SeriesPoint[]} */
  let refPoints = [];
  /** @type {NoteBlock[]} */
  let refNotes = [];
  let playhead = 0;
  /** 사용자 트레이스 링 (t, y | null). 좌측 창 + 여유만 유지. */
  /** @type {Array<{ t: number, y: number | null, voiced: boolean }>} */
  let userTrail = [];
  let destroyed = false;

  const leftWindowMs = (width * NOW_RATIO) / PX_PER_MS + 500;
  const rightWindowMs = (width * (1 - NOW_RATIO)) / PX_PER_MS + 500;

  function redrawRef() {
    refLayer.clear();
    const tMin = playhead - leftWindowMs;
    const tMax = playhead + rightWindowMs;
    // 노트 블록.
    for (const n of refNotes) {
      if (n.e < tMin || n.s > tMax) continue;
      const x0 = timeToX(n.s, playhead, width);
      const x1 = timeToX(n.e, playhead, width);
      const y = midiToY(n.m, midiLo, midiHi, height);
      refLayer
        .roundRect(x0, y - 5, Math.max(2, x1 - x0), 10, 3)
        .fill({ color: C.refNoteFill, alpha: 0.85 })
        .stroke({ width: 1, color: C.refNote });
    }
    // 컨투어 (mid = (min+max)/2).
    let started = false;
    for (const p of refPoints) {
      if (p.t < tMin || p.t > tMax || p.min === null || p.max === null) {
        started = false;
        continue;
      }
      const x = timeToX(p.t, playhead, width);
      const y = midiToY((p.min + p.max) / 2, midiLo, midiHi, height);
      if (!started) {
        refLayer.moveTo(x, y);
        started = true;
      } else {
        refLayer.lineTo(x, y);
      }
    }
    refLayer.stroke({ width: 1.5, color: C.refContour, alpha: 0.9 });
  }

  function redrawUser() {
    userLayer.clear();
    let started = false;
    for (const u of userTrail) {
      if (u.y === null) {
        started = false;
        continue;
      }
      const x = timeToX(u.t, playhead, width);
      if (x < -10 || x > nowX + 10) {
        started = false;
        continue;
      }
      if (!started) {
        userLayer.moveTo(x, u.y);
        started = true;
      } else {
        userLayer.lineTo(x, u.y);
      }
    }
    userLayer.stroke({ width: 2.5, color: C.user, alpha: 0.95, cap: "round" });
  }

  return {
    /**
     * 레퍼런스 데이터 설정 (풀 해상도 시리즈 + 노트).
     * @param {SeriesPoint[]} points
     * @param {NoteBlock[]} notes
     */
    setReference(points, notes) {
      refPoints = points;
      refNotes = notes;
      redrawRef();
    },

    /**
     * 재생 플레이헤드 갱신 (~20Hz Channel 이벤트).
     * @param {number} ms
     */
    setPlayhead(ms) {
      if (destroyed) return;
      playhead = ms;
      redrawRef();
      redrawUser();
    },

    /**
     * 사용자 프레임 (62.5Hz Channel 직행).
     * @param {PitchFrame} frame
     */
    pushFrame(frame) {
      if (destroyed) return;
      const drawable = frame.f0 > 0 && (frame.voiced || frame.confidence >= 0.5);
      userTrail.push({
        t: frame.t,
        y: drawable ? midiToY(frame.midi, midiLo, midiHi, height) : null,
        voiced: frame.voiced,
      });
      const cutoff = frame.t - leftWindowMs;
      if (userTrail.length > 4 && userTrail[0].t < cutoff - 1000) {
        userTrail = userTrail.filter((u) => u.t >= cutoff);
      }
      // 무반주 모드(플레이헤드 없음)에서는 사용자 프레임이 시계 역할.
      if (playhead < frame.t) {
        playhead = frame.t;
        redrawRef();
      }
      redrawUser();
    },

    rendererType() {
      return app.renderer.name;
    },

    destroy() {
      destroyed = true;
      app.destroy(true, { children: true, texture: true });
    },
  };
}

/**
 * PixiJS v8 스크롤 피아노롤 (스펙 §3: WebGPU 우선 + WebGL 폴백,
 * 스크롤 텍스처 방식).
 *
 * 프레임워크 무관 모듈 — Channel 콜백에서 `pushFrame`을 직접 호출한다
 * (Solid 반응성 우회). 스크롤은 핑퐁 RenderTexture로 구현: 이전 텍스처를
 * -PX_PER_FRAME 위치의 스프라이트로 + 오른쪽 끝 새 컬럼을 함께 다음
 * 텍스처에 렌더하고 스왑한다. 트레이스 레이어는 투명(알파 0 클리어)이라
 * 아래 그리드 레이어가 비쳐 보인다.
 * @file
 */

import {
  Application,
  Container,
  Graphics,
  RenderTexture,
  Sprite,
  Text,
} from "pixi.js";

/** @typedef {import("../lib/types.js").PitchFrame} PitchFrame */

/** 프레임(16ms)당 스크롤 픽셀. 2px → 125px/s. */
export const PX_PER_FRAME = 2;

/**
 * midi → y 좌표 (위가 고음). 표시 범위 밖은 클램프.
 * @param {number} midi
 * @param {number} lo 표시 하한 midi
 * @param {number} hi 표시 상한 midi
 * @param {number} height 캔버스 높이 px
 * @returns {number}
 */
export function midiToY(midi, lo, hi, height) {
  const clamped = Math.min(hi, Math.max(lo, midi));
  return ((hi - clamped) / (hi - lo)) * height;
}

const COLORS = {
  bg: 0x14151a,
  gridOctave: 0x33364a,
  gridSemi: 0x1d1f29,
  voiced: 0x4fc1ff,
  lowConf: 0x8a8fa8,
  nowLine: 0x2c2f3f,
  label: 0x8b8fa3,
  reference: 0xffa94f,
};

/**
 * @param {HTMLElement} host 렌더 캔버스를 붙일 요소 (크기 결정)
 * @param {{ midiLo?: number, midiHi?: number }} [opts]
 */
export async function createPianoRoll(host, opts = {}) {
  const midiLo = opts.midiLo ?? 36; // C2
  const midiHi = opts.midiHi ?? 84; // C6
  const width = Math.max(320, host.clientWidth | 0);
  const height = Math.max(200, host.clientHeight | 0);

  const app = new Application();
  await app.init({
    width,
    height,
    preference: "webgpu",
    background: COLORS.bg,
    antialias: true,
  });
  host.appendChild(app.canvas);

  // ── 정적 배경: 반음 그리드 + C옥타브 라벨 ──
  const grid = new Graphics();
  for (let m = midiLo; m <= midiHi; m++) {
    const y = midiToY(m, midiLo, midiHi, height);
    const isC = m % 12 === 0;
    grid.moveTo(0, y).lineTo(width, y).stroke({
      width: 1,
      color: isC ? COLORS.gridOctave : COLORS.gridSemi,
      alpha: isC ? 1 : 0.6,
    });
  }
  app.stage.addChild(grid);
  for (let m = midiLo; m <= midiHi; m += 12) {
    if (m % 12 !== 0) continue;
    const label = new Text({
      text: `C${Math.floor(m / 12) - 1}`,
      style: { fontSize: 10, fill: COLORS.label },
    });
    label.position.set(4, midiToY(m, midiLo, midiHi, height) - 12);
    app.stage.addChild(label);
  }

  // ── 핑퐁 스크롤 텍스처 (투명 클리어 → 그리드 투과) ──
  let rtCur = RenderTexture.create({ width, height });
  let rtNext = RenderTexture.create({ width, height });
  const histSprite = new Sprite(rtCur);
  const column = new Graphics();
  const compose = new Container();
  compose.addChild(histSprite, column);

  const displaySprite = new Sprite(rtCur);
  app.stage.addChild(displaySprite);

  // ── 현재 위치 표시 ──
  const nowLine = new Graphics();
  nowLine.moveTo(width - 1, 0).lineTo(width - 1, height).stroke({ width: 2, color: COLORS.nowLine });
  app.stage.addChild(nowLine);
  const dot = new Graphics();
  dot.circle(0, 0, 4).fill(COLORS.voiced);
  dot.visible = false;
  app.stage.addChild(dot);

  /** @type {number | null} */
  let prevY = null;
  let destroyed = false;
  const TRANSPARENT = [0, 0, 0, 0];

  return {
    /**
     * 62.5Hz Channel 콜백 직행 경로.
     * @param {PitchFrame} frame
     */
    pushFrame(frame) {
      if (destroyed) return;
      // 발성 온셋의 극저신뢰 추정(f0 튐)은 그리지 않는다.
      const drawable = frame.f0 > 0 && (frame.voiced || frame.confidence >= 0.5);
      const y = drawable ? midiToY(frame.midi, midiLo, midiHi, height) : null;

      histSprite.texture = rtCur;
      histSprite.position.set(-PX_PER_FRAME, 0);
      column.clear();
      if (y !== null) {
        const from = prevY ?? y;
        column
          .moveTo(width - PX_PER_FRAME, from)
          .lineTo(width, y)
          .stroke({
            width: 2.5,
            color: frame.voiced ? COLORS.voiced : COLORS.lowConf,
            alpha: frame.voiced ? Math.max(0.6, frame.confidence) : 0.4,
            cap: "round",
          });
      }
      app.renderer.render({
        container: compose,
        target: rtNext,
        clear: true,
        clearColor: TRANSPARENT,
      });
      [rtCur, rtNext] = [rtNext, rtCur];
      displaySprite.texture = rtCur;
      prevY = y;

      if (frame.voiced && y !== null) {
        dot.visible = true;
        dot.position.set(width - 4, y);
      } else {
        dot.visible = false;
      }
    },

    /** 렌더러 종류 (진단/표시용): 'webgpu' | 'webgl' */
    rendererType() {
      return app.renderer.name;
    },

    clear() {
      for (const rt of [rtCur, rtNext]) {
        app.renderer.render({
          container: new Container(),
          target: rt,
          clear: true,
          clearColor: TRANSPARENT,
        });
      }
      displaySprite.texture = rtCur;
      prevY = null;
    },

    destroy() {
      destroyed = true;
      rtCur.destroy(true);
      rtNext.destroy(true);
      app.destroy(true, { children: true, texture: true });
    },
  };
}

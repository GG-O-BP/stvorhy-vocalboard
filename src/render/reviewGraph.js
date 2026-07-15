/**
 * 세션 리뷰 그래프 (canvas 2D, 정적 시리즈 + 재생 커서 오버레이) 와
 * preview BLOB 썸네일 렌더러. 프레임워크 무관 모듈.
 * @file
 */

/** @typedef {import("../lib/types.js").SessionSeries} SessionSeries */
/** @typedef {import("../lib/types.js").SeriesPoint} SeriesPoint */

/**
 * 시리즈의 voiced midi 범위에 여유를 더해 표시 범위를 정한다.
 * voiced가 없으면 [48, 72] (C3..C5).
 * @param {SeriesPoint[]} points
 * @returns {{ lo: number, hi: number }}
 */
export function displayRange(points) {
  let lo = Infinity;
  let hi = -Infinity;
  for (const p of points) {
    if (p.min !== null) lo = Math.min(lo, p.min);
    if (p.max !== null) hi = Math.max(hi, p.max);
  }
  if (!Number.isFinite(lo) || !Number.isFinite(hi)) {
    return { lo: 48, hi: 72 };
  }
  lo = Math.floor(lo) - 2;
  hi = Math.ceil(hi) + 2;
  if (hi - lo < 12) {
    const pad = (12 - (hi - lo)) / 2;
    lo -= Math.ceil(pad);
    hi += Math.ceil(pad);
  }
  return { lo, hi };
}

/**
 * preview BLOB([버전][256 buckets], §5) → 미니 썸네일을 캔버스에 그린다.
 * @param {HTMLCanvasElement} canvas
 * @param {number[] | null} preview
 */
export function drawPreviewThumb(canvas, preview) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const { width: w, height: h } = canvas;
  ctx.clearRect(0, 0, w, h);
  if (!preview || preview.length < 257) return;
  const buckets = preview.slice(1);
  let lo = 255;
  let hi = 1;
  for (const b of buckets) {
    if (b === 0) continue;
    lo = Math.min(lo, b);
    hi = Math.max(hi, b);
  }
  if (hi < lo) return; // 전부 무성
  const span = Math.max(8, hi - lo + 2);
  ctx.fillStyle = "#4fc1ff";
  for (let i = 0; i < 256; i++) {
    const b = buckets[i];
    if (b === 0) continue;
    const x = (i / 256) * w;
    const y = h - ((b - lo + 1) / span) * h;
    ctx.fillRect(x, y - 1, Math.max(1, w / 256), 2);
  }
}

const G = {
  bg: "#14151a",
  gridOctave: "#33364a",
  gridSemi: "#1d1f29",
  band: "rgba(79, 193, 255, 0.35)",
  line: "#4fc1ff",
  cursor: "#ffa94f",
  label: "#8b8fa3",
};

/**
 * 리뷰 그래프 렌더러. `draw(cursorMs)`로 커서만 갱신해도 시리즈 레이어는
 * 오프스크린 캔버스에 캐시되어 다시 계산되지 않는다.
 * @param {HTMLCanvasElement} canvas
 * @param {SessionSeries} series
 */
export function createReviewGraph(canvas, series) {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const { lo, hi } = displayRange(series.points);
  const duration = Math.max(1, series.duration_ms);

  /** @param {number} midi */
  const yOf = (midi) => ((hi - Math.min(hi, Math.max(lo, midi))) / (hi - lo)) * h;
  /** @param {number} t */
  const xOf = (t) => (t / duration) * w;

  // ── 정적 레이어 (그리드 + 시리즈 밴드) ──
  const staticLayer = document.createElement("canvas");
  staticLayer.width = w;
  staticLayer.height = h;
  const sctx = /** @type {CanvasRenderingContext2D} */ (staticLayer.getContext("2d"));
  sctx.fillStyle = G.bg;
  sctx.fillRect(0, 0, w, h);
  for (let m = lo; m <= hi; m++) {
    const isC = m % 12 === 0;
    sctx.strokeStyle = isC ? G.gridOctave : G.gridSemi;
    sctx.lineWidth = 1;
    sctx.beginPath();
    const y = Math.round(yOf(m)) + 0.5;
    sctx.moveTo(0, y);
    sctx.lineTo(w, y);
    sctx.stroke();
    if (isC) {
      sctx.fillStyle = G.label;
      sctx.font = "10px sans-serif";
      sctx.fillText(`C${Math.floor(m / 12) - 1}`, 4, y - 3);
    }
  }
  // min/max 밴드 + 중앙선.
  sctx.fillStyle = G.band;
  sctx.strokeStyle = G.line;
  sctx.lineWidth = 1.5;
  let run = /** @type {SeriesPoint[]} */ ([]);
  const flush = () => {
    if (run.length === 0) return;
    sctx.beginPath();
    for (let i = 0; i < run.length; i++) {
      const p = run[i];
      const x = xOf(p.t);
      const y = yOf(/** @type {number} */ (p.max));
      if (i === 0) sctx.moveTo(x, y);
      else sctx.lineTo(x, y);
    }
    for (let i = run.length - 1; i >= 0; i--) {
      const p = run[i];
      sctx.lineTo(xOf(p.t), yOf(/** @type {number} */ (p.min)));
    }
    sctx.closePath();
    sctx.fill();
    sctx.beginPath();
    for (let i = 0; i < run.length; i++) {
      const p = run[i];
      const mid = (/** @type {number} */ (p.min) + /** @type {number} */ (p.max)) / 2;
      const x = xOf(p.t);
      const y = yOf(mid);
      if (i === 0) sctx.moveTo(x, y);
      else sctx.lineTo(x, y);
    }
    sctx.stroke();
    run = [];
  };
  for (const p of series.points) {
    if (p.min === null || p.max === null) flush();
    else run.push(p);
  }
  flush();

  /**
   * @param {number | null} cursorMs 재생 커서 위치 (null이면 숨김)
   */
  function draw(cursorMs) {
    if (!ctx) return;
    ctx.clearRect(0, 0, w, h);
    ctx.drawImage(staticLayer, 0, 0);
    if (cursorMs !== null) {
      const x = Math.round(xOf(Math.min(cursorMs, duration))) + 0.5;
      ctx.strokeStyle = G.cursor;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, h);
      ctx.stroke();
    }
  }

  return {
    draw,
    /** 캔버스 x좌표 → 재생 ms (클릭 시킹용). @param {number} x */
    msAtX(x) {
      return Math.round((Math.min(Math.max(x, 0), w) / w) * duration);
    },
  };
}

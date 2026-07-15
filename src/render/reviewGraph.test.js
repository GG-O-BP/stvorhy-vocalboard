import { describe, expect, it } from "vitest";
import { displayRange } from "./reviewGraph.js";

describe("displayRange", () => {
  it("pads voiced range and enforces minimum span", () => {
    const r = displayRange([
      { t: 0, min: 60, max: 62 },
      { t: 16, min: 59.5, max: 63.2 },
    ]);
    expect(r.lo).toBeLessThanOrEqual(57.5);
    expect(r.hi).toBeGreaterThanOrEqual(66);
    expect(r.hi - r.lo).toBeGreaterThanOrEqual(12);
  });

  it("falls back to C3..C5 when all unvoiced", () => {
    expect(displayRange([{ t: 0, min: null, max: null }])).toEqual({ lo: 48, hi: 72 });
    expect(displayRange([])).toEqual({ lo: 48, hi: 72 });
  });
});

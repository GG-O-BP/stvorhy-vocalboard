import { describe, expect, it } from "vitest";
import { NOW_RATIO, PX_PER_MS, timeToX } from "./practiceRoll.js";

describe("timeToX", () => {
  it("maps playhead to the now-line", () => {
    expect(timeToX(5000, 5000, 1000)).toBe(1000 * NOW_RATIO);
  });

  it("future is right of now, past is left", () => {
    const w = 800;
    const now = timeToX(2000, 2000, w);
    expect(timeToX(3000, 2000, w)).toBe(now + 1000 * PX_PER_MS);
    expect(timeToX(1000, 2000, w)).toBe(now - 1000 * PX_PER_MS);
  });
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createReadoutThrottle } from "./throttle.js";

describe("createReadoutThrottle", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("emits at throttle rate, not input rate", () => {
    /** @type {unknown[]} */
    const seen = [];
    const t = createReadoutThrottle((v) => seen.push(v), 80);
    // 62.5Hz 입력 시뮬레이션: 16ms마다 push, 800ms 동안.
    for (let i = 0; i < 50; i++) {
      t.push(i);
      vi.advanceTimersByTime(16);
    }
    // 800ms / 80ms = 10회 근처만 방출되어야 한다.
    expect(seen.length).toBeGreaterThanOrEqual(9);
    expect(seen.length).toBeLessThanOrEqual(11);
    // 항상 최신 값을 방출.
    expect(seen[seen.length - 1]).toBe(49);
    t.stop();
  });

  it("does not emit without new values", () => {
    /** @type {unknown[]} */
    const seen = [];
    const t = createReadoutThrottle((v) => seen.push(v), 80);
    t.push("a");
    vi.advanceTimersByTime(400);
    expect(seen).toEqual(["a"]); // 한 번만
    t.stop();
  });

  it("stop clears the timer", () => {
    /** @type {unknown[]} */
    const seen = [];
    const t = createReadoutThrottle((v) => seen.push(v), 80);
    t.push(1);
    t.stop();
    vi.advanceTimersByTime(400);
    expect(seen).toEqual([]);
  });
});

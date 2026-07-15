import { describe, expect, it } from "vitest";
import { midiToY, PX_PER_FRAME } from "./pianoRoll.js";

describe("midiToY", () => {
  it("maps top to hi and bottom to lo", () => {
    expect(midiToY(84, 36, 84, 480)).toBe(0);
    expect(midiToY(36, 36, 84, 480)).toBe(480);
    expect(midiToY(60, 36, 84, 480)).toBe(240);
  });

  it("clamps out-of-range midi", () => {
    expect(midiToY(100, 36, 84, 480)).toBe(0);
    expect(midiToY(10, 36, 84, 480)).toBe(480);
  });

  it("scroll speed constant sanity", () => {
    // 62.5Hz × PX_PER_FRAME = 스크롤 속도. 8초 이상의 이력이 화면(≥1000px
    // 가정 아님 — 최소 320px에서도 2.5초)을 지나가는지 계산 자체를 고정.
    expect(PX_PER_FRAME * 62.5).toBe(125);
  });
});

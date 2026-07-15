import { describe, expect, it } from "vitest";
import { formatCents, midiToNoteName } from "./noteNames.js";

describe("midiToNoteName", () => {
  it("maps A4=69 per 12-TET", () => {
    expect(midiToNoteName(69)).toBe("A4");
    expect(midiToNoteName(60)).toBe("C4");
    expect(midiToNoteName(61)).toBe("C#4");
    expect(midiToNoteName(59)).toBe("B3");
  });

  it("rounds fractional midi to nearest semitone", () => {
    expect(midiToNoteName(69.4)).toBe("A4");
    expect(midiToNoteName(69.6)).toBe("A#4");
    expect(midiToNoteName(68.5)).toBe("A4");
  });

  it("covers the SwiftF0 range edges (G1..C7)", () => {
    expect(midiToNoteName(31)).toBe("G1");
    expect(midiToNoteName(96)).toBe("C7");
  });
});

describe("formatCents", () => {
  it("formats sign explicitly", () => {
    expect(formatCents(12.3)).toBe("+12");
    expect(formatCents(-3.4)).toBe("-3");
    expect(formatCents(0)).toBe("±0");
    expect(formatCents(-0.2)).toBe("±0");
  });
});

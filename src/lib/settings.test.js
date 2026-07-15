import { describe, expect, it } from "vitest";
import { DEFAULTS, mergeWithDefaults } from "./settings.js";

describe("mergeWithDefaults", () => {
  it("returns defaults for empty/absent saved value", () => {
    expect(mergeWithDefaults(undefined)).toEqual(DEFAULTS);
    expect(mergeWithDefaults(null)).toEqual(DEFAULTS);
    expect(mergeWithDefaults({})).toEqual(DEFAULTS);
  });

  it("overlays known keys and drops unknown/typo keys", () => {
    const merged = mergeWithDefaults({
      gate_dbfs: -60,
      unknown_key: 123,
      octave_invariant: true,
    });
    expect(merged.gate_dbfs).toBe(-60);
    expect(merged.octave_invariant).toBe(true);
    expect(merged.conf_threshold).toBe(DEFAULTS.conf_threshold);
    expect(/** @type {any} */ (merged).unknown_key).toBeUndefined();
  });

  it("rejects type-mismatched values", () => {
    const merged = mergeWithDefaults({ gate_dbfs: "loud", recording_enabled: 1 });
    expect(merged.gate_dbfs).toBe(DEFAULTS.gate_dbfs);
    expect(merged.recording_enabled).toBe(DEFAULTS.recording_enabled);
  });
});

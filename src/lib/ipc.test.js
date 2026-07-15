import { afterEach, describe, expect, it } from "vitest";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { configure, startCapture, stopCapture } from "./ipc.js";
import { DEFAULTS } from "./settings.js";

afterEach(() => {
  clearMocks();
});

describe("ipc contract", () => {
  it("startCapture invokes start_capture with a channel and streams frames", async () => {
    /** @type {any} */
    let channelArg = null;
    mockIPC((cmd, args) => {
      if (cmd === "start_capture") {
        channelArg = /** @type {any} */ (args).channel;
        return { sample_rate: 48000, channels: 1, simulated: true };
      }
      throw new Error(`unexpected cmd ${cmd}`);
    });

    /** @type {import("./types.js").PitchFrame[]} */
    const frames = [];
    const info = await startCapture((f) => frames.push(f));
    expect(info).toEqual({ sample_rate: 48000, channels: 1, simulated: true });
    expect(channelArg).not.toBeNull();

    // 백엔드 송출 시뮬레이션 → onFrame 배선 검증 (types.js PitchFrame 계약).
    const frame = {
      t: 16,
      f0: 440,
      midi: 69,
      cents: 0,
      confidence: 0.97,
      rms: -20.5,
      voiced: true,
    };
    channelArg.onmessage(frame);
    expect(frames).toEqual([frame]);
  });

  it("configure sends the config object under the `config` key", async () => {
    /** @type {any} */
    let seen = null;
    mockIPC((cmd, args) => {
      if (cmd === "configure") {
        seen = /** @type {any} */ (args).config;
      }
    });
    await configure({ ...DEFAULTS, gate_dbfs: -50 });
    expect(seen).toMatchObject({ gate_dbfs: -50, conf_threshold: 0.9 });
  });

  it("stopCapture invokes stop_capture", async () => {
    /** @type {string[]} */
    const calls = [];
    mockIPC((cmd) => {
      calls.push(cmd);
    });
    await stopCapture();
    expect(calls).toEqual(["stop_capture"]);
  });
});

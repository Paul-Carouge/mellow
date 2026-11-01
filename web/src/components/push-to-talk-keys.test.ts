import { describe, expect, it } from "vitest";

import { DEFAULT_VOICE_SETTINGS } from "../lib/voice-settings";
import { pushToTalkKeyDown, pushToTalkKeyUp } from "./push-to-talk-keys";

function fakeVoice(overrides: Partial<typeof DEFAULT_VOICE_SETTINGS> = {}, phase: "idle" | "connected" = "connected") {
  const calls: string[] = [];
  return {
    calls,
    voice: {
      settings: {
        ...DEFAULT_VOICE_SETTINGS,
        inputMode: "push-to-talk" as const,
        pushToTalkKey: { code: "KeyV", label: "V" },
        ...overrides,
      },
      phase,
      pressPushToTalk: () => void calls.push("press"),
      releasePushToTalk: () => void calls.push("release"),
    },
  };
}

function keyEvent(code: string, key: string, modifiers: { ctrlKey?: boolean } = {}) {
  const event = {
    code,
    key,
    ctrlKey: modifiers.ctrlKey ?? false,
    metaKey: false,
    altKey: false,
    target: null,
    isDefaultPrevented: false,
    preventDefault: () => (event.isDefaultPrevented = true),
  };
  return event;
}

describe("push-to-talk keys", () => {
  it("opens on the key and closes on its release, without typing its letter", () => {
    const { voice, calls } = fakeVoice();
    const down = keyEvent("KeyV", "v");
    pushToTalkKeyDown(down, voice);
    pushToTalkKeyUp(keyEvent("KeyV", "v"), voice);
    expect(calls).toEqual(["press", "release"]);
    expect(down.isDefaultPrevented).toBe(true);
  });

  it("leaves a modifier key's default alone", () => {
    const { voice } = fakeVoice({ pushToTalkKey: { code: "ControlRight", label: "Right Ctrl" } });
    const down = keyEvent("ControlRight", "Control");
    pushToTalkKeyDown(down, voice);
    expect(down.isDefaultPrevented).toBe(false);
  });

  it("ignores other keys, voice activity mode, no key, and being out of a call", () => {
    const cases = [
      fakeVoice(),
      fakeVoice({ inputMode: "voice-activity" }),
      fakeVoice({ pushToTalkKey: null }),
      fakeVoice({}, "idle"),
    ];
    pushToTalkKeyDown(keyEvent("KeyB", "b"), cases[0]!.voice);
    for (const { voice } of cases.slice(1)) {
      pushToTalkKeyDown(keyEvent("KeyV", "v"), voice);
    }
    expect(cases.flatMap(({ calls }) => calls)).toEqual([]);
  });
});

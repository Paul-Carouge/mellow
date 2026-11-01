import { describe, expect, it, vi } from "vitest";

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
    target: null as unknown,
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

  it("releases the key even when it comes up inside the key recorder", () => {
    const { voice, calls } = fakeVoice();
    vi.stubGlobal("Element", class {});
    try {
      const recorder = { closest: (selector: string) => (selector === "[data-push-to-talk-recorder]" ? {} : null) };
      Object.setPrototypeOf(recorder, (globalThis as unknown as { Element: { prototype: object } }).Element.prototype);
      pushToTalkKeyDown(keyEvent("KeyV", "v"), voice);
      // Held, then the mouse clicked "Change key": focus moved into the recorder before the release.
      pushToTalkKeyUp({ ...keyEvent("KeyV", "v"), target: recorder }, voice);
      // A press inside the recorder is the recorder's.
      pushToTalkKeyDown({ ...keyEvent("KeyV", "v"), target: recorder }, voice);
    } finally {
      vi.unstubAllGlobals();
      vi.stubGlobal("navigator", { languages: ["en-GB"] });
    }
    expect(calls).toEqual(["press", "release"]);
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

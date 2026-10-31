import { describe, expect, it } from "vitest";

import {
  UNMUTED_VOICE_TOGGLES,
  readVoiceToggles,
  toggleDeafened,
  toggleMicrophone,
  writeVoiceToggles,
} from "./voice-toggles";

describe("voice toggles", () => {
  it("mutes and unmutes the microphone alone", () => {
    const muted = toggleMicrophone(UNMUTED_VOICE_TOGGLES);
    expect(muted.isMicrophoneMuted).toBe(true);
    expect(muted.isDeafened).toBe(false);
    expect(toggleMicrophone(muted).isMicrophoneMuted).toBe(false);
  });

  it("mutes the microphone while deafened, then restores its previous state", () => {
    const deafened = toggleDeafened(UNMUTED_VOICE_TOGGLES);
    expect(deafened).toMatchObject({ isDeafened: true, isMicrophoneMuted: true });
    expect(toggleDeafened(deafened)).toMatchObject({ isDeafened: false, isMicrophoneMuted: false });

    const mutedThenDeafened = toggleDeafened(toggleMicrophone(UNMUTED_VOICE_TOGGLES));
    expect(toggleDeafened(mutedThenDeafened)).toMatchObject({ isDeafened: false, isMicrophoneMuted: true });
  });

  it("undeafens when the microphone is unmuted while deafened", () => {
    const deafened = toggleDeafened(UNMUTED_VOICE_TOGGLES);
    expect(toggleMicrophone(deafened)).toMatchObject({ isDeafened: false, isMicrophoneMuted: false });
  });
});

describe("stored voice toggles", () => {
  function storageWith(initial: string | null): Pick<Storage, "getItem" | "setItem"> {
    let text = initial;
    return {
      getItem: () => text,
      setItem: (_key: string, value: string) => {
        text = value;
      },
    };
  }

  it("round-trips through the browser's storage", () => {
    const storage = storageWith(null);
    const deafened = toggleDeafened(toggleMicrophone(UNMUTED_VOICE_TOGGLES));
    writeVoiceToggles(storage, deafened);
    expect(readVoiceToggles(storage)).toEqual(deafened);
  });

  it("falls back to unmuted on nothing, garbage or a blocked storage", () => {
    expect(readVoiceToggles(null)).toEqual(UNMUTED_VOICE_TOGGLES);
    expect(readVoiceToggles(storageWith("not json"))).toEqual(UNMUTED_VOICE_TOGGLES);
    expect(readVoiceToggles(storageWith("[true]"))).toEqual(UNMUTED_VOICE_TOGGLES);
    const blocked = {
      getItem: (): string | null => {
        throw new Error("blocked");
      },
    };
    expect(readVoiceToggles(blocked)).toEqual(UNMUTED_VOICE_TOGGLES);
  });

  it("never reads deafened without muted", () => {
    expect(readVoiceToggles(storageWith('{"isDeafened":true,"isMicrophoneMuted":false}'))).toMatchObject({
      isDeafened: true,
      isMicrophoneMuted: true,
    });
  });
});

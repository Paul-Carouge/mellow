import { describe, expect, it } from "vitest";

import { UNMUTED_VOICE_TOGGLES, toggleDeafened, toggleMicrophone } from "./voice-toggles";

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

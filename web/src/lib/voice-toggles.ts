// The user card's microphone and headset toggles (ADR-0022 §5), Discord's rules: deafening also
// mutes the microphone, and undeafening gives the microphone back the state it had before.

export interface VoiceToggles {
  readonly isMicrophoneMuted: boolean;
  readonly isDeafened: boolean;
  /** The microphone's state when deafening started, restored when it ends. */
  readonly wasMicrophoneMutedBeforeDeafening: boolean;
}

export const UNMUTED_VOICE_TOGGLES: VoiceToggles = {
  isMicrophoneMuted: false,
  isDeafened: false,
  wasMicrophoneMutedBeforeDeafening: false,
};

export function toggleMicrophone(toggles: VoiceToggles): VoiceToggles {
  // Unmuting the microphone while deafened also undeafens: nobody talks without hearing.
  if (toggles.isDeafened) {
    return { ...toggles, isMicrophoneMuted: false, isDeafened: false };
  }
  return { ...toggles, isMicrophoneMuted: !toggles.isMicrophoneMuted };
}

export function toggleDeafened(toggles: VoiceToggles): VoiceToggles {
  if (toggles.isDeafened) {
    return { ...toggles, isDeafened: false, isMicrophoneMuted: toggles.wasMicrophoneMutedBeforeDeafening };
  }
  return { isDeafened: true, isMicrophoneMuted: true, wasMicrophoneMutedBeforeDeafening: toggles.isMicrophoneMuted };
}

/** Per browser, as on Discord: the toggles hold before joining and across reloads. */
export const VOICE_TOGGLES_STORAGE_KEY = "mellow.voice-toggles";

/** What this browser remembers, field by field; anything unreadable falls back to unmuted. */
export function readVoiceToggles(storage: Pick<Storage, "getItem"> | null): VoiceToggles {
  try {
    const stored: unknown = JSON.parse(storage?.getItem(VOICE_TOGGLES_STORAGE_KEY) ?? "null");
    if (typeof stored !== "object" || stored === null || Array.isArray(stored)) {
      return UNMUTED_VOICE_TOGGLES;
    }
    const fields = stored as Record<string, unknown>;
    const isDeafened = fields["isDeafened"] === true;
    return {
      isDeafened,
      // Deafened always means muted, whatever was stored.
      isMicrophoneMuted: isDeafened || fields["isMicrophoneMuted"] === true,
      wasMicrophoneMutedBeforeDeafening: fields["wasMicrophoneMutedBeforeDeafening"] === true,
    };
  } catch (error) {
    console.warn("unreadable voice toggles", error);
    return UNMUTED_VOICE_TOGGLES;
  }
}

/** A full or blocked storage keeps them for this tab only. */
export function writeVoiceToggles(storage: Pick<Storage, "setItem"> | null, toggles: VoiceToggles): void {
  try {
    storage?.setItem(VOICE_TOGGLES_STORAGE_KEY, JSON.stringify(toggles));
  } catch (error) {
    console.warn("couldn't store the voice toggles", error);
  }
}

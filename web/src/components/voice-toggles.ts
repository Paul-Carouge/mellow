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

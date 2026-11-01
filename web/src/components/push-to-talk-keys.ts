// The push-to-talk key, from anywhere in the tab (docs/design/web-voice-comfort.md). Listened to
// on the window in the capture phase, so it works while typing, as on Discord; a key that types a
// character does not also type it. Browsers only see keys while the tab has focus: losing focus
// with the key held releases it, or the microphone would stay open with nobody pressing.

import { isCharacterKey } from "../lib/voice-settings";
import type { VoiceSession } from "../lib/voice-session.svelte";

type PushToTalkVoice = Pick<VoiceSession, "settings" | "phase" | "pressPushToTalk" | "releasePushToTalk">;

interface KeyEventLike {
  readonly code: string;
  readonly key: string;
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
  readonly altKey: boolean;
  readonly target: unknown;
  preventDefault(): void;
}

/** Keys pressed while recording a new one belong to the recorder. */
function isInKeyRecorder(target: unknown): boolean {
  return typeof Element !== "undefined" && target instanceof Element && target.closest("[data-push-to-talk-recorder]") !== null;
}

function isPushToTalkKey(event: KeyEventLike, voice: PushToTalkVoice): boolean {
  const key = voice.settings.pushToTalkKey;
  return voice.settings.inputMode === "push-to-talk" && key !== null && voice.phase !== "idle" && event.code === key.code;
}

export function pushToTalkKeyDown(event: KeyEventLike, voice: PushToTalkVoice): void {
  if (!isPushToTalkKey(event, voice) || isInKeyRecorder(event.target)) {
    return;
  }
  if (isCharacterKey(event)) {
    event.preventDefault();
  }
  voice.pressPushToTalk();
}

/** Wherever focus went meanwhile (into the recorder too): a lost release would leave the microphone open. */
export function pushToTalkKeyUp(event: KeyEventLike, voice: PushToTalkVoice): void {
  if (isPushToTalkKey(event, voice)) {
    voice.releasePushToTalk();
  }
}

/** Listens on `target` (the window) until the returned function is called. */
export function listenForPushToTalk(target: Window, voice: PushToTalkVoice): () => void {
  const down = (event: KeyboardEvent) => pushToTalkKeyDown(event, voice);
  const up = (event: KeyboardEvent) => pushToTalkKeyUp(event, voice);
  const release = () => voice.releasePushToTalk();
  const releaseWhenHidden = () => {
    if (document.visibilityState === "hidden") {
      voice.releasePushToTalk();
    }
  };
  target.addEventListener("keydown", down, { capture: true });
  target.addEventListener("keyup", up, { capture: true });
  target.addEventListener("blur", release);
  document.addEventListener("visibilitychange", releaseWhenHidden);
  return () => {
    target.removeEventListener("keydown", down, { capture: true });
    target.removeEventListener("keyup", up, { capture: true });
    target.removeEventListener("blur", release);
    document.removeEventListener("visibilitychange", releaseWhenHidden);
  };
}

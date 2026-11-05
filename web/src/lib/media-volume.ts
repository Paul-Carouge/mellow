// The volume a reader last set on a video or a sound (an attachment's player or a YouTube
// player), so the next one starts there instead of at full volume. One value for every player in
// this browser, checked on the way in. docs/design/web-media-volume.md.
//
// shortcut: kept per browser in `localStorage`, so another device keeps its own volume; move it
// to a server setting when settings sync across devices.

import { browserStorage } from "./reactions";

/** `volume` from 0 (silent) to 1 (full), as `HTMLMediaElement.volume`; muting keeps it. */
export interface MediaVolume {
  readonly volume: number;
  readonly isMuted: boolean;
}

const MEDIA_VOLUME_STORAGE_KEY = "mellow.media-volume";

export function isMediaVolume(value: unknown): value is MediaVolume {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const { volume, isMuted } = value as Record<string, unknown>;
  return (
    typeof volume === "number" && Number.isFinite(volume) && volume >= 0 && volume <= 1 && typeof isMuted === "boolean"
  );
}

export function isSameMediaVolume(first: MediaVolume, second: MediaVolume): boolean {
  return first.volume === second.volume && first.isMuted === second.isMuted;
}

/** What this browser remembers; null when nothing is stored or it is not a volume. */
export function readMediaVolume(storage: Pick<Storage, "getItem"> | null): MediaVolume | null {
  let stored: unknown;
  try {
    stored = JSON.parse(storage?.getItem(MEDIA_VOLUME_STORAGE_KEY) ?? "null");
  } catch (error) {
    console.warn("unreadable media volume", error);
    return null;
  }
  return isMediaVolume(stored) ? { volume: stored.volume, isMuted: stored.isMuted } : null;
}

/** Stores the volume when the browser lets us; a full or blocked storage only loses the memory. */
export function writeMediaVolume(storage: Pick<Storage, "setItem"> | null, volume: MediaVolume): void {
  try {
    storage?.setItem(MEDIA_VOLUME_STORAGE_KEY, JSON.stringify({ volume: volume.volume, isMuted: volume.isMuted }));
  } catch (error) {
    console.warn("couldn't remember the media volume", error);
  }
}

/** Stores `volume` unless it is already what is remembered, so a chatty player writes once per change. */
export function rememberMediaVolume(storage: Pick<Storage, "getItem" | "setItem"> | null, volume: MediaVolume): void {
  const remembered = readMediaVolume(storage);
  if (remembered === null || !isSameMediaVolume(remembered, volume)) {
    writeMediaVolume(storage, volume);
  }
}

type VolumeControlledMedia = Pick<HTMLMediaElement, "volume" | "muted" | "addEventListener" | "removeEventListener">;

/**
 * Svelte attachment for an attachment's `<video>` or `<audio>`: starts at the remembered volume and
 * remembers what the reader sets on it.
 */
export function playAtRememberedVolume(
  media: VolumeControlledMedia,
  storage: Pick<Storage, "getItem" | "setItem"> | null = browserStorage(),
): () => void {
  const remembered = readMediaVolume(storage);
  if (remembered !== null) {
    media.volume = remembered.volume;
    media.muted = remembered.isMuted;
  }
  const rememberChange = () => rememberMediaVolume(storage, { volume: media.volume, isMuted: media.muted });
  media.addEventListener("volumechange", rememberChange);
  return () => media.removeEventListener("volumechange", rememberChange);
}

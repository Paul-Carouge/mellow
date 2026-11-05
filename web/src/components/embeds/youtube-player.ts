import { isSameMediaVolume, type MediaVolume, readMediaVolume, rememberMediaVolume } from "../../lib/media-volume";
import { browserStorage } from "../../lib/reactions";

// Talks to the YouTube player in its iframe with `postMessage` only, the protocol YouTube's own
// IFrame API speaks (the page loads no YouTube script and the CSP is unchanged): the player starts
// at the remembered volume and the volume the reader sets is remembered for the next one
// (docs/design/web-media-volume.md). The player is loaded with `enablejsapi=1&origin=…`.

/** The only origin a player message is taken from: the privacy-enhanced player (ADR-0041). */
export const YOUTUBE_PLAYER_ORIGIN = "https://www.youtube-nocookie.com";
/** YouTube's volume runs from 0 to 100 in whole steps. */
const YOUTUBE_FULL_VOLUME = 100;
/** Far above any real player message (a few kilobytes); longer data is not parsed at all. */
const MAXIMUM_PLAYER_MESSAGE_LENGTH = 65_536;

export type YouTubePlayerMessage =
  { readonly kind: "ready" } | { readonly kind: "volume"; readonly volume: MediaVolume };

/**
 * What a player's message says that matters here: it is ready for commands, or its volume.
 * Null for anything else, including malformed data; the caller checks origin and source first.
 */
export function parseYouTubePlayerMessage(data: unknown): YouTubePlayerMessage | null {
  if (typeof data !== "string" || data.length > MAXIMUM_PLAYER_MESSAGE_LENGTH) {
    return null;
  }
  let message: unknown;
  try {
    message = JSON.parse(data);
  } catch {
    // Not JSON: not a player message (extensions post to every frame), nothing to report.
    return null;
  }
  if (typeof message !== "object" || message === null) {
    return null;
  }
  const { event, info } = message as Record<string, unknown>;
  if (event === "onReady") {
    return { kind: "ready" };
  }
  if (event !== "infoDelivery" || typeof info !== "object" || info === null) {
    return null;
  }
  const { volume, muted } = info as Record<string, unknown>;
  const isYouTubeVolume =
    typeof volume === "number" && Number.isInteger(volume) && volume >= 0 && volume <= YOUTUBE_FULL_VOLUME;
  if (!isYouTubeVolume || typeof muted !== "boolean") {
    return null;
  }
  return { kind: "volume", volume: { volume: volume / YOUTUBE_FULL_VOLUME, isMuted: muted } };
}

/** Asks the player to send its events (`onReady`, `infoDelivery`) to this page. */
export function youTubeListeningMessage(): string {
  return JSON.stringify({ event: "listening", id: 1, channel: "widget" });
}

/** The commands that set the player to `volume`. */
export function youTubeVolumeCommands(volume: MediaVolume): string[] {
  const command = (func: string, args: unknown[]) =>
    JSON.stringify({ event: "command", func, args, id: 1, channel: "widget" });
  return [
    command("setVolume", [Math.round(volume.volume * YOUTUBE_FULL_VOLUME)]),
    command(volume.isMuted ? "mute" : "unMute", []),
  ];
}

/** Compared in YouTube's whole steps, since an attachment's player can remember 0.333. */
function isSameYouTubeVolume(first: MediaVolume, second: MediaVolume): boolean {
  const steps = (volume: MediaVolume) => ({
    volume: Math.round(volume.volume * YOUTUBE_FULL_VOLUME),
    isMuted: volume.isMuted,
  });
  return isSameMediaVolume(steps(first), steps(second));
}

/**
 * One player's side of the conversation: on ready, sends the remembered volume; then remembers
 * every volume the player reports. Reports of the player's own starting volume, which can still
 * arrive after the commands, are skipped until the player reports the volume it was sent. The wait
 * ends too at the first report of any other volume (the reader, or YouTube refusing the command,
 * moved it), so a player that never reports the sent volume still remembers the reader's changes.
 */
export function followYouTubeVolume(
  storage: Pick<Storage, "getItem" | "setItem"> | null,
  send: (message: string) => void,
): (message: YouTubePlayerMessage) => void {
  let isReady = false;
  let awaitedVolume: MediaVolume | null = null;
  let startingVolume: MediaVolume | null = null;
  return (message) => {
    if (message.kind === "ready") {
      if (isReady) {
        return;
      }
      isReady = true;
      awaitedVolume = readMediaVolume(storage);
      if (awaitedVolume !== null) {
        youTubeVolumeCommands(awaitedVolume).forEach(send);
      }
      return;
    }
    if (!isReady) {
      startingVolume = message.volume;
      return;
    }
    if (awaitedVolume !== null) {
      if (isSameYouTubeVolume(message.volume, awaitedVolume)) {
        awaitedVolume = null;
        return;
      }
      startingVolume ??= message.volume;
      if (isSameYouTubeVolume(message.volume, startingVolume)) {
        return;
      }
      awaitedVolume = null;
    }
    rememberMediaVolume(storage, message.volume);
  };
}

/**
 * Whether a message comes from this player: another frame (another player, an extension) can post
 * to the page too.
 */
export function isFromYouTubePlayer(event: { origin: string; source: unknown }, playerWindow: unknown): boolean {
  return event.origin === YOUTUBE_PLAYER_ORIGIN && event.source !== null && event.source === playerWindow;
}

/** Svelte attachment for a YouTube player's iframe; see `followYouTubeVolume`. */
export function playYouTubeAtRememberedVolume(player: HTMLIFrameElement): () => void {
  const send = (message: string) => player.contentWindow?.postMessage(message, YOUTUBE_PLAYER_ORIGIN);
  const follow = followYouTubeVolume(browserStorage(), send);
  const listen = () => send(youTubeListeningMessage());
  const receive = (event: MessageEvent) => {
    if (!isFromYouTubePlayer(event, player.contentWindow)) {
      return;
    }
    const message = parseYouTubePlayerMessage(event.data);
    if (message !== null) {
      follow(message);
    }
  };
  player.addEventListener("load", listen);
  window.addEventListener("message", receive);
  return () => {
    player.removeEventListener("load", listen);
    window.removeEventListener("message", receive);
  };
}

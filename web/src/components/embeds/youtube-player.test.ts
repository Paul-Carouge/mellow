import { describe, expect, it } from "vitest";

import { readMediaVolume, writeMediaVolume } from "../../lib/media-volume";
import { memoryStorage } from "../../test-memory-storage";
import {
  followYouTubeVolume,
  isFromYouTubePlayer,
  parseYouTubePlayerMessage,
  youTubeListeningMessage,
  youTubeVolumeCommands,
  type YouTubePlayerMessage,
} from "./youtube-player";

/** A player's `infoDelivery`, as the embed sends it (seen in Chrome on 2026-11-05). */
function delivery(info: Record<string, unknown>): string {
  return JSON.stringify({ event: "infoDelivery", info, channel: "widget", id: 1 });
}

function volumeMessage(volume: number, isMuted = false): YouTubePlayerMessage {
  return { kind: "volume", volume: { volume, isMuted } };
}

describe("parseYouTubePlayerMessage", () => {
  it("reads the player's readiness", () => {
    expect(parseYouTubePlayerMessage('{"event":"onReady","info":null,"channel":"widget","id":1}')).toEqual({
      kind: "ready",
    });
  });

  it("reads a volume report in YouTube's 0 to 100 steps", () => {
    expect(parseYouTubePlayerMessage(delivery({ muted: false, volume: 100 }))).toEqual(volumeMessage(1));
    expect(parseYouTubePlayerMessage(delivery({ muted: true, volume: 35, currentTime: 12.5 }))).toEqual(
      volumeMessage(0.35, true),
    );
  });

  it("ignores reports without both a volume and a muted state", () => {
    expect(parseYouTubePlayerMessage(delivery({ currentTime: 12.5 }))).toBeNull();
    expect(parseYouTubePlayerMessage(delivery({ volume: 50 }))).toBeNull();
    expect(parseYouTubePlayerMessage(delivery({ muted: false }))).toBeNull();
  });

  it("ignores a volume out of range or not a whole step", () => {
    for (const volume of [-1, 101, 50.5, Number.NaN, "50", null]) {
      expect(parseYouTubePlayerMessage(delivery({ muted: false, volume })), String(volume)).toBeNull();
    }
  });

  it("ignores anything that is not a player message", () => {
    const others: unknown[] = [
      undefined,
      null,
      42,
      { event: "onReady" },
      "not json",
      "null",
      '"onReady"',
      "[]",
      '{"event":"initialDelivery","info":{"volume":100,"muted":false}}',
      '{"event":"infoDelivery","info":null}',
      '{"event":"infoDelivery","info":"volume"}',
      `${delivery({ muted: false, volume: 50 })}${" ".repeat(70_000)}`,
    ];
    for (const data of others) {
      expect(parseYouTubePlayerMessage(data), String(data).slice(0, 40)).toBeNull();
    }
  });
});

describe("isFromYouTubePlayer", () => {
  const playerWindow = { name: "the player's window" };

  it("takes messages from this player only", () => {
    expect(
      isFromYouTubePlayer({ origin: "https://www.youtube-nocookie.com", source: playerWindow }, playerWindow),
    ).toBe(true);
  });

  it("refuses another origin, another frame, or a player without a window", () => {
    const otherWindow = { name: "another frame" };
    expect(isFromYouTubePlayer({ origin: "https://www.youtube.com", source: playerWindow }, playerWindow)).toBe(false);
    expect(isFromYouTubePlayer({ origin: "https://evil.example", source: playerWindow }, playerWindow)).toBe(false);
    expect(isFromYouTubePlayer({ origin: "https://www.youtube-nocookie.com", source: otherWindow }, playerWindow)).toBe(
      false,
    );
    expect(isFromYouTubePlayer({ origin: "https://www.youtube-nocookie.com", source: null }, null)).toBe(false);
  });
});

describe("YouTube player commands", () => {
  it("asks the player to report to this page", () => {
    expect(JSON.parse(youTubeListeningMessage())).toEqual({ event: "listening", id: 1, channel: "widget" });
  });

  it("sets the volume in whole steps, then the muted state", () => {
    expect(youTubeVolumeCommands({ volume: 0.333, isMuted: true }).map((command) => JSON.parse(command))).toEqual([
      { event: "command", func: "setVolume", args: [33], id: 1, channel: "widget" },
      { event: "command", func: "mute", args: [], id: 1, channel: "widget" },
    ]);
    expect(JSON.parse(youTubeVolumeCommands({ volume: 1, isMuted: false })[1] ?? "")).toMatchObject({ func: "unMute" });
  });
});

describe("followYouTubeVolume", () => {
  it("remembers the volume the reader sets once the player is ready, and nothing before", () => {
    const storage = memoryStorage();
    const sent: string[] = [];
    const follow = followYouTubeVolume(storage, (message) => sent.push(message));

    follow(volumeMessage(1));
    expect(readMediaVolume(storage)).toBeNull();

    follow({ kind: "ready" });
    expect(sent).toEqual([]);
    follow(volumeMessage(1));
    expect(readMediaVolume(storage)).toEqual({ volume: 1, isMuted: false });
    follow(volumeMessage(0.4));
    expect(readMediaVolume(storage)).toEqual({ volume: 0.4, isMuted: false });
  });

  it("sends the remembered volume on ready and skips the player's own starting volume", () => {
    const storage = memoryStorage();
    writeMediaVolume(storage, { volume: 0.3, isMuted: false });
    const sent: string[] = [];
    const follow = followYouTubeVolume(storage, (message) => sent.push(message));

    follow({ kind: "ready" });
    expect(sent).toEqual(youTubeVolumeCommands({ volume: 0.3, isMuted: false }));

    // The player's starting volume, reported before it took the commands, is not the reader's.
    follow(volumeMessage(1));
    expect(readMediaVolume(storage)).toEqual({ volume: 0.3, isMuted: false });

    follow(volumeMessage(0.3));
    follow(volumeMessage(0.55, true));
    expect(readMediaVolume(storage)).toEqual({ volume: 0.55, isMuted: true });
  });

  it("matches an attachment's finer volume in YouTube's whole steps", () => {
    const storage = memoryStorage();
    writeMediaVolume(storage, { volume: 0.333, isMuted: false });
    const follow = followYouTubeVolume(storage, () => {});

    follow({ kind: "ready" });
    follow(volumeMessage(0.33));
    follow(volumeMessage(0.8));
    expect(readMediaVolume(storage)).toEqual({ volume: 0.8, isMuted: false });
  });

  it("sends the commands once even if the player says it is ready again", () => {
    const storage = memoryStorage();
    writeMediaVolume(storage, { volume: 0.5, isMuted: false });
    const sent: string[] = [];
    const follow = followYouTubeVolume(storage, (message) => sent.push(message));

    follow({ kind: "ready" });
    follow({ kind: "ready" });
    expect(sent).toHaveLength(2);
  });
});

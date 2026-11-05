import { afterEach, describe, expect, it, vi } from "vitest";

import { memoryStorage } from "../test-memory-storage";
import { playAtRememberedVolume, readMediaVolume, rememberMediaVolume, writeMediaVolume } from "./media-volume";

const STORAGE_KEY = "mellow.media-volume";

afterEach(() => {
  vi.restoreAllMocks();
});

/** An attachment's player: its volume and muted state, and the event the browser fires on a change. */
class FakeMedia extends EventTarget {
  volume = 1;
  muted = false;

  setByReader(volume: number, muted: boolean): void {
    this.volume = volume;
    this.muted = muted;
    this.dispatchEvent(new Event("volumechange"));
  }
}

describe("media volume storage", () => {
  it("remembers nothing until a volume is stored", () => {
    expect(readMediaVolume(null)).toBeNull();
    expect(readMediaVolume(memoryStorage())).toBeNull();
  });

  it("reads back what was stored", () => {
    const storage = memoryStorage();
    writeMediaVolume(storage, { volume: 0.35, isMuted: true });
    expect(readMediaVolume(storage)).toEqual({ volume: 0.35, isMuted: true });
  });

  it("ignores anything that is not a volume", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const storage = memoryStorage();
    const corrupt = [
      "not json",
      "0.5",
      "null",
      "[]",
      '{"volume":1.5,"isMuted":false}',
      '{"volume":-0.1,"isMuted":false}',
      '{"volume":"0.5","isMuted":false}',
      '{"volume":0.5,"isMuted":"yes"}',
      '{"volume":0.5}',
    ];
    for (const stored of corrupt) {
      storage.setItem(STORAGE_KEY, stored);
      expect(readMediaVolume(storage), stored).toBeNull();
    }
  });

  it("keeps only the volume's own fields", () => {
    const storage = memoryStorage();
    storage.setItem(STORAGE_KEY, '{"volume":0.2,"isMuted":false,"extra":"x"}');
    expect(readMediaVolume(storage)).toEqual({ volume: 0.2, isMuted: false });
  });

  it("survives a storage that refuses to read or write", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const blocked = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
    };
    expect(readMediaVolume(blocked)).toBeNull();
    expect(() => rememberMediaVolume(blocked, { volume: 0.5, isMuted: false })).not.toThrow();
    expect(warn).toHaveBeenCalled();
  });

  it("writes only when the volume changes", () => {
    const storage = memoryStorage();
    const setItem = vi.spyOn(storage, "setItem");
    rememberMediaVolume(storage, { volume: 0.4, isMuted: false });
    rememberMediaVolume(storage, { volume: 0.4, isMuted: false });
    rememberMediaVolume(storage, { volume: 0.4, isMuted: true });
    expect(setItem).toHaveBeenCalledTimes(2);
  });
});

describe("playAtRememberedVolume", () => {
  it("leaves a player at its own volume when nothing is remembered", () => {
    const media = new FakeMedia();
    playAtRememberedVolume(media, memoryStorage());
    expect([media.volume, media.muted]).toEqual([1, false]);
  });

  it("starts a player at the remembered volume and remembers the reader's changes", () => {
    const storage = memoryStorage();
    writeMediaVolume(storage, { volume: 0.25, isMuted: false });
    const first = new FakeMedia();
    const detach = playAtRememberedVolume(first, storage);
    expect([first.volume, first.muted]).toEqual([0.25, false]);

    first.setByReader(0.6, true);
    expect(readMediaVolume(storage)).toEqual({ volume: 0.6, isMuted: true });

    const next = new FakeMedia();
    playAtRememberedVolume(next, storage);
    expect([next.volume, next.muted]).toEqual([0.6, true]);

    detach();
    first.setByReader(0.1, false);
    expect(readMediaVolume(storage)).toEqual({ volume: 0.6, isMuted: true });
  });
});

import { afterEach, describe, expect, it, vi } from "vitest";

import type { Attachment } from "../lib/protocol";
import { MAXIMUM_WAVEFORM_SOURCE_BYTES, MAXIMUM_WAVEFORM_SOURCE_SECONDS } from "./audio-player";
import { readSoundShape, rememberSoundDuration, soundShapeOf } from "./audio-waveforms.svelte";

const VOICE_NOTE_SECONDS = 14;

function sound(id: string, sizeBytes: number): Attachment {
  return { id, file_name: "note.ogg", content_type: "audio/ogg", size_bytes: sizeBytes };
}

/** An `OfflineAudioContext` whose decoding gives these samples and this length. */
function stubDecoder(samples: Float32Array, durationSeconds: number) {
  vi.stubGlobal(
    "OfflineAudioContext",
    class {
      decodeAudioData() {
        return Promise.resolve({ numberOfChannels: 1, duration: durationSeconds, getChannelData: () => samples });
      }
    },
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("rememberSoundDuration", () => {
  it("keeps a played sound's length for the tab's life", () => {
    rememberSoundDuration("701", 14.2);
    expect(soundShapeOf("701")).toEqual({ peaks: null, durationSeconds: 14.2 });
  });

  it("ignores lengths the browser does not know yet", () => {
    rememberSoundDuration("702", Number.NaN);
    rememberSoundDuration("702", Number.POSITIVE_INFINITY);
    rememberSoundDuration("702", 0);
    expect(soundShapeOf("702")).toBeNull();
  });

  it("forgets the oldest sound past 200", () => {
    for (let index = 0; index <= 200; index += 1) {
      rememberSoundDuration(`evict-${index}`, 10);
    }
    expect(soundShapeOf("evict-0")).toBeNull();
    expect(soundShapeOf("evict-1")).not.toBeNull();
    expect(soundShapeOf("evict-200")).not.toBeNull();
  });
});

describe("readSoundShape", () => {
  it("never downloads a file over the waveform limit a second time", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    await readSoundShape(sound("703", MAXIMUM_WAVEFORM_SOURCE_BYTES + 1), VOICE_NOTE_SECONDS, 48);
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(soundShapeOf("703")).toBeNull();
  });

  it("never decodes a sound longer than the limit, however small the file (silent Opus or FLAC)", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    await readSoundShape(sound("706", 1_000), MAXIMUM_WAVEFORM_SOURCE_SECONDS + 1, 48);
    await readSoundShape(sound("707", 1_000), Number.NaN, 48);
    await readSoundShape(sound("708", 1_000), Number.POSITIVE_INFINITY, 48);
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("drops the decoded samples when the file was longer than its header said", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(new ArrayBuffer(8))));
    vi.spyOn(console, "warn").mockImplementation(() => undefined);
    stubDecoder(new Float32Array([0, 0.5]), MAXIMUM_WAVEFORM_SOURCE_SECONDS + 1);
    await readSoundShape(sound("709", 1_000), VOICE_NOTE_SECONDS, 2);
    expect(soundShapeOf("709")?.peaks ?? null).toBeNull();
  });

  it("keeps the flat line when the instance refuses the file, and says so in the console", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null, { status: 503 })));
    const warning = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    rememberSoundDuration("704", 30);
    await readSoundShape(sound("704", 1_000), 30, 48);
    expect(soundShapeOf("704")).toEqual({ peaks: null, durationSeconds: 30 });
    expect(warning).toHaveBeenCalledOnce();
  });

  it("does not ask again for a sound that failed, however often it is played", async () => {
    const fetchSpy = vi.fn().mockImplementation(() => Promise.resolve(new Response(null, { status: 503 })));
    vi.stubGlobal("fetch", fetchSpy);
    vi.spyOn(console, "warn").mockImplementation(() => undefined);
    await readSoundShape(sound("710", 1_000), VOICE_NOTE_SECONDS, 48);
    await readSoundShape(sound("710", 1_000), VOICE_NOTE_SECONDS, 48);
    expect(fetchSpy).toHaveBeenCalledOnce();
  });

  it("reads a sound once while a reading is under way", async () => {
    let answer: (response: Response) => void = () => undefined;
    const fetchSpy = vi.fn().mockReturnValue(new Promise<Response>((resolve) => (answer = resolve)));
    vi.stubGlobal("fetch", fetchSpy);
    stubDecoder(new Float32Array([0, 0.5]), VOICE_NOTE_SECONDS);
    const first = readSoundShape(sound("711", 1_000), VOICE_NOTE_SECONDS, 2);
    const second = readSoundShape(sound("711", 1_000), VOICE_NOTE_SECONDS, 2);
    answer(new Response(new ArrayBuffer(8)));
    await Promise.all([first, second]);
    expect(fetchSpy).toHaveBeenCalledOnce();
    expect(soundShapeOf("711")?.peaks).toEqual([0, 1]);
  });

  it("decodes the file into one normalised level per bar", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(new ArrayBuffer(8))));
    stubDecoder(new Float32Array([0, 0.25, 0, 0.5]), 4);
    await readSoundShape(sound("705", 1_000), 4, 2);
    expect(soundShapeOf("705")).toEqual({ peaks: [0.5, 1], durationSeconds: 4 });
  });
});

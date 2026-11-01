import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { BROWSER_VOICE_PORTS } from "./voice-browser";

/** The few Web Audio and media element members playback uses. */
class FakeAudioContext {
  static created: FakeAudioContext[] = [];
  state = "running";
  isClosed = false;
  constructor() {
    FakeAudioContext.created.push(this);
  }
  createMediaStreamDestination() {
    return { stream: {} };
  }
  createMediaStreamSource() {
    return { connect: (node: unknown) => node, disconnect: () => {} };
  }
  createGain() {
    return { gain: { value: 1 }, connect: (node: unknown) => node, disconnect: () => {} };
  }
  close() {
    this.isClosed = true;
    return Promise.resolve();
  }
}

class FakeAudio {
  static created: FakeAudio[] = [];
  autoplay = false;
  muted = false;
  volume = 1;
  srcObject: unknown = null;
  constructor() {
    FakeAudio.created.push(this);
  }
  play() {
    return Promise.resolve();
  }
  pause() {}
}

describe("voice playback", () => {
  beforeEach(() => {
    FakeAudioContext.created = [];
    FakeAudio.created = [];
    vi.stubGlobal("AudioContext", FakeAudioContext);
    vi.stubGlobal("Audio", FakeAudio);
    vi.stubGlobal("MediaStream", class {});
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.stubGlobal("navigator", { languages: ["en-GB"] });
  });

  it("shares one amplification graph above 100 %, and closes it when the last voice leaves it", () => {
    const track = {} as MediaStreamTrack;
    const first = BROWSER_VOICE_PORTS.playTrack(track);
    const second = BROWSER_VOICE_PORTS.playTrack(track);
    first.setGain(2);
    second.setGain(1.5);
    expect(FakeAudioContext.created).toHaveLength(1);
    const graph = FakeAudioContext.created[0];
    // The element playing the graph: created after the two voices' own elements.
    const graphElement = FakeAudio.created[2];
    first.setGain(1);
    expect(graph?.isClosed).toBe(false);
    second.stop();
    expect(graph?.isClosed).toBe(true);
    expect(graphElement?.srcObject).toBeNull();
    // The next voice above 100 % gets a fresh graph.
    first.setGain(3);
    expect(FakeAudioContext.created).toHaveLength(2);
    first.stop();
    expect(FakeAudioContext.created[1]?.isClosed).toBe(true);
  });

  it("keeps voices at or under 100 % on their own element, without Web Audio", () => {
    const output = BROWSER_VOICE_PORTS.playTrack({} as MediaStreamTrack);
    output.setGain(0.4);
    output.setGain(0);
    expect(FakeAudioContext.created).toHaveLength(0);
    expect(FakeAudio.created[0]).toMatchObject({ muted: true, volume: 0 });
    output.stop();
  });
});

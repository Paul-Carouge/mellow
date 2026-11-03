import { describe, expect, it } from "vitest";

import {
  isLinuxDesktop,
  isScreenShareQuality,
  preferredCodecs,
  readScreenSoundDeviceId,
  screenCaptureConstraints,
  screenCaptureOptions,
  screenEncodings,
  screenShareProblemOf,
  screenSoundDevicesOf,
  shownScreenSoundDeviceId,
  soundlessShareAdvice,
  soundSourceConstraints,
  streamSoundStatus,
  withActiveLayers,
  withQuality,
} from "./screen-share";

describe("the share's sound", () => {
  it("lists audio inputs with names, monitors and stereo mixes first, without the system's duplicates", () => {
    const devices = screenSoundDevicesOf([
      { kind: "audioinput", deviceId: "default", label: "Default - Built-in Microphone" },
      { kind: "audioinput", deviceId: "communications", label: "Communications - Headset" },
      { kind: "audioinput", deviceId: "mic", label: "Built-in Microphone" },
      { kind: "audioinput", deviceId: "monitor", label: "Monitor of Built-in Audio Analog Stereo" },
      { kind: "audioinput", deviceId: "share", label: "Moniteur de Partage-Mellow" },
      { kind: "audioinput", deviceId: "mix", label: "Mixage stéréo (Realtek(R) Audio)" },
      { kind: "audioinput", deviceId: "monitoring-mic", label: "Monitoring Headset" },
      { kind: "audiooutput", deviceId: "speakers", label: "Speakers" },
    ]);
    expect(devices.map((device) => [device.deviceId, device.isSystemSound])).toEqual([
      ["monitor", true],
      ["share", true],
      ["mix", true],
      ["mic", false],
      ["monitoring-mic", false],
    ]);
  });

  it("lists nothing while the browser hides the inputs' names", () => {
    expect(screenSoundDevicesOf([{ kind: "audioinput", deviceId: "", label: "" }])).toEqual([]);
  });

  it("reads the remembered input, ignoring empty or implausible values", () => {
    const storage = (value: string | null) => ({ getItem: () => value });
    expect(readScreenSoundDeviceId(storage("abc"))).toBe("abc");
    expect(readScreenSoundDeviceId(storage(""))).toBeNull();
    expect(readScreenSoundDeviceId(storage("x".repeat(513)))).toBeNull();
    expect(readScreenSoundDeviceId(null)).toBeNull();
  });

  it("shows the remembered input checked only while it is plugged in, or while the list is unknown", () => {
    const listed = [{ deviceId: "monitor", label: "Monitor of Speakers", isSystemSound: true }];
    expect(shownScreenSoundDeviceId(listed, "monitor")).toBe("monitor");
    expect(shownScreenSoundDeviceId(listed, "gone")).toBeNull();
    expect(shownScreenSoundDeviceId([], "gone")).toBe("gone");
    expect(shownScreenSoundDeviceId(listed, null)).toBeNull();
  });

  it("opens an input exactly, without the microphone's processing", () => {
    expect(soundSourceConstraints("monitor")).toEqual({
      audio: { deviceId: { exact: "monitor" }, echoCancellation: false, noiseSuppression: false, autoGainControl: false },
      video: false,
    });
  });

  it("advises by what the browser's picker can share", () => {
    expect(soundlessShareAdvice("Windows")).toBe("tab-or-screen");
    expect(soundlessShareAdvice("Chrome OS")).toBe("tab-or-screen");
    expect(soundlessShareAdvice("Linux")).toBe("tab");
    expect(soundlessShareAdvice("macOS")).toBe("tab");
    // Firefox and Safari have no userAgentData, and share no sound from their picker.
    expect(soundlessShareAdvice(null)).toBe("sound-source");
  });

  it("recognises Linux desktops, not Android", () => {
    expect(isLinuxDesktop("Linux")).toBe(true);
    expect(isLinuxDesktop("Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0")).toBe(true);
    expect(isLinuxDesktop("Mozilla/5.0 (Linux; Android 14; Pixel 8)")).toBe(false);
    expect(isLinuxDesktop("macOS")).toBe(false);
  });
});

describe("streamSoundStatus", () => {
  it("is unknown until the picture has been there a moment, then silent without any sound", () => {
    expect(streamSoundStatus(null, Number.NEGATIVE_INFINITY, 10_000)).toBe("listening");
    expect(streamSoundStatus(8_000, Number.NEGATIVE_INFINITY, 10_000)).toBe("listening");
    expect(streamSoundStatus(5_000, Number.NEGATIVE_INFINITY, 10_000)).toBe("silent");
  });

  it("has sound while heard within the last 5 s, before the picture too", () => {
    expect(streamSoundStatus(null, 9_000, 10_000)).toBe("sound");
    expect(streamSoundStatus(0, 5_001, 10_000)).toBe("sound");
    expect(streamSoundStatus(0, 5_000, 10_000)).toBe("silent");
  });
});

const SCREEN_ENCODINGS = screenEncodings("1080p");

describe("screen share layers", () => {
  it("keeps only the watched layers on, and leaves them be when the list is empty", () => {
    const active = withActiveLayers(SCREEN_ENCODINGS, ["q", "h"]);
    expect(active.map((encoding) => [encoding.rid, encoding.active])).toEqual([
      ["q", true],
      ["l", false],
      ["h", true],
    ]);
    expect(withActiveLayers(active, [])).toEqual(active);
  });

  it("sends the RIDs the instance knows, within its per-layer budgets", () => {
    expect(SCREEN_ENCODINGS.map((encoding) => encoding.rid)).toEqual(["q", "l", "h"]);
    // The instance allows 37.5 kB/s on q, 125 kB/s on l and 1.25 MB/s on h (ADR-0035, ADR-0037).
    expect(SCREEN_ENCODINGS.map((encoding) => encoding.maxBitrate)).toEqual([150_000, 500_000, 5_000_000]);
    expect(SCREEN_ENCODINGS.every((encoding) => encoding.maxFramerate === 30)).toBe(true);
  });

  it("captures and sends each quality at 30 frames per second, and changes it live keeping paused layers", () => {
    expect(screenCaptureConstraints("720p")).toEqual({
      width: { max: 1_280 },
      height: { max: 720 },
      frameRate: { ideal: 30, max: 30 },
    });
    expect(screenCaptureConstraints("1440p").width).toEqual({ max: 2_560 });
    expect(screenEncodings("1440p").at(-1)?.maxBitrate).toBe(8_000_000);
    const paused = withActiveLayers(screenEncodings("720p"), ["q"]);
    const upgraded = withQuality(paused, "1440p");
    expect(upgraded.map((encoding) => [encoding.active, encoding.maxBitrate])).toEqual([
      [true, 150_000],
      [false, 500_000],
      [false, 8_000_000],
    ]);
    expect(isScreenShareQuality("1440p")).toBe(true);
    expect(isScreenShareQuality("4k")).toBe(false);
  });
});

describe("screen share capture", () => {
  it("asks for the shared sound untouched, and never offers the call's own tab", () => {
    const options = screenCaptureOptions("1080p");
    expect(options.video).toEqual(screenCaptureConstraints("1080p"));
    expect(options.audio).toEqual({ echoCancellation: false, noiseSuppression: false, autoGainControl: false, restrictOwnAudio: true });
    expect(options.systemAudio).toBe("include");
    expect(options.selfBrowserSurface).toBe("exclude");
  });
});

describe("screen share problems and codecs", () => {
  it("tells a cancelled picker from a browser that cannot share", () => {
    expect(screenShareProblemOf(new DOMException("no", "NotAllowedError"))).toBe("denied");
    expect(screenShareProblemOf(new TypeError("no getDisplayMedia"))).toBe("unsupported");
  });

  it("keeps Opus, VP8 and its retransmissions, or nothing when one is missing", () => {
    const codec = (mimeType: string) => ({ mimeType, clockRate: 90_000 });
    const audio = [codec("audio/opus"), codec("audio/PCMU")];
    const video = [codec("video/H264"), codec("video/VP8"), codec("video/rtx"), codec("video/AV1")];
    expect(preferredCodecs(audio, video)).toEqual({
      audio: [codec("audio/opus")],
      video: [codec("video/VP8"), codec("video/rtx")],
    });
    expect(preferredCodecs(audio, [codec("video/H264")])).toBeNull();
  });
});

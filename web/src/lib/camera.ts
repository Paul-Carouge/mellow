// What a camera is sent as (ADR-0035): 720p at 30 frames per second in three simulcast layers,
// on the same line and with the same RIDs as a shared screen (ADR-0037), at the bit rates of the
// instance's layer targets.

import type { VoiceVideoRefusal } from "./protocol";

export const CAMERA_DEVICE_STORAGE_KEY = "mellow.camera-device";
/** Device ids are opaque, but a stored value longer than this is not one. */
const MAXIMUM_STORED_DEVICE_ID_LENGTH = 512;
const CAMERA_WIDTH = 1_280;
const CAMERA_HEIGHT = 720;
const CAMERA_FRAMES_PER_SECOND = 30;

/** A camera the browser named (it names them once one was allowed). */
export interface CameraDevice {
  readonly deviceId: string;
  readonly label: string;
}

/**
 * The remembered camera if it is still plugged in (`ideal`: the browser falls back to another one
 * instead of failing), at 720p and 30 frames per second, without sound.
 */
export function cameraConstraints(deviceId: string | null): MediaStreamConstraints {
  return {
    video: {
      ...(deviceId === null ? {} : { deviceId: { ideal: deviceId } }),
      width: { ideal: CAMERA_WIDTH },
      height: { ideal: CAMERA_HEIGHT },
      frameRate: { ideal: CAMERA_FRAMES_PER_SECOND, max: CAMERA_FRAMES_PER_SECOND },
    },
    audio: false,
  };
}

/** Lowest first, as browsers list simulcast layers: 180p, 360p and 720p at the `q`, `l` and `h` targets. */
export function cameraEncodings(): RTCRtpEncodingParameters[] {
  const frames = CAMERA_FRAMES_PER_SECOND;
  return [
    { rid: "q", scaleResolutionDownBy: 4, maxBitrate: 150_000, maxFramerate: frames },
    { rid: "l", scaleResolutionDownBy: 2, maxBitrate: 500_000, maxFramerate: frames },
    { rid: "h", scaleResolutionDownBy: 1, maxBitrate: 1_500_000, maxFramerate: frames },
  ];
}

/** The live encodings with the bit rates and frame rates of `wanted`, keeping which layers are paused. */
export function withTargetsOf(
  encodings: readonly RTCRtpEncodingParameters[],
  wanted: readonly RTCRtpEncodingParameters[],
): RTCRtpEncodingParameters[] {
  return encodings.map((encoding) => {
    const target = wanted.find((candidate) => candidate.rid === encoding.rid);
    return target === undefined ? encoding : { ...encoding, maxBitrate: target.maxBitrate, maxFramerate: target.maxFramerate };
  });
}

export function camerasOf(devices: readonly { readonly kind: string; readonly deviceId: string; readonly label: string }[]): CameraDevice[] {
  return devices
    .filter((device) => device.kind === "videoinput" && device.deviceId !== "" && device.label !== "")
    .map((device) => ({ deviceId: device.deviceId, label: device.label }));
}

export function readCameraDeviceId(storage: Pick<Storage, "getItem"> | null): string | null {
  const stored = storage?.getItem(CAMERA_DEVICE_STORAGE_KEY) ?? null;
  return stored !== null && stored.length > 0 && stored.length <= MAXIMUM_STORED_DEVICE_ID_LENGTH ? stored : null;
}

export type CameraProblem =
  /** The person or the browser refused the camera. */
  | "denied"
  /** No camera is plugged in. */
  | "not-found"
  /** Another application holds the camera. */
  | "in-use"
  /** This browser cannot send a camera into a call. */
  | "unsupported"
  /** The camera was unplugged, or the system took it away, while on. */
  | "ended"
  /** The instance stopped relaying it (permission taken away, or nothing sent for 10 s). */
  | "stopped"
  | VoiceVideoRefusal;

/** `getUserMedia`'s rejections, by the names browsers give them. */
export function cameraProblemOf(error: unknown): CameraProblem {
  const name = error instanceof Error || error instanceof DOMException ? error.name : "";
  switch (name) {
    case "NotAllowedError":
    case "SecurityError":
      return "denied";
    case "NotFoundError":
    case "OverconstrainedError":
      return "not-found";
    case "NotReadableError":
    case "AbortError":
      return "in-use";
    default:
      return "unsupported";
  }
}

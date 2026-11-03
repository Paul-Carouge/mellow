import { describe, expect, it } from "vitest";

import { cameraConstraints, cameraProblemOf, camerasOf, readCameraDeviceId } from "./camera";

describe("camera", () => {
  it("tells refused, missing and busy cameras apart", () => {
    const named = (name: string) => Object.assign(new Error(name), { name });
    expect(cameraProblemOf(named("NotAllowedError"))).toBe("denied");
    expect(cameraProblemOf(named("NotFoundError"))).toBe("not-found");
    expect(cameraProblemOf(named("NotReadableError"))).toBe("in-use");
    expect(cameraProblemOf(new TypeError("no mediaDevices"))).toBe("unsupported");
  });

  it("lists only named cameras", () => {
    expect(
      camerasOf([
        { kind: "audioinput", deviceId: "mic", label: "Micro" },
        { kind: "videoinput", deviceId: "", label: "" },
        { kind: "videoinput", deviceId: "cam", label: "FaceTime HD Camera" },
      ]),
    ).toEqual([{ deviceId: "cam", label: "FaceTime HD Camera" }]);
  });

  it("asks for the remembered camera without failing when it is gone", () => {
    expect(cameraConstraints("cam").video).toMatchObject({ deviceId: { ideal: "cam" }, width: { ideal: 1_280 } });
    expect(cameraConstraints(null).video).not.toHaveProperty("deviceId");
  });

  it("reads back an empty or oversized stored camera as the default", () => {
    const storageOf = (value: string | null) => ({ getItem: () => value });
    expect(readCameraDeviceId(storageOf("cam"))).toBe("cam");
    expect(readCameraDeviceId(storageOf(""))).toBeNull();
    expect(readCameraDeviceId(storageOf("x".repeat(513)))).toBeNull();
  });
});

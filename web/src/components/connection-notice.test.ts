import { describe, expect, it } from "vitest";

import { connectionTrouble, isComingBackOnline, troubleToRemember } from "./connection-notice";

describe("connectionTrouble", () => {
  it("says nothing while connected and online", () => {
    expect(connectionTrouble("connected", true, false, false)).toBeNull();
  });

  it("says connecting while the gateway checks its socket after the network came back", () => {
    expect(connectionTrouble("connected", true, false, true)).toBe("checking");
    expect(connectionTrouble("connected", false, false, true)).toBe("offline");
  });

  it("puts the device being offline before what the gateway is doing", () => {
    expect(connectionTrouble("connected", false, false, false)).toBe("offline");
    expect(connectionTrouble("reconnecting", false, true, false)).toBe("offline");
    expect(connectionTrouble("connecting", false, false, false)).toBe("offline");
  });

  it("offers more after reconnecting for long", () => {
    expect(connectionTrouble("reconnecting", true, false, false)).toBe("reconnecting");
    expect(connectionTrouble("reconnecting", true, true, false)).toBe("still-reconnecting");
  });

  it("says connecting during the first connection", () => {
    expect(connectionTrouble("connecting", true, false, false)).toBe("connecting");
  });

  it("says nothing outside the shell", () => {
    expect(connectionTrouble("signed-out", false, false, false)).toBeNull();
    expect(connectionTrouble("unreachable", true, false, false)).toBeNull();
  });
});

describe("isComingBackOnline", () => {
  it("follows the end of an offline or reconnecting spell", () => {
    expect(isComingBackOnline("offline", null)).toBe(true);
    expect(isComingBackOnline("reconnecting", null)).toBe(true);
    expect(isComingBackOnline("still-reconnecting", null)).toBe(true);
  });

  it("stays quiet after the first connection and while trouble goes on", () => {
    expect(isComingBackOnline("connecting", null)).toBe(false);
    expect(isComingBackOnline(null, null)).toBe(false);
    expect(isComingBackOnline("offline", "reconnecting")).toBe(false);
  });
});

describe("troubleToRemember", () => {
  it("keeps what came before a socket check, so back online follows only a real loss", () => {
    // Offline, then the browser says online while the gateway checks its socket, then it answers.
    let remembered = troubleToRemember(null, "offline");
    remembered = troubleToRemember(remembered, "checking");
    expect(isComingBackOnline(remembered, "checking")).toBe(false);
    expect(isComingBackOnline(remembered, null)).toBe(true);

    // A network change without a loss: the check passes quietly.
    expect(isComingBackOnline(troubleToRemember(null, "checking"), null)).toBe(false);
  });

  it("forgets the check once the socket is found dead", () => {
    expect(troubleToRemember("offline", "reconnecting")).toBe("reconnecting");
  });
});

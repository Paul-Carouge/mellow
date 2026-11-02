import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GatewayConnection } from "./gateway";

/** Records every socket opened, so a test can close one as the network would. */
class FakeWebSocket extends EventTarget {
  static readonly OPEN = 1;
  static opened: FakeWebSocket[] = [];
  readyState = 0;

  constructor(readonly url: string) {
    super();
    FakeWebSocket.opened.push(this);
  }

  send(): void {}

  close(): void {}

  dropByNetwork(): void {
    this.dispatchEvent(new Event("close"));
  }
}

describe("GatewayConnection", () => {
  let fakeWindow: EventTarget;

  beforeEach(() => {
    vi.useFakeTimers();
    FakeWebSocket.opened = [];
    fakeWindow = new EventTarget();
    vi.stubGlobal("WebSocket", FakeWebSocket);
    vi.stubGlobal("window", fakeWindow);
    vi.stubGlobal("location", { protocol: "https:", host: "mellow.example" });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  /** A gateway that lost its first socket after `ready` and waits out its backoff. */
  function gatewayWaitingToReconnect(): GatewayConnection {
    const gateway = new GatewayConnection("", () => {}, () => Promise.resolve(false));
    gateway.open();
    FakeWebSocket.opened[0]!.dropByNetwork();
    expect(FakeWebSocket.opened).toHaveLength(1);
    return gateway;
  }

  it("reconnects at once when the network comes back instead of waiting out the backoff", () => {
    gatewayWaitingToReconnect();

    fakeWindow.dispatchEvent(new Event("online"));

    expect(FakeWebSocket.opened).toHaveLength(2);
    // The cancelled backoff timer does not open a third socket later.
    vi.runOnlyPendingTimers();
    expect(FakeWebSocket.opened).toHaveLength(2);
  });

  it("ignores the network coming back while a socket is already open", () => {
    const gateway = new GatewayConnection("", () => {}, () => Promise.resolve(false));
    gateway.open();

    fakeWindow.dispatchEvent(new Event("online"));

    expect(FakeWebSocket.opened).toHaveLength(1);
  });

  it("ignores the network coming back once the backoff timer has already reopened", () => {
    gatewayWaitingToReconnect();
    vi.advanceTimersToNextTimer();
    expect(FakeWebSocket.opened).toHaveLength(2);

    fakeWindow.dispatchEvent(new Event("online"));

    expect(FakeWebSocket.opened).toHaveLength(2);
  });

  it("stops listening for the network once closed", () => {
    const gateway = gatewayWaitingToReconnect();
    gateway.close();

    fakeWindow.dispatchEvent(new Event("online"));

    expect(FakeWebSocket.opened).toHaveLength(1);
  });
});

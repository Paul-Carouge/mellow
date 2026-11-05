import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GatewayConnection, NETWORK_CHANGE_HEARTBEAT_ACKNOWLEDGEMENT_TIMEOUT_MILLISECONDS } from "./gateway";

const HEARTBEAT_INTERVAL_MILLISECONDS = 30_000;

/** Records every socket opened, so a test can close one as the network would. */
class FakeWebSocket extends EventTarget {
  static readonly OPEN = 1;
  static opened: FakeWebSocket[] = [];
  readyState = 0;
  readonly sentFrames: unknown[] = [];

  constructor(readonly url: string) {
    super();
    FakeWebSocket.opened.push(this);
  }

  send(frame: string): void {
    this.sentFrames.push(JSON.parse(frame));
  }

  close(): void {}

  /** Opens the socket and delivers a server frame, as the instance would. */
  receive(frame: object): void {
    this.readyState = FakeWebSocket.OPEN;
    this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(frame) }));
  }

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

  /** A gateway attached to a session on its first socket: `hello` then `ready` came back. */
  function attachedGateway(): GatewayConnection {
    const gateway = new GatewayConnection("", () => {}, () => Promise.resolve(false));
    gateway.open();
    const socket = FakeWebSocket.opened[0]!;
    socket.receive({ op: "hello", d: { heartbeat_interval_milliseconds: HEARTBEAT_INTERVAL_MILLISECONDS } });
    socket.receive({ op: "ready", d: { session_id: "session" } });
    return gateway;
  }

  it("resumes on a new socket when an open one does not answer a heartbeat after a network change", () => {
    attachedGateway();
    const firstSocket = FakeWebSocket.opened[0]!;

    fakeWindow.dispatchEvent(new Event("online"));

    // The check goes out at once, without waiting for the next regular heartbeat.
    expect(firstSocket.sentFrames.at(-1)).toEqual({ op: "heartbeat" });
    expect(FakeWebSocket.opened).toHaveLength(1);
    vi.advanceTimersByTime(NETWORK_CHANGE_HEARTBEAT_ACKNOWLEDGEMENT_TIMEOUT_MILLISECONDS);
    vi.advanceTimersToNextTimer();
    expect(FakeWebSocket.opened).toHaveLength(2);
    const secondSocket = FakeWebSocket.opened[1]!;
    secondSocket.receive({ op: "hello", d: { heartbeat_interval_milliseconds: HEARTBEAT_INTERVAL_MILLISECONDS } });
    expect(secondSocket.sentFrames).toEqual([
      { op: "resume", d: { token: "", session_id: "session", last_sequence: 0 } },
    ]);
  });

  it("keeps an open socket that answers the heartbeat sent after a network change", () => {
    attachedGateway();
    const socket = FakeWebSocket.opened[0]!;

    fakeWindow.dispatchEvent(new Event("online"));
    socket.receive({ op: "heartbeat_ack" });
    vi.advanceTimersByTime(NETWORK_CHANGE_HEARTBEAT_ACKNOWLEDGEMENT_TIMEOUT_MILLISECONDS + 1);

    expect(FakeWebSocket.opened).toHaveLength(1);
  });

  it("stops listening for the network once closed", () => {
    const gateway = gatewayWaitingToReconnect();
    gateway.close();

    fakeWindow.dispatchEvent(new Event("online"));

    expect(FakeWebSocket.opened).toHaveLength(1);
  });
});

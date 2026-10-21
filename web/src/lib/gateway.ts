// WebSocket gateway at `/gateway?v=1` (ARCHITECTURE.md §4.8): identify, heartbeats, resume
// after a dropped connection, and backoff while the instance is unreachable.

import type { ClientFrame, GatewayEvent, Ready, ServerFrame } from "./protocol";

const GATEWAY_PATH = "/gateway?v=1";
const FIRST_RECONNECT_DELAY_MILLISECONDS = 1_000;
const MAXIMUM_RECONNECT_DELAY_MILLISECONDS = 30_000;

export type GatewayUpdate =
  | { kind: "ready"; ready: Ready }
  | { kind: "event"; event: GatewayEvent }
  | { kind: "resumed" }
  | { kind: "disconnected" }
  | { kind: "signed-out" };

interface ResumableSession {
  sessionId: string;
  lastSequence: number;
}

export class GatewayConnection {
  private socket: WebSocket | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectDelay = FIRST_RECONNECT_DELAY_MILLISECONDS;
  private resumable: ResumableSession | null = null;
  private isClosedByUs = false;
  /** An `identify` was sent on this connection and no `ready` came back yet. */
  private isAwaitingReady = false;

  /**
   * `isTokenRefused` asks the REST API whether the token still works, as the native client does:
   * a connection closed after `identify` may mean a refused token or a full session table.
   */
  /** The token is empty for the web client: the instance reads the session cookie instead. */
  constructor(
    private readonly token: string,
    private readonly onUpdate: (update: GatewayUpdate) => void,
    private readonly isTokenRefused: () => Promise<boolean>,
  ) {}

  open(): void {
    this.isClosedByUs = false;
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    const socket = new WebSocket(`${scheme}//${location.host}${GATEWAY_PATH}`);
    this.socket = socket;
    socket.addEventListener("message", (message) => this.receive(message.data));
    socket.addEventListener("close", () => this.closed());
  }

  close(): void {
    this.isClosedByUs = true;
    this.stopHeartbeat();
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.socket?.close();
    this.socket = null;
  }

  private send(frame: ClientFrame): void {
    if (this.socket?.readyState === WebSocket.OPEN) {
      this.socket.send(JSON.stringify(frame));
    }
  }

  private receive(data: unknown): void {
    if (typeof data !== "string") {
      return;
    }
    let frame: ServerFrame;
    try {
      frame = JSON.parse(data) as ServerFrame;
    } catch {
      // A frame this client cannot read is skipped, as the native clients do.
      console.warn("skipping an unreadable gateway frame");
      return;
    }
    switch (frame.op) {
      case "hello":
        this.startHeartbeat(frame.d.heartbeat_interval_milliseconds);
        if (this.resumable) {
          this.send({
            op: "resume",
            d: {
              token: this.token,
              session_id: this.resumable.sessionId,
              last_sequence: this.resumable.lastSequence,
            },
          });
        } else {
          this.isAwaitingReady = true;
          this.send({ op: "identify", d: { token: this.token } });
        }
        break;
      case "ready":
        this.isAwaitingReady = false;
        this.reconnectDelay = FIRST_RECONNECT_DELAY_MILLISECONDS;
        this.resumable = { sessionId: frame.d.session_id, lastSequence: 0 };
        this.onUpdate({ kind: "ready", ready: frame.d });
        break;
      case "resumed":
        this.reconnectDelay = FIRST_RECONNECT_DELAY_MILLISECONDS;
        this.onUpdate({ kind: "resumed" });
        break;
      case "invalid_session":
        // Identify again on the next connection; `ready` then replaces the whole state.
        this.resumable = null;
        this.socket?.close();
        break;
      case "heartbeat_ack":
        break;
      case "dispatch":
        if (this.resumable) {
          this.resumable.lastSequence = frame.s;
        }
        this.onUpdate({ kind: "event", event: frame.d });
        break;
      default: {
        const unknown: never = frame;
        console.warn("skipping an unknown gateway frame", unknown);
      }
    }
  }

  private closed(): void {
    this.stopHeartbeat();
    this.socket = null;
    if (this.isClosedByUs) {
      return;
    }
    const wasIdentifying = this.isAwaitingReady;
    this.isAwaitingReady = false;
    if (wasIdentifying) {
      void this.isTokenRefused().then((isRefused) => {
        if (isRefused) {
          this.onUpdate({ kind: "signed-out" });
        } else if (!this.isClosedByUs) {
          this.scheduleReconnect();
        }
      });
      return;
    }
    this.onUpdate({ kind: "disconnected" });
    this.scheduleReconnect();
  }

  private scheduleReconnect(): void {
    this.reconnectTimer = setTimeout(() => this.open(), this.reconnectDelay);
    this.reconnectDelay = Math.min(this.reconnectDelay * 2, MAXIMUM_RECONNECT_DELAY_MILLISECONDS);
  }

  private startHeartbeat(intervalMilliseconds: number): void {
    this.stopHeartbeat();
    this.heartbeatTimer = setInterval(() => this.send({ op: "heartbeat" }), intervalMilliseconds);
  }

  private stopHeartbeat(): void {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }
}

// WebSocket gateway at `/gateway?v=1` (ARCHITECTURE.md §4.8): identify, heartbeats, resume
// after a dropped connection, and backoff while the instance is unreachable.

import type { ClientFrame, GatewayEvent, Ready, ServerFrame } from "./protocol";

const GATEWAY_PATH = "/gateway?v=1";
const FIRST_RECONNECT_DELAY_MILLISECONDS = 1_000;
const MAXIMUM_RECONNECT_DELAY_MILLISECONDS = 30_000;
/** `hello` then `ready` or `resumed` must arrive within this, as `HANDSHAKE_TIMEOUT` in the core. */
const HANDSHAKE_TIMEOUT_MILLISECONDS = 15_000;

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
  /** Detaches the current socket's listeners when it is abandoned. */
  private socketListeners: AbortController | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private handshakeTimer: ReturnType<typeof setTimeout> | null = null;
  /** A link that went dead (sleep, Wi-Fi change) can stay "open" for minutes: an unanswered
   * heartbeat ends it, as in the core. */
  private isHeartbeatAcknowledged = true;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectDelay = FIRST_RECONNECT_DELAY_MILLISECONDS;
  private resumable: ResumableSession | null = null;
  private isClosedByUs = false;
  /** An `identify` was sent on this connection and no `ready` came back yet. */
  private isAwaitingReady = false;

  /**
   * `isTokenRefused` asks the REST API whether the token still works, as the native client does:
   * a connection closed after `identify` may mean a refused token or a full session table. The
   * token is empty for the web client: the instance reads the session cookie instead.
   */
  constructor(
    private readonly token: string,
    private readonly onUpdate: (update: GatewayUpdate) => void,
    private readonly isTokenRefused: () => Promise<boolean>,
  ) {}

  open(): void {
    this.isClosedByUs = false;
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    const socket = new WebSocket(`${scheme}//${location.host}${GATEWAY_PATH}`);
    const listeners = new AbortController();
    this.socket = socket;
    this.socketListeners = listeners;
    socket.addEventListener("message", (message) => this.receive(message.data), { signal: listeners.signal });
    socket.addEventListener("close", () => this.closed(), { signal: listeners.signal });
    this.handshakeTimer = setTimeout(() => this.abandonSocket(), HANDSHAKE_TIMEOUT_MILLISECONDS);
  }

  /** Ends a connection that stopped answering without waiting for a `close` that may never come. */
  private abandonSocket(): void {
    this.socketListeners?.abort();
    this.socket?.close();
    this.closed();
  }

  close(): void {
    this.isClosedByUs = true;
    this.stopTimers();
    this.socketListeners?.abort();
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.socket?.close();
    this.socket = null;
  }

  /** Whether the frame left: nothing is queued while the link is down (voice rejoins after it). */
  send(frame: ClientFrame): boolean {
    if (this.socket?.readyState !== WebSocket.OPEN) {
      return false;
    }
    this.socket.send(JSON.stringify(frame));
    return true;
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
        this.stopHandshakeTimer();
        this.isAwaitingReady = false;
        this.reconnectDelay = FIRST_RECONNECT_DELAY_MILLISECONDS;
        this.resumable = { sessionId: frame.d.session_id, lastSequence: 0 };
        this.onUpdate({ kind: "ready", ready: frame.d });
        break;
      case "resumed":
        this.stopHandshakeTimer();
        this.reconnectDelay = FIRST_RECONNECT_DELAY_MILLISECONDS;
        this.onUpdate({ kind: "resumed" });
        break;
      case "invalid_session":
        // Identify again on the next connection; `ready` then replaces the whole state.
        this.resumable = null;
        this.socket?.close();
        break;
      case "heartbeat_ack":
        this.isHeartbeatAcknowledged = true;
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
    this.stopTimers();
    this.socket = null;
    this.socketListeners = null;
    if (this.isClosedByUs) {
      return;
    }
    const wasIdentifying = this.isAwaitingReady;
    this.isAwaitingReady = false;
    if (wasIdentifying) {
      void this.isTokenRefused().then((isRefused) => {
        // Closed meanwhile (signed out, another session started): this connection is over.
        if (this.isClosedByUs) {
          return;
        }
        if (isRefused) {
          this.onUpdate({ kind: "signed-out" });
        } else {
          this.scheduleReconnect();
        }
      });
      return;
    }
    this.onUpdate({ kind: "disconnected" });
    this.scheduleReconnect();
  }

  /** Between half and all of the backoff delay, so every tab does not come back at once after the
   * instance restarts, as the core's `reconnect_delay` does. */
  private scheduleReconnect(): void {
    const delay = this.reconnectDelay * (0.5 + Math.random() / 2);
    this.reconnectTimer = setTimeout(() => this.open(), delay);
    this.reconnectDelay = Math.min(this.reconnectDelay * 2, MAXIMUM_RECONNECT_DELAY_MILLISECONDS);
  }

  private startHeartbeat(intervalMilliseconds: number): void {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
    }
    this.isHeartbeatAcknowledged = true;
    this.heartbeatTimer = setInterval(() => {
      if (!this.isHeartbeatAcknowledged) {
        this.abandonSocket();
        return;
      }
      this.isHeartbeatAcknowledged = false;
      this.send({ op: "heartbeat" });
    }, intervalMilliseconds);
  }

  private stopHandshakeTimer(): void {
    if (this.handshakeTimer) {
      clearTimeout(this.handshakeTimer);
      this.handshakeTimer = null;
    }
  }

  private stopTimers(): void {
    this.stopHandshakeTimer();
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }
}

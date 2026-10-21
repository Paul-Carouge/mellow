// REST client for `/api/v1` on the origin that served this page (ADR-0019). The session is an
// HttpOnly cookie the instance sets at sign-in: page scripts never see the token.

import type {
  ApiErrorCode,
  Channel,
  CurrentUser,
  DirectMessageChannel,
  Guild,
  Invite,
  Member,
  Message,
  SessionCreated,
  Snowflake,
} from "./protocol";

export const HISTORY_PAGE_SIZE = 50;
const DEVICE_NAME = "Web browser";
const SESSION_TRANSPORT_HEADER = "X-Mellow-Session-Transport";

/** A refusal from the instance. Its `message` is the instance's own text: never shown as is. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: ApiErrorCode | "NETWORK",
  ) {
    super(`request failed: ${code}`);
    this.name = "ApiError";
  }

  get needsNewLogin(): boolean {
    return this.code === "UNAUTHORIZED";
  }
}

export class RestClient {
  private async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = { [SESSION_TRANSPORT_HEADER]: "cookie" };
    if (body !== undefined) {
      headers["Content-Type"] = "application/json";
    }
    let response: Response;
    try {
      response = await fetch(`/api/v1/${path}`, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        credentials: "same-origin",
      });
    } catch {
      throw new ApiError(0, "NETWORK");
    }
    if (!response.ok) {
      const code = await response
        .json()
        .then((error: { code?: ApiErrorCode }) => error.code ?? "INTERNAL")
        .catch(() => "INTERNAL" as const);
      throw new ApiError(response.status, code);
    }
    if (response.status === 204) {
      return undefined as T;
    }
    return (await response.json()) as T;
  }

  login(login: string, password: string): Promise<SessionCreated> {
    return this.request<SessionCreated>("POST", "auth/login", {
      login,
      password,
      device_name: DEVICE_NAME,
    });
  }

  register(username: string, email: string, password: string): Promise<SessionCreated> {
    return this.request<SessionCreated>("POST", "auth/register", {
      username,
      email,
      password,
      device_name: DEVICE_NAME,
    });
  }

  logout(): Promise<void> {
    return this.request<void>("POST", "auth/logout");
  }

  currentUser(): Promise<CurrentUser> {
    return this.request("GET", "users/@me");
  }

  createGuild(name: string): Promise<Guild> {
    return this.request("POST", "guilds", { name });
  }

  joinWithInvite(code: string): Promise<Guild> {
    // The code becomes a path segment: refuse anything that is not a plain code.
    if (!/^[A-Za-z0-9]+$/.test(code)) {
      return Promise.reject(new ApiError(404, "INVALID_INVITE"));
    }
    return this.request("POST", `invites/${code}`);
  }

  createInvite(guildId: Snowflake): Promise<Invite> {
    return this.request("POST", `guilds/${guildId}/invites`, {
      max_uses: null,
      expires_in_seconds: null,
    });
  }

  createTextChannel(guildId: Snowflake, name: string): Promise<Channel> {
    return this.request("POST", `guilds/${guildId}/channels`, {
      kind: "text",
      name,
      topic: "",
      parent_id: null,
    });
  }

  leaveGuild(guildId: Snowflake): Promise<void> {
    return this.request("DELETE", `guilds/${guildId}/members/@me`);
  }

  listMembers(guildId: Snowflake): Promise<Member[]> {
    return this.request("GET", `guilds/${guildId}/members`);
  }

  listMessages(channelId: Snowflake, before?: Snowflake): Promise<Message[]> {
    const query = new URLSearchParams({ limit: String(HISTORY_PAGE_SIZE) });
    if (before) {
      query.set("before", before);
    }
    return this.request("GET", `channels/${channelId}/messages?${query}`);
  }

  sendMessage(
    channelId: Snowflake,
    content: string,
    nonce: string,
    replyToId: Snowflake | null = null,
  ): Promise<Message> {
    return this.request("POST", `channels/${channelId}/messages`, {
      content,
      reply_to_id: replyToId,
      nonce,
    });
  }

  editMessage(channelId: Snowflake, messageId: Snowflake, content: string): Promise<Message> {
    return this.request("PATCH", `channels/${channelId}/messages/${messageId}`, { content });
  }

  deleteMessage(channelId: Snowflake, messageId: Snowflake): Promise<void> {
    return this.request("DELETE", `channels/${channelId}/messages/${messageId}`);
  }

  acknowledge(channelId: Snowflake, messageId: Snowflake): Promise<void> {
    return this.request("PUT", `channels/${channelId}/read-state`, {
      last_read_message_id: messageId,
    });
  }

  startTyping(channelId: Snowflake): Promise<void> {
    return this.request("POST", `channels/${channelId}/typing`);
  }

  openDirectMessage(recipientId: Snowflake): Promise<DirectMessageChannel> {
    return this.request("POST", "users/@me/channels", { recipient_id: recipientId });
  }
}

/** 32 hex characters, like the client core's `generate_message_nonce`. */
export function generateMessageNonce(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import type { MellowClient } from "../lib/client.svelte";
import type { Message } from "../lib/protocol";

// A notification that records what it was built with; replacing one by tag does not close it,
// as in Chrome.
class FakeNotification {
  static permission: NotificationPermission = "granted";
  static instances: FakeNotification[] = [];
  static shouldRefuse = false;
  onclick: (() => void) | null = null;
  onclose: (() => void) | null = null;
  isClosed = false;

  constructor(
    readonly title: string,
    readonly options: NotificationOptions,
  ) {
    if (FakeNotification.shouldRefuse) {
      throw new TypeError("Illegal constructor");
    }
    FakeNotification.instances.push(this);
  }

  close(): void {
    if (!this.isClosed) {
      this.isClosed = true;
      this.onclose?.();
    }
  }
}

const ME = "1";
const ANA = { id: "2", username: "ana", display_name: "Ana" };
const CONVERSATION = "300";

let notifier: typeof import("./notifier.svelte");
let preferences: typeof import("./notification-preferences.svelte");
let stop: () => void;
let nextMessageId = 1000;

const client = {
  conversations: [{ id: CONVERSATION, recipients: [ANA], last_message_id: null }],
  guilds: [],
  user: { id: ME, username: "me", display_name: "Me", status: "online" },
  selectedChannelId: null,
  isBlocked: () => false,
  selectGuild: vi.fn(),
  onIncomingMessage: null as ((message: Message) => void) | null,
};

function receive(content: string): void {
  nextMessageId += 1;
  client.onIncomingMessage?.({
    id: String(nextMessageId),
    channel_id: CONVERSATION,
    author: ANA,
    content,
    reply_to_id: null,
    edited_at_unix_milliseconds: null,
  });
}

const titles = () => FakeNotification.instances.map((notification) => notification.title);

beforeAll(async () => {
  vi.stubGlobal("Notification", FakeNotification);
  vi.stubGlobal("window", { addEventListener: vi.fn(), removeEventListener: vi.fn(), focus: vi.fn() });
  // The reader is away: the tab is hidden.
  vi.stubGlobal("document", {
    visibilityState: "hidden",
    hasFocus: () => false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    querySelector: () => null,
    title: "",
  });
  // Imported after the stubs: the notifier reads the permission when it loads.
  notifier = await import("./notifier.svelte");
  preferences = await import("./notification-preferences.svelte");
  preferences.updateNotificationSettings((settings) => ({ ...settings, areDesktopNotificationsEnabled: true, isSoundEnabled: false }));
});

beforeEach(() => {
  FakeNotification.instances = [];
  stop = notifier.startNotifier(client as unknown as MellowClient);
});

afterEach(() => stop());

afterAll(() => vi.unstubAllGlobals());

describe("browser notifications", () => {
  it("shows one notification per channel, replaced by tag and counted", () => {
    receive("Tu viens jeudi ?");
    receive("Réponds-moi");
    receive("Dernier message");
    expect(titles()).toEqual(["Ana", "Ana, 2 new messages", "Ana, 3 new messages"]);
    expect(new Set(FakeNotification.instances.map((notification) => notification.options.tag))).toEqual(
      new Set([`mellow-channel-${CONVERSATION}`]),
    );
    expect(FakeNotification.instances.every((notification) => notification.options.silent === true)).toBe(true);
  });

  it("starts counting again once the reader has seen the channel", () => {
    receive("un");
    receive("deux");
    notifier.noteChannelSeen(CONVERSATION);
    expect(FakeNotification.instances.at(-1)?.isClosed).toBe(true);
    receive("trois");
    expect(titles().at(-1)).toBe("Ana");
  });

  it("forgets the count when its notification closes, but not when a replaced one does", () => {
    receive("un");
    receive("deux");
    FakeNotification.instances[0]?.close();
    receive("trois");
    expect(titles().at(-1)).toBe("Ana, 3 new messages");
    FakeNotification.instances.at(-1)?.close();
    receive("quatre");
    expect(titles().at(-1)).toBe("Ana");
  });

  it("opens the conversation when its notification is clicked", () => {
    receive("un");
    FakeNotification.instances[0]?.onclick?.();
    expect(client.selectGuild).toHaveBeenCalledWith(null, CONVERSATION);
    expect(FakeNotification.instances[0]?.isClosed).toBe(true);
  });

  it("stops trying for the session once the browser refuses to build one", () => {
    FakeNotification.shouldRefuse = true;
    receive("un");
    expect(notifier.notificationSupport()).toBe("unsupported");
    FakeNotification.shouldRefuse = false;
    receive("deux");
    expect(FakeNotification.instances).toHaveLength(0);
  });
});

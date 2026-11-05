import type { Snowflake } from "../../lib/protocol";

// The one provider player open in the message list (ADR-0044, docs/design/web-provider-embeds.md
// principle 4): opening a player returns the open one to its poster, and unmounting its iframe
// stops its sound. Attachments already play one at a time (ADR-0038).

/** A preview's player in one message: the same preview can show under several messages. */
export function embedPlayerKey(messageId: Snowflake, previewId: Snowflake): string {
  return `${messageId}:${previewId}`;
}

let openPlayerKey = $state<string | null>(null);

export const embedPlayers = {
  isOpen(key: string): boolean {
    return openPlayerKey === key;
  },
  open(key: string): void {
    openPlayerKey = key;
  },
  /** Closes this player only: another one opened since stays open. */
  close(key: string): void {
    if (openPlayerKey === key) {
      openPlayerKey = null;
    }
  },
};

import type { Message, Snowflake } from "../lib/protocol";
import { messageCharactersLeft } from "./message-length";

type EditedMessage = Pick<Message, "id" | "content">;

/**
 * The chat's inline message editor (DESIGN.md §5.5): one message at a time, held above the rows so
 * a refused save gives the typed text back instead of losing it.
 */
export class MessageEditor {
  messageId = $state<Snowflake | null>(null);
  text = $state("");
  /** Counted as sent, like the composer's: a typed ":name:" or "@Role" goes out as longer markup. */
  readonly charactersLeft = $derived.by(() => messageCharactersLeft(this.#textAsSent(this.text)));
  readonly isTooLong = $derived(this.charactersLeft < 0);
  /** The instance would refuse it: say why at the first try, until the editor closes. */
  hasTriedSavingTooLong = $state(false);
  // Refused saves of messages whose editor could not reopen, until the message is edited again.
  readonly #refusedTextByMessageId = new Map<Snowflake, string>();
  readonly #textAsSent: (text: string) => string;

  constructor(textAsSent: (text: string) => string) {
    this.#textAsSent = textAsSent;
  }

  open(message: EditedMessage): void {
    this.messageId = message.id;
    this.text = this.#refusedTextByMessageId.get(message.id) ?? message.content;
    this.#refusedTextByMessageId.delete(message.id);
    this.hasTriedSavingTooLong = false;
  }

  close(): void {
    this.messageId = null;
    this.hasTriedSavingTooLong = false;
  }

  /** The text to save, closing the editor; null, with the editor left open, when it is too long. */
  takeTextToSave(): string | null {
    if (this.isTooLong) {
      this.hasTriedSavingTooLong = true;
      return null;
    }
    const text = this.text;
    this.close();
    return text;
  }

  /**
   * A refused save: the editor reopens with `text` when it is free and `canReopen` (the message is
   * still in the open chat); otherwise the text waits for the message's next edit.
   */
  giveBack(message: EditedMessage, text: string, canReopen: boolean): void {
    if (canReopen && this.messageId === null) {
      this.open(message);
      this.text = text;
    } else if (this.messageId !== message.id) {
      this.#refusedTextByMessageId.set(message.id, text);
    }
  }
}

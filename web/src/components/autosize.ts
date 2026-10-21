import type { Attachment } from "svelte/attachments";

/** Grows a text area with its text up to `maximumHeight()` pixels, then it scrolls. */
export function autosize(readValue: () => string, maximumHeight: () => number): Attachment<HTMLTextAreaElement> {
  return (textarea) => {
    const value = readValue();
    // The binding may not have written a programmatic change (send, draft restore) to the DOM yet.
    if (textarea.value !== value) {
      textarea.value = value;
    }
    textarea.style.height = "auto";
    textarea.style.height = `${Math.min(textarea.scrollHeight, maximumHeight())}px`;
  };
}

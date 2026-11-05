// How a checklist or a progress bar reads where its card does not show (ADR-0047): the one line
// that stands for it where a message is quoted, and its look while still in the outbox.

import { strings } from "../i18n";
import type { Checklist, Message, NewChecklist, NewProgressBar, ProgressBar } from "../lib/protocol";

/** "Checklist: Release" or "Progress: Migration"; `null` for any other message. */
export function trackerSummary(message: Pick<Message, "checklist" | "progress_bar">): string | null {
  if (message.checklist) {
    return strings.checklists.summary(message.checklist.title);
  }
  return message.progress_bar ? strings.progressBars.summary(message.progress_bar.label) : null;
}

/** A checklist still in the outbox, shown as it will arrive: nothing ticked. */
export function pendingChecklistPreview(checklist: NewChecklist): Checklist {
  return {
    title: checklist.title,
    items: checklist.items.map((text, index) => ({ id: index + 1, text })),
    revision: 0,
  };
}

export function pendingProgressBarPreview(bar: NewProgressBar): ProgressBar {
  return { ...bar, revision: 0 };
}

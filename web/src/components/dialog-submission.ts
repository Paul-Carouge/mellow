// What every dialog with one action on the instance shares (ModalDialog): busy while it answers, so
// the dialog cannot close or submit twice, and a refusal shown as the dialog's own error sentence.

import { describeError } from "../lib/client.svelte";

export interface DialogSubmission {
  isBusy: boolean;
  errorMessage: string;
}

/**
 * Waits for `action` with `submission` busy and its previous error cleared. A failure becomes
 * `failureSummary` followed by the reason; success leaves the error empty, as the dialog closes.
 */
export async function trackDialogSubmission(
  submission: DialogSubmission,
  action: Promise<void>,
  failureSummary: string,
): Promise<void> {
  submission.isBusy = true;
  submission.errorMessage = "";
  try {
    await action;
  } catch (error) {
    submission.errorMessage = describeError(error, failureSummary);
  } finally {
    submission.isBusy = false;
  }
}

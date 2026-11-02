import { describe, expect, it } from "vitest";

import { strings } from "../i18n";
import { ApiError } from "../lib/api";
import { type DialogSubmission, trackDialogSubmission } from "./dialog-submission";

const FAILURE_SUMMARY = "Couldn't rename the category";

/** A promise the test settles itself, so the state in between can be read. */
function deferredAction(): { promise: Promise<void>; resolve: () => void; reject: (error: unknown) => void } {
  let resolve = () => {};
  let reject: (error: unknown) => void = () => {};
  const promise = new Promise<void>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("trackDialogSubmission", () => {
  it("is busy, with the previous error cleared, until the action succeeds", async () => {
    const submission: DialogSubmission = { isBusy: false, errorMessage: "an earlier failure" };
    const action = deferredAction();

    const tracking = trackDialogSubmission(submission, action.promise, FAILURE_SUMMARY);
    expect(submission).toEqual({ isBusy: true, errorMessage: "" });

    action.resolve();
    await tracking;
    expect(submission).toEqual({ isBusy: false, errorMessage: "" });
  });

  it("explains a refusal with the instance's reason and lets the dialog be used again", async () => {
    const submission: DialogSubmission = { isBusy: false, errorMessage: "" };
    const action = deferredAction();

    const tracking = trackDialogSubmission(submission, action.promise, FAILURE_SUMMARY);
    action.reject(new ApiError(0, "NETWORK"));
    await tracking;

    expect(submission).toEqual({
      isBusy: false,
      errorMessage: strings.errors.withReason(FAILURE_SUMMARY, strings.errors.reasons.NETWORK),
    });
  });

  it("falls back to the unknown reason for anything but an API error", async () => {
    const submission: DialogSubmission = { isBusy: false, errorMessage: "" };

    await trackDialogSubmission(submission, Promise.reject(new TypeError("offline")), FAILURE_SUMMARY);

    expect(submission).toEqual({
      isBusy: false,
      errorMessage: strings.errors.withReason(FAILURE_SUMMARY, strings.errors.unknownReason),
    });
  });
});

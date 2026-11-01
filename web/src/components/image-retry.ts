import type { Snowflake } from "../lib/protocol";

/** Long enough for a busy instance to free a download slot, short enough to go unnoticed. */
export const IMAGE_RETRY_DELAY_MILLISECONDS = 2_000;

/**
 * Called when an image fails to load. The first failure schedules one more try (the id joins
 * `retriedIds` after a short delay, which reloads the image); a failure after that is final and
 * the id joins `failedIds`. A busy instance answering 503 for a moment then breaks no image.
 */
export function retryImageOnceThenMarkFailed(
  id: Snowflake,
  retriedIds: Set<Snowflake>,
  failedIds: Set<Snowflake>,
): void {
  if (retriedIds.has(id)) {
    failedIds.add(id);
    return;
  }
  setTimeout(() => retriedIds.add(id), IMAGE_RETRY_DELAY_MILLISECONDS);
}

// Live "is this name free?" while someone types a new name (ADR-0048): one request once typing
// pauses, and an answer about an older draft never replaces the newer one's.

import { ApiError } from "../lib/api";

/** Long enough that a typed word sends one request, short enough to feel live. */
export const NAME_CHECK_PAUSE_MILLISECONDS = 400;

export type NameAvailabilityState = "idle" | "checking" | "available" | "taken" | "invalid" | "unknown";

/** Trimmed, with inner runs of spaces made one, as the instance stores names. */
export function tidiedName(name: string): string {
  return name.trim().replace(/\s+/gu, " ");
}

export class NameAvailabilityCheck {
  state = $state<NameAvailabilityState>("idle");
  private generation = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor(private readonly isNameAvailable: (name: string) => Promise<boolean>) {}

  /** Checks `draft` once typing pauses; nothing to check while it is empty or the current name. */
  update(draft: string, currentName: string): void {
    clearTimeout(this.timer);
    this.generation += 1;
    const generation = this.generation;
    const name = tidiedName(draft);
    if (name === "" || name === currentName) {
      this.state = "idle";
      return;
    }
    this.state = "checking";
    this.timer = setTimeout(() => {
      this.isNameAvailable(name).then(
        (isAvailable) => {
          if (generation === this.generation) {
            this.state = isAvailable ? "available" : "taken";
          }
        },
        (error: unknown) => {
          if (generation === this.generation) {
            // The save still checks; an unreachable instance only loses the hint.
            this.state = error instanceof ApiError && error.code === "VALIDATION_FAILED" ? "invalid" : "unknown";
          }
        },
      );
    }, NAME_CHECK_PAUSE_MILLISECONDS);
  }

  /** Stops a pending check, so a closed form gets no late answer. */
  cancel(): void {
    clearTimeout(this.timer);
    this.generation += 1;
    this.state = "idle";
  }
}

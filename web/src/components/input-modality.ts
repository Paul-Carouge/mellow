// The last input modality, kept on <html> as `data-input-modality` (DESIGN.md §7.2). A text field
// matches `:focus-visible` on a click too, so the composer, focused almost all the time, reads this
// instead to show its ring only when focus came from the keyboard.

export type InputModality = "keyboard" | "pointer";

/** Keys that move focus between controls: Tab (with or without Shift), F6 between regions. */
const FOCUS_MOVING_KEYS: ReadonlySet<string> = new Set(["Tab", "F6"]);
const ARROW_KEYS: ReadonlySet<string> = new Set(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"]);

/**
 * "keyboard" when `key` navigates, null when it leaves the modality as it was. Arrows navigate in
 * lists and menus, but inside a text field they only move the caret: typing after a click is
 * still a pointer session.
 */
export function inputModalityAfterKeyDown(key: string, isInTextField: boolean): InputModality | null {
  if (FOCUS_MOVING_KEYS.has(key) || (ARROW_KEYS.has(key) && !isInTextField)) {
    return "keyboard";
  }
  return null;
}

function isTextField(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && (target.isContentEditable || target.matches("input, textarea"));
}

/**
 * Writes the modality on `root` from the window's key and pointer presses, in the capture phase so
 * a handler stopping the event cannot hide it; until the returned function is called.
 */
export function trackInputModality(target: Window, root: HTMLElement): () => void {
  const noteKey = (event: KeyboardEvent) => {
    const modality = inputModalityAfterKeyDown(event.key, isTextField(event.target));
    if (modality !== null) {
      root.dataset.inputModality = modality;
    }
  };
  const notePointer = () => {
    root.dataset.inputModality = "pointer";
  };
  target.addEventListener("keydown", noteKey, { capture: true });
  target.addEventListener("pointerdown", notePointer, { capture: true });
  return () => {
    target.removeEventListener("keydown", noteKey, { capture: true });
    target.removeEventListener("pointerdown", notePointer, { capture: true });
    delete root.dataset.inputModality;
  };
}

// The last input modality, kept on <html> as `data-input-modality` (DESIGN.md §7.2). A text field
// matches `:focus-visible` on a click too, so the composer, focused almost all the time, reads this
// instead to show its ring only when focus came from the keyboard.

export type InputModality = "keyboard" | "pointer";

export interface InputModalityState {
  /** Null until the first navigation key or pointer press. */
  readonly modality: InputModality | null;
  /** The last key was Tab or F6: it counts as navigation only if focus moves next. */
  readonly isFocusMoveFromKeyAwaited: boolean;
}

export type InputModalityEvent =
  | { readonly kind: "keydown"; readonly key: string; readonly isInTextField: boolean }
  | { readonly kind: "focusin" }
  | { readonly kind: "pointerdown" };

export const INITIAL_INPUT_MODALITY_STATE: InputModalityState = { modality: null, isFocusMoveFromKeyAwaited: false };

/** Keys that move focus between controls: Tab (with or without Shift), F6 between regions. */
const FOCUS_MOVING_KEYS: ReadonlySet<string> = new Set(["Tab", "F6"]);
const ARROW_KEYS: ReadonlySet<string> = new Set(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"]);

/**
 * The modality after `event`. Tab and F6 count once focus actually moves: the composer keeps Tab
 * to accept a suggestion, and that is still typing. Arrows navigate in lists and menus, but inside
 * a text field they only move the caret.
 */
export function nextInputModalityState(state: InputModalityState, event: InputModalityEvent): InputModalityState {
  switch (event.kind) {
    case "pointerdown":
      return { modality: "pointer", isFocusMoveFromKeyAwaited: false };
    case "focusin":
      return state.isFocusMoveFromKeyAwaited ? { modality: "keyboard", isFocusMoveFromKeyAwaited: false } : state;
    case "keydown": {
      const isArrowNavigation = ARROW_KEYS.has(event.key) && !event.isInTextField;
      return {
        modality: isArrowNavigation ? "keyboard" : state.modality,
        isFocusMoveFromKeyAwaited: FOCUS_MOVING_KEYS.has(event.key),
      };
    }
    default: {
      const unreachable: never = event;
      return unreachable;
    }
  }
}

function isTextField(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && (target.isContentEditable || target.matches("input, textarea"));
}

/**
 * Writes the modality on `root` from the window's key presses, focus moves and pointer presses, in
 * the capture phase so a handler stopping the event cannot hide it; until the returned function is
 * called.
 */
export function trackInputModality(target: Window, root: HTMLElement): () => void {
  let state = INITIAL_INPUT_MODALITY_STATE;
  const apply = (event: InputModalityEvent) => {
    state = nextInputModalityState(state, event);
    if (state.modality !== null) {
      root.dataset.inputModality = state.modality;
    }
  };
  const noteKey = (event: KeyboardEvent) => apply({ kind: "keydown", key: event.key, isInTextField: isTextField(event.target) });
  const noteFocus = () => apply({ kind: "focusin" });
  const notePointer = () => apply({ kind: "pointerdown" });
  target.addEventListener("keydown", noteKey, { capture: true });
  target.addEventListener("focusin", noteFocus, { capture: true });
  target.addEventListener("pointerdown", notePointer, { capture: true });
  return () => {
    target.removeEventListener("keydown", noteKey, { capture: true });
    target.removeEventListener("focusin", noteFocus, { capture: true });
    target.removeEventListener("pointerdown", notePointer, { capture: true });
    delete root.dataset.inputModality;
  };
}

import { describe, expect, it } from "vitest";
import { type InputModalityEvent, type InputModalityState, INITIAL_INPUT_MODALITY_STATE, nextInputModalityState } from "./input-modality";

function modalityAfter(events: readonly InputModalityEvent[]): InputModalityState["modality"] {
  return events.reduce(nextInputModalityState, INITIAL_INPUT_MODALITY_STATE).modality;
}

const POINTER_PRESS: InputModalityEvent = { kind: "pointerdown" };
const FOCUS_MOVED: InputModalityEvent = { kind: "focusin" };

function keyDown(key: string, isInTextField: boolean): InputModalityEvent {
  return { kind: "keydown", key, isInTextField };
}

describe("nextInputModalityState", () => {
  it("counts Tab and F6 as keyboard navigation once they move focus, even from a text field", () => {
    expect(modalityAfter([POINTER_PRESS, keyDown("Tab", false), FOCUS_MOVED])).toBe("keyboard");
    expect(modalityAfter([POINTER_PRESS, keyDown("Tab", true), FOCUS_MOVED])).toBe("keyboard");
    expect(modalityAfter([POINTER_PRESS, keyDown("F6", true), FOCUS_MOVED])).toBe("keyboard");
  });

  it("keeps a pointer session when a text field keeps Tab to accept a suggestion", () => {
    // Click the composer, type ":smi", Tab completes the emoji: focus never leaves the field.
    expect(modalityAfter([POINTER_PRESS, keyDown("Tab", true)])).toBe("pointer");
    // A later focus change that no key caused (a click, a programmatic focus) is not a Tab.
    expect(modalityAfter([POINTER_PRESS, keyDown("Tab", true), keyDown("a", true), FOCUS_MOVED])).toBe("pointer");
    expect(modalityAfter([keyDown("Tab", true), POINTER_PRESS, FOCUS_MOVED])).toBe("pointer");
  });

  it("counts arrows as navigation outside text fields only", () => {
    expect(modalityAfter([POINTER_PRESS, keyDown("ArrowDown", false)])).toBe("keyboard");
    expect(modalityAfter([POINTER_PRESS, keyDown("ArrowLeft", true)])).toBe("pointer");
  });

  it("leaves the modality alone while typing", () => {
    expect(modalityAfter([POINTER_PRESS, keyDown("a", true), keyDown("Enter", false), keyDown("Escape", false)])).toBe("pointer");
    expect(modalityAfter([keyDown("a", true)])).toBeNull();
  });

  it("goes back to pointer on a press", () => {
    expect(modalityAfter([keyDown("ArrowDown", false), POINTER_PRESS])).toBe("pointer");
  });
});

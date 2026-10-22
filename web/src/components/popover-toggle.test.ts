import { describe, expect, it } from "vitest";

import { createPopoverToggle } from "./popover-toggle";

describe("popover toggles", () => {
  it("open on a click when closed, and stay closed when the press light-dismissed them", () => {
    let isOpen = false;
    const toggle = createPopoverToggle(() => isOpen);
    toggle.notePress();
    expect(toggle.shouldOpenOnClick()).toBe(true);

    isOpen = true;
    toggle.notePress();
    // The light dismiss closes it between the press and the click.
    isOpen = false;
    expect(toggle.shouldOpenOnClick()).toBe(false);

    // A keyboard click has no press: it opens a closed popover.
    expect(toggle.shouldOpenOnClick()).toBe(true);
  });

  it("never reopens a popover still open at the click", () => {
    const toggle = createPopoverToggle(() => true);
    expect(toggle.shouldOpenOnClick()).toBe(false);
  });
});

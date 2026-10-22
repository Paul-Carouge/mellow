// The button that opens a light-dismiss popover (`popover="auto"`) or a picker held by its parent.

/**
 * A press on the button of an open popover light-dismisses it before the click arrives, so a
 * plain toggle in the click would open it again at once. The press notes whether it was open;
 * the click then opens it only if it was not.
 */
export function createPopoverToggle(isOpen: () => boolean): {
  notePress: () => void;
  shouldOpenOnClick: () => boolean;
} {
  let wasOpenAtPress = false;
  return {
    notePress: () => {
      wasOpenAtPress = isOpen();
    },
    shouldOpenOnClick: () => {
      const shouldOpen = !isOpen() && !wasOpenAtPress;
      wasOpenAtPress = false;
      return shouldOpen;
    },
  };
}

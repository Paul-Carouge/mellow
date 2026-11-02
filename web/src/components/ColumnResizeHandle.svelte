<script lang="ts">
  import { strings } from "../i18n";
  import { type ColumnBounds, clampColumnWidth, widthAfterKey } from "./column-widths";
  import { lengthTokenInPixels } from "./tokens";

  // A focusable window splitter (WAI-ARIA) in the gap beside a column of the wide layout
  // (DESIGN.md 4.5): drag it, or use the arrows, Home and End; a double-click puts the default
  // width back and Esc cancels a drag. It covers the gap and the leading edge of the card after
  // it, never the trailing edge before it, where the scrollbars are. It reports widths only: the
  // shell owns them and lays them out through one custom property.
  interface Props {
    label: string;
    /** The id of the column it resizes. */
    controls: string;
    /** The column's edge it sits on: the channel list's trailing edge, the side sheet's leading edge. */
    edge: "start" | "end";
    width: number;
    /** `maximum` is what the window allows now, not only the column's own maximum. */
    bounds: ColumnBounds;
    onresize: (width: number) => void;
    /** The gesture or key is over: the width is worth remembering. */
    onresizeend: () => void;
    /** Esc during a drag: the width goes back to the one remembered. */
    onresizecancel: () => void;
  }

  let { label, controls, edge, width, bounds, onresize, onresizeend, onresizecancel }: Props = $props();

  let handle: HTMLDivElement;
  let isDragging = $state(false);
  /** Ends the drag in progress without a word to the shell. */
  let stopDrag: (() => void) | null = null;

  // A column that goes mid-drag (its server left, Ctrl+F swapping members for results) takes its
  // handle along: the drag stops there, and later moves resize nothing.
  $effect(() => () => stopDrag?.());

  /** Whether moving the edge to the right widens the column, right to left included. */
  function growsRightwards(): boolean {
    const isRightToLeft = getComputedStyle(handle).direction === "rtl";
    return (edge === "end") !== isRightToLeft;
  }

  function startDrag(event: PointerEvent) {
    if (event.button !== 0 || !event.isPrimary) {
      return;
    }
    const startX = event.clientX;
    const startWidth = width;
    const direction = growsRightwards() ? 1 : -1;

    const follow = (move: PointerEvent) => {
      if (move.pointerId === event.pointerId) {
        onresize(clampColumnWidth(startWidth + direction * (move.clientX - startX), bounds, bounds.maximum));
      }
    };
    const cancelWithEscape = (key: KeyboardEvent) => {
      if (key.key === "Escape") {
        key.preventDefault();
        key.stopPropagation();
        stop();
        onresizecancel();
      }
    };
    const finish = () => {
      stop();
      onresizeend();
    };
    const stop = () => {
      stopDrag = null;
      window.removeEventListener("pointermove", follow);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
      window.removeEventListener("keydown", cancelWithEscape, true);
      handle.removeEventListener("lostpointercapture", finish);
      if (handle.hasPointerCapture(event.pointerId)) {
        handle.releasePointerCapture(event.pointerId);
      }
      isDragging = false;
    };

    stopDrag?.();
    stopDrag = stop;
    isDragging = true;
    // Captured, so the drag follows the pointer past the handle; followed on the window as well,
    // where synthetic drags arrive (docs/development-notes.md, "Drags under CDP").
    handle.setPointerCapture(event.pointerId);
    window.addEventListener("pointermove", follow);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    window.addEventListener("keydown", cancelWithEscape, true);
    // A capture lost with the button released elsewhere ends the drag, or hovering would go on
    // resizing.
    handle.addEventListener("lostpointercapture", finish);
  }

  function resizeWithKey(event: KeyboardEvent) {
    if (event.ctrlKey || event.metaKey || event.altKey) {
      return;
    }
    const step = lengthTokenInPixels("component-column-resize-keyboard-step");
    const next = widthAfterKey(event.key, width, bounds, growsRightwards(), step);
    if (next === null) {
      return;
    }
    event.preventDefault();
    onresize(next);
    onresizeend();
  }

  function resetWidth() {
    onresize(clampColumnWidth(bounds.default, bounds, bounds.maximum));
    onresizeend();
  }
</script>

<!-- A focusable separator is a widget in WAI-ARIA (the window splitter), which Svelte's checks
     do not know. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  bind:this={handle}
  role="separator"
  tabindex="0"
  aria-orientation="vertical"
  aria-label={label}
  aria-controls={controls}
  aria-valuenow={width}
  aria-valuemin={bounds.minimum}
  aria-valuemax={bounds.maximum}
  aria-valuetext={strings.shell.columnWidth(width)}
  data-dragging={isDragging || undefined}
  class={[
    // From the start of the gap onwards: the gap, then the leading edge of the next card (the
    // conversation or the side sheet), whose first pixels hold no control and no scrollbar.
    "group absolute inset-y-0 z-10 flex w-(--mw-component-column-resize-hit-width) cursor-col-resize touch-none justify-start select-none outline-none pointer-coarse:w-(--mw-size-hit-web-min)",
    edge === "end" ? "start-full" : "start-[calc(-1*var(--mw-size-sheet-inset))]",
  ]}
  onpointerdown={startDrag}
  onkeydown={resizeWithKey}
  ondblclick={resetWidth}
>
  <!-- The line stays centred on the gap. -->
  <span
    aria-hidden="true"
    class="my-(--mw-component-column-resize-line-inset) ms-[calc((var(--mw-size-sheet-inset)-var(--mw-component-column-resize-line-width))/2)] w-(--mw-component-column-resize-line-width) rounded-full transition-colors duration-(--mw-motion-duration-fast) group-hover:bg-(--mw-component-column-resize-line-hover) group-focus-visible:bg-(--mw-component-column-resize-line-focus) group-data-dragging:bg-(--mw-component-column-resize-line-active)"
  ></span>
</div>

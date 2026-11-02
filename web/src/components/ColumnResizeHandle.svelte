<script lang="ts">
  import { strings } from "../i18n";
  import { type ColumnBounds, clampColumnWidth, widthAfterKey } from "./column-widths";
  import { lengthTokenInPixels } from "./tokens";

  // A focusable window splitter (WAI-ARIA) in the gap beside a column of the wide layout
  // (DESIGN.md 4.5): drag it, or use the arrows, Home and End; a double-click puts the default
  // width back and Esc cancels a drag. It sits on its column's edge, centred on the gap, and
  // reports widths only: the shell owns them and lays them out through one custom property.
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
  }

  let { label, controls, edge, width, bounds, onresize, onresizeend }: Props = $props();

  let handle: HTMLDivElement;
  let isDragging = $state(false);

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
    // Captured, so the drag follows the pointer past the handle; followed on the window as well,
    // where synthetic drags arrive (docs/development-notes.md, "Drags under CDP").
    handle.setPointerCapture(event.pointerId);
    isDragging = true;

    const follow = (move: PointerEvent) => {
      if (move.pointerId === event.pointerId) {
        onresize(clampColumnWidth(startWidth + direction * (move.clientX - startX), bounds, bounds.maximum));
      }
    };
    const cancelWithEscape = (key: KeyboardEvent) => {
      if (key.key === "Escape") {
        key.preventDefault();
        key.stopPropagation();
        onresize(startWidth);
        finish();
      }
    };
    const finish = () => {
      window.removeEventListener("pointermove", follow);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
      window.removeEventListener("keydown", cancelWithEscape, true);
      if (handle.hasPointerCapture(event.pointerId)) {
        handle.releasePointerCapture(event.pointerId);
      }
      isDragging = false;
      onresizeend();
    };
    window.addEventListener("pointermove", follow);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    window.addEventListener("keydown", cancelWithEscape, true);
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
    // Centred on the gap after (or before) the card: half the gap out, half the hit width back.
    "group absolute inset-y-0 z-10 flex w-(--column-resize-hit-width) cursor-col-resize touch-none justify-center select-none outline-none",
    "[--column-resize-hit-width:var(--mw-component-column-resize-hit-width)] pointer-coarse:[--column-resize-hit-width:var(--mw-size-hit-web-min)]",
    edge === "end"
      ? "start-full ms-[calc((var(--mw-size-sheet-inset)-var(--column-resize-hit-width))/2)]"
      : "end-full me-[calc((var(--mw-size-sheet-inset)-var(--column-resize-hit-width))/2)]",
  ]}
  onpointerdown={startDrag}
  onkeydown={resizeWithKey}
  ondblclick={resetWidth}
>
  <span
    aria-hidden="true"
    class="my-(--mw-component-column-resize-line-inset) w-(--mw-component-column-resize-line-width) rounded-full transition-colors duration-(--mw-motion-duration-fast) group-hover:bg-(--mw-component-column-resize-line-hover) group-focus-visible:bg-(--mw-component-column-resize-line-focus) group-data-dragging:bg-(--mw-component-column-resize-line-active)"
  ></span>
</div>

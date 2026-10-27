<script lang="ts">
  import { onMount, type Snippet } from "svelte";

  // A panel that slides over the sheet (DESIGN.md §4.2, ADR-0022 §1): the navigator from the
  // leading edge below 816 px, the side sheet from the trailing edge below 1064 px. A native modal
  // <dialog> keeps the rest inert; Esc and the scrim close it.
  interface Props {
    label: string;
    side: "start" | "end";
    /** The side sheet covers the whole window on phones; elsewhere it floats inside the inset. */
    isFullWindow?: boolean;
    onclose: () => void;
    children: Snippet;
  }

  let { label, side, isFullWindow = false, onclose, children }: Props = $props();

  let dialog: HTMLDialogElement;

  onMount(() => {
    const opener =
      document.activeElement instanceof HTMLElement && document.activeElement !== document.body
        ? document.activeElement
        : null;
    dialog.showModal();
    // The current channel takes focus, so the reader starts where they are, not on the first icon.
    const current = [...dialog.querySelectorAll<HTMLElement>("[aria-current=page]")].at(-1);
    (current ?? dialog).focus();
    return () => {
      dialog.close();
      if (opener?.isConnected && !dialog.contains(document.activeElement)) {
        opener.focus();
      }
    };
  });
</script>

<dialog
  bind:this={dialog}
  aria-label={label}
  tabindex="-1"
  class={[
    "drawer fixed m-0 max-h-none overflow-hidden bg-transparent p-0 outline-none backdrop:bg-scrim",
    side === "start"
      ? "drawer-from-start inset-y-0 left-0 h-dvh max-w-[calc(100vw_-_var(--mw-size-mobile-panel-peek))]"
      : isFullWindow
        ? "drawer-from-end inset-0 size-full max-w-none"
        : // Inside the conversation sheet's frame, one inset in from its top and bottom (as in the mockup).
          "drawer-from-end top-[calc(3*var(--mw-size-sheet-inset))] right-[calc(2*var(--mw-size-sheet-inset)+env(safe-area-inset-right))] bottom-[calc(2*var(--mw-size-sheet-inset))] left-auto h-auto rounded-(--mw-component-side-sheet-radius) shadow-high",
  ]}
  oncancel={(event) => {
    event.preventDefault();
    onclose();
  }}
  onclick={(event) => {
    if (event.target === dialog) {
      onclose();
    }
  }}
>
  <div class="flex h-full">
    {@render children()}
  </div>
</dialog>

<style>
  .drawer::backdrop {
    animation: mw-fade-in var(--mw-motion-duration-base) var(--mw-motion-easing-standard) both;
  }
  .drawer-from-start {
    animation: mw-slide-from-start var(--mw-motion-duration-slow) var(--mw-motion-easing-enter) both;
  }
  .drawer-from-end {
    animation: mw-slide-from-end var(--mw-motion-duration-slow) var(--mw-motion-easing-enter) both;
  }
</style>

<script lang="ts">
  import { onMount, type Snippet } from "svelte";

  // A pane that slides over the chat in narrow windows (DESIGN.md §4.2): the rail and channel
  // sidebar from the leading edge, the member list from the trailing edge. A native modal
  // <dialog> keeps the chat behind it inert; Esc and the scrim close it.
  interface Props {
    label: string;
    side: "start" | "end";
    onclose: () => void;
    children: Snippet;
  }

  let { label, side, onclose, children }: Props = $props();

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
    "fixed inset-y-0 m-0 outline-none h-dvh max-h-none max-w-[calc(100vw_-_var(--mw-size-mobile-panel-peek))] overflow-hidden bg-transparent p-0 shadow-high backdrop:bg-scrim",
    side === "start" ? "left-0" : "right-0 left-auto",
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

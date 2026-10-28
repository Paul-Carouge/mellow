<script lang="ts">
  import { onMount, tick, type Snippet } from "svelte";

  import { strings } from "../i18n";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";

  // DESIGN.md §6.35: title, optional sentence, the dialog's own controls, an inline error, and a
  // footer with Cancel and one action. A native modal <dialog> keeps the page behind it inert.
  // Esc, Cancel and the scrim close it except while busy; focus goes back where it came from.
  interface Props {
    title: string;
    subtitle?: string;
    isHeadingCentered?: boolean;
    /** Two columns side by side ("Create a server"), stacked when the window is narrower. */
    isWide?: boolean;
    errorMessage?: string;
    isBusy?: boolean;
    cancelLabel?: string;
    /** Empty leaves Cancel alone in the footer. */
    submitLabel?: string;
    isSubmitDanger?: boolean;
    isSubmitEnabled?: boolean;
    onclose: () => void;
    onsubmit?: () => void;
    /** Where focus goes when the element that opened the dialog is gone. */
    fallbackFocus?: () => void;
    /** False once the dialog did its job (a server created, a channel opened): focus then goes to
     * `fallbackFocus`, since the opener (the rail's +, a menu item) would only reopen it. A function,
     * read when the dialog closes: a prop changed right before closing is not seen by then. */
    shouldReturnFocusToOpener?: () => boolean;
    children?: Snippet;
  }

  let {
    title,
    subtitle = "",
    isHeadingCentered = false,
    isWide = false,
    errorMessage = "",
    isBusy = false,
    cancelLabel = strings.common.cancel,
    submitLabel = "",
    isSubmitDanger = false,
    isSubmitEnabled = true,
    onclose,
    onsubmit,
    fallbackFocus,
    shouldReturnFocusToOpener = () => true,
    children,
  }: Props = $props();

  const id = $props.id();
  let dialog: HTMLDialogElement;

  function requestClose() {
    if (!isBusy) {
      onclose();
    }
  }

  /** An element gone, or hidden like a message's action bar outside its row, cannot take focus. */
  function focusIfPossible(element: HTMLElement): boolean {
    element.focus();
    return element.isConnected && document.activeElement === element;
  }

  onMount(() => {
    const opener =
      document.activeElement instanceof HTMLElement && document.activeElement !== document.body
        ? document.activeElement
        : null;
    dialog.showModal();
    // Each dialog marks the control that takes focus first; destructive ones mark Cancel (§8).
    dialog.querySelector<HTMLElement>("[data-initial-focus]")?.focus();
    return () => {
      dialog.close();
      const isFocusBackOnOpener = shouldReturnFocusToOpener() && opener !== null && focusIfPossible(opener);
      if (!isFocusBackOnOpener && fallbackFocus) {
        // After the view the dialog changed (a new server, a new channel) has rendered, and after
        // the native dialog put focus back on its opener.
        const focusFallback = fallbackFocus;
        void tick().then(() => requestAnimationFrame(focusFallback));
      }
    };
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="{id}-title"
  aria-describedby={subtitle ? `${id}-subtitle` : undefined}
  class:dialog-wide={isWide}
  class="modal-dialog m-auto w-[min(var(--mw-component-dialog-width),calc(100vw_-_2*var(--mw-component-dialog-margin)))] max-w-none overflow-hidden rounded-(--mw-component-dialog-radius) bg-(--mw-component-dialog-background) p-0 text-(--mw-component-dialog-body-text) shadow-high backdrop:bg-(--mw-component-dialog-scrim) dark:[outline:1px_solid_var(--mw-color-border-divider)]"
  oncancel={(event) => {
    event.preventDefault();
    requestClose();
  }}
  onclick={(event) => {
    // A click on the ::backdrop targets the dialog itself; the card's padding sits inside it.
    if (event.target === dialog) {
      requestClose();
    }
  }}
>
  <form
    method="dialog"
    class="flex max-h-[calc(100dvh_-_2*var(--mw-component-dialog-margin))] flex-col"
    onsubmit={(event) => {
      event.preventDefault();
      if (!isBusy && isSubmitEnabled && submitLabel) {
        onsubmit?.();
      }
    }}
  >
    <div class="flex flex-col gap-(--mw-component-dialog-section-gap) overflow-y-auto p-(--mw-component-dialog-padding)">
      <div class={["flex flex-col gap-(--mw-component-dialog-subtitle-gap)", isHeadingCentered && "text-center"]}>
        <h2 id="{id}-title" class="type-heading-lg text-(--mw-component-dialog-title-text)">{title}</h2>
        {#if subtitle}
          <p id="{id}-subtitle" class="type-body text-(--mw-component-dialog-subtitle-text)">{subtitle}</p>
        {/if}
      </div>

      {@render children?.()}

      <!-- Kept in the page, so the error is announced as soon as it appears. -->
      <div role="alert" class="contents">
        {#if errorMessage}
          <p class="flex gap-(--mw-component-dialog-error-gap) type-body text-(--mw-component-dialog-error-text)">
            <Icon name="warning-circle" class="size-(--mw-component-dialog-error-icon-size)" />
            <span>{errorMessage}</span>
          </p>
        {/if}
      </div>
    </div>
    <div
      class="flex flex-wrap justify-end gap-(--mw-component-dialog-button-gap) bg-(--mw-component-dialog-footer-background) px-(--mw-component-dialog-padding) pb-(--mw-component-dialog-padding)"
    >
      <!-- Beside an action, Cancel stays quiet; alone, it is the dialog's button. -->
      <Button
        variant={submitLabel ? "ghost" : "secondary"}
        isDisabled={isBusy}
        data-initial-focus={isSubmitDanger || !submitLabel ? "" : undefined}
        onclick={requestClose}>{cancelLabel}</Button
      >
      {#if submitLabel}
        <Button type="submit" variant={isSubmitDanger ? "danger" : "primary"} isDisabled={!isSubmitEnabled || isBusy}
          >{submitLabel}</Button
        >
      {/if}
    </div>
  </form>
</dialog>

<style>
  /* Dialogs land with the settle curve (ADR-0022 §3); the scrim fades. Reduced motion: at once. */
  .modal-dialog.dialog-wide {
    width: min(var(--mw-component-dialog-width-wide), calc(100vw - 2 * var(--mw-component-dialog-margin)));
  }
  .modal-dialog[open] {
    animation: modal-dialog-in var(--mw-motion-duration-settle) var(--mw-motion-easing-settle) both;
  }
  .modal-dialog[open]::backdrop {
    animation: mw-fade-in var(--mw-motion-duration-base) var(--mw-motion-easing-standard) both;
  }
  @keyframes modal-dialog-in {
    from {
      opacity: 0;
      transform: scale(var(--mw-component-dialog-enter-scale));
    }
  }
</style>

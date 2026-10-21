<script lang="ts">
  import { onMount, type Snippet } from "svelte";

  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";

  // DESIGN.md §6.35: title, optional sentence, the dialog's own controls, an inline error, and a
  // footer with Cancel and one action. A native modal <dialog> keeps the page behind it inert.
  // Esc, Cancel and the scrim close it except while busy; focus goes back where it came from.
  interface Props {
    title: string;
    subtitle?: string;
    isHeadingCentered?: boolean;
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
    children?: Snippet;
  }

  let {
    title,
    subtitle = "",
    isHeadingCentered = false,
    errorMessage = "",
    isBusy = false,
    cancelLabel = "Cancel",
    submitLabel = "",
    isSubmitDanger = false,
    isSubmitEnabled = true,
    onclose,
    onsubmit,
    fallbackFocus,
    children,
  }: Props = $props();

  const id = $props.id();
  let dialog: HTMLDialogElement;

  function requestClose() {
    if (!isBusy) {
      onclose();
    }
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
      if (opener?.isConnected) {
        opener.focus();
      } else {
        fallbackFocus?.();
      }
    };
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="{id}-title"
  aria-describedby={subtitle ? `${id}-subtitle` : undefined}
  class="m-auto w-[min(var(--mw-component-dialog-width),calc(100vw_-_2*var(--mw-component-dialog-margin)))] max-w-none overflow-hidden rounded-(--mw-component-dialog-radius) bg-(--mw-component-dialog-background) p-0 text-(--mw-component-dialog-body-text) shadow-high backdrop:bg-(--mw-component-dialog-scrim)"
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
      class="flex flex-wrap justify-end gap-(--mw-component-dialog-button-gap) bg-(--mw-component-dialog-footer-background) p-(--mw-component-dialog-padding)"
    >
      <Button
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

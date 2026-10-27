<script lang="ts">
  import { onMount, tick, type Snippet } from "svelte";

  import { strings } from "../i18n";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";

  // The layout of the server and channel settings (DESIGN.md §5.8), over the whole window like the
  // user settings: the navigation on the ground and the page as a sheet beside it, or one after the
  // other on phones, where choosing a section opens its page with a back button. The owner of the
  // sheet decides what may close or leave a page (unsaved changes keep the reader in it).
  interface Props {
    /** The dialog's accessible name: "Server settings, Rust Lyon". */
    label: string;
    /** The navigation page's title on phones. */
    title: string;
    backLabel: string;
    isWide: boolean;
    /** On phones: the page is shown instead of the navigation. */
    isPageShownOnPhone: boolean;
    /** The shown section's title, or null while there is none. */
    pageTitle: string | null;
    /** The navigation item to focus when the sheet opens. */
    initialFocusId: string | null;
    onrequestclose: () => void;
    onback: () => void;
    navigation: Snippet;
    page: Snippet;
    /** Dialogs opened from the sheet. */
    children?: Snippet;
  }

  let {
    label,
    title,
    backLabel,
    isWide,
    isPageShownOnPhone,
    pageTitle,
    initialFocusId,
    onrequestclose,
    onback,
    navigation,
    page,
    children,
  }: Props = $props();

  let dialog: HTMLDialogElement;
  let pageSection = $state<HTMLElement | null>(null);

  const isNavigationShown = $derived(isWide || !isPageShownOnPhone);
  const isPageShown = $derived(isWide || isPageShownOnPhone);

  // On phones the page replaces the list: reading starts at its title.
  $effect(() => {
    if (!isWide && isPageShownOnPhone) {
      void tick().then(() => pageSection?.focus());
    }
  });

  onMount(() => {
    const opener =
      document.activeElement instanceof HTMLElement && document.activeElement !== document.body
        ? document.activeElement
        : null;
    dialog.showModal();
    if (initialFocusId) {
      document.getElementById(initialFocusId)?.focus();
    }
    return () => {
      dialog.close();
      if (opener?.isConnected) {
        opener.focus();
      }
    };
  });
</script>

{#snippet closeButton()}
  <div class="flex shrink-0 flex-col items-center gap-(--mw-component-settings-close-label-gap)">
    <button
      type="button"
      aria-label={strings.settings.closeSettings}
      aria-keyshortcuts="Escape"
      class="group grid size-(--mw-size-hit-web-min) place-items-center rounded-full"
      onclick={onrequestclose}
    >
      <span
        class="grid size-(--mw-component-settings-close-button-size) place-items-center rounded-full border-(length:--mw-component-settings-close-border-width) border-(--mw-component-settings-close-border) text-(--mw-component-settings-close-icon) group-hover:border-(--mw-component-settings-close-border-hover) group-hover:bg-(--mw-component-settings-close-background-hover) group-hover:text-(--mw-component-settings-close-icon-hover)"
      >
        <Icon name="x" class="size-(--mw-component-settings-close-icon-size)" />
      </span>
    </button>
    {#if isWide}
      <span aria-hidden="true" class="type-category text-(--mw-component-settings-close-label-text)">{strings.settings.escapeKey}</span>
    {/if}
  </div>
{/snippet}

<dialog
  bind:this={dialog}
  aria-label={label}
  class={[
    "fixed inset-0 m-0 size-full max-h-none max-w-none font-sans text-text-normal",
    isWide
      ? "bg-(--mw-component-settings-sidebar-background) p-(--mw-size-sheet-inset)"
      : "safe-area-padding bg-(--mw-component-settings-content-background)",
  ]}
  onkeydown={(event) => {
    // Handled here rather than in `cancel`, which browsers stop honouring after repeated Esc. Esc
    // in a dialog opened from here (kick, revoke, crop…) belongs to that dialog.
    if (event.key === "Escape" && event.target instanceof Element && event.target.closest("dialog") === dialog) {
      event.preventDefault();
      onrequestclose();
    }
  }}
  oncancel={(event) => {
    event.preventDefault();
  }}
>
  <div class={["flex h-full", isWide && "gap-(--mw-size-sheet-inset)"]}>
    {#if isNavigationShown}
      <div
        class={[
          "flex justify-end overflow-y-auto bg-(--mw-component-settings-sidebar-background)",
          isWide ? "flex-[1_1_0]" : "flex-1 flex-col justify-start",
        ]}
      >
        {#if !isWide}
          <div class="flex items-center justify-between gap-xs px-(--mw-component-settings-content-padding-inline-narrow) pt-xs">
            <h1 class="min-w-0 truncate type-heading-md text-(--mw-component-settings-page-title-text)">{title}</h1>
            {@render closeButton()}
          </div>
        {/if}
        <nav
          aria-label={title}
          class={[
            "flex flex-col gap-(--mw-component-settings-nav-item-gap)",
            isWide
              ? "w-(--mw-component-settings-nav-width) pt-(--mw-component-settings-nav-padding-top) pr-(--mw-component-settings-nav-padding-trailing) pl-(--mw-component-settings-nav-padding-leading)"
              : "px-(--mw-component-settings-content-padding-inline-narrow) py-xs",
          ]}
        >
          {@render navigation()}
        </nav>
      </div>
    {/if}

    {#if isPageShown}
      <div
        class={[
          "flex min-w-0 flex-col overflow-y-auto bg-(--mw-component-settings-content-background)",
          isWide
            ? "flex-[2_1_0] rounded-(--mw-component-sheet-radius) ps-(--mw-component-settings-content-padding-inline) pe-lg"
            : "flex-1",
        ]}
      >
        {#if !isWide}
          <div class="flex items-center justify-between gap-xs px-(--mw-component-settings-content-padding-inline-narrow) pt-xs">
            <IconButton
              icon="caret-right"
              label={backLabel}
              class="size-(--mw-size-hit-web-min) text-(--mw-component-settings-close-icon) hover:bg-(--mw-component-settings-close-background-hover) hover:text-(--mw-component-settings-close-icon-hover)"
              iconClass="size-(--mw-component-settings-close-icon-size) rotate-180"
              onclick={onback}
            />
            {@render closeButton()}
          </div>
        {/if}
        <div
          class={[
            "flex flex-1 gap-(--mw-component-settings-close-gap)",
            isWide
              ? "pt-(--mw-component-settings-content-padding-top)"
              : "px-(--mw-component-settings-content-padding-inline-narrow) pt-xs",
          ]}
        >
          {#if pageTitle !== null}
            <section
              bind:this={pageSection}
              tabindex="-1"
              aria-label={pageTitle}
              class="flex min-w-0 max-w-(--mw-component-settings-content-max-width) flex-1 flex-col outline-none"
            >
              <h2 class="type-heading-lg text-(--mw-component-settings-page-title-text)">{pageTitle}</h2>
              <div class="mt-(--mw-component-settings-page-title-gap) flex flex-1 flex-col pb-(--mw-component-settings-content-padding-bottom)">
                {@render page()}
              </div>
            </section>
          {/if}
          {#if isWide}
            {@render closeButton()}
          {/if}
        </div>
      </div>
    {/if}
  </div>

  {@render children?.()}
</dialog>

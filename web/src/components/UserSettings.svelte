<script lang="ts">
  import { onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import AccountSettings from "./AccountSettings.svelte";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import NotificationSettings from "./NotificationSettings.svelte";
  import ProfileSettings from "./ProfileSettings.svelte";

  // User settings (DESIGN.md §5.8 layout, §5.9 sections), over the whole window as on desktop: a
  // native modal <dialog> keeps the shell behind it inert. Esc closes the open form first, then the
  // settings; focus goes back to the button that opened them. On phones the navigation is a page of
  // its own, and choosing a section opens it with a back button, as Discord's mobile settings do.
  interface Props {
    client: MellowClient;
    /** Wide enough for the navigation beside the page (the shell's sidebar breakpoint). */
    isWide: boolean;
    onclose: () => void;
  }

  let { client, isWide, onclose }: Props = $props();

  type Section = "account" | "profile" | "notifications" | "about";
  const SECTION_TITLES: Record<Section, string> = {
    account: strings.settings.myAccount,
    profile: strings.settings.profile,
    notifications: strings.settings.notifications,
    about: strings.settings.about,
  };
  const SOURCE_CODE_URL = "https://github.com/Paul-Carouge/mellow";

  const id = $props.id();
  let dialog: HTMLDialogElement;
  let accountSettings = $state<{ closeOpenForm: () => boolean } | null>(null);
  let pageHeading = $state<HTMLElement | null>(null);
  let section = $state<Section>("account");
  let isSectionShownOnPhone = $state(false);

  const isNavigationShown = $derived(isWide || !isSectionShownOnPhone);
  const isPageShown = $derived(isWide || isSectionShownOnPhone);

  function navItemId(target: Section): string {
    return `${id}-nav-${target}`;
  }

  async function choose(target: Section) {
    section = target;
    if (!isWide) {
      isSectionShownOnPhone = true;
      // The page replaces the list: start reading at its title.
      await tick();
      pageHeading?.focus();
    }
  }

  async function backToNavigation() {
    isSectionShownOnPhone = false;
    await tick();
    document.getElementById(navItemId(section))?.focus();
  }

  onMount(() => {
    const opener =
      document.activeElement instanceof HTMLElement && document.activeElement !== document.body
        ? document.activeElement
        : null;
    dialog.showModal();
    // Focus lands on the selected section, so screen readers start in the navigation, as on desktop.
    document.getElementById(navItemId(section))?.focus();
    return () => {
      dialog.close();
      if (opener?.isConnected) {
        opener.focus();
      }
    };
  });
</script>

{#snippet navItem(target: Section)}
  {@const isSelected = section === target}
  <button
    id={navItemId(target)}
    type="button"
    aria-current={isSelected ? "page" : undefined}
    class={[
      "flex h-(--mw-component-settings-nav-item-height) w-full items-center rounded-(--mw-component-settings-nav-item-radius) px-(--mw-component-settings-nav-item-padding-inline) text-start type-channel-name pointer-coarse:h-(--mw-size-hit-web-min)",
      isSelected
        ? "bg-(--mw-component-settings-nav-item-background-selected) text-(--mw-component-settings-nav-item-text-selected)"
        : "text-(--mw-component-settings-nav-item-text) hover:bg-(--mw-component-settings-nav-item-background-hover) hover:text-(--mw-component-settings-nav-item-text-hover)",
    ]}
    onclick={() => void choose(target)}
  >
    <span class="truncate">{SECTION_TITLES[target]}</span>
  </button>
{/snippet}

{#snippet navGroup(label: string, targets: Section[])}
  <h2
    class="px-(--mw-component-settings-nav-section-padding-inline) py-(--mw-component-settings-nav-section-padding-block) type-category text-(--mw-component-settings-nav-section-text)"
  >
    {label}
  </h2>
  {#each targets as target (target)}
    {@render navItem(target)}
  {/each}
{/snippet}

{#snippet separator()}
  <hr
    class="mx-(--mw-component-settings-nav-separator-margin-inline) my-(--mw-component-settings-nav-separator-margin-block) border-0 border-t border-(--mw-component-settings-nav-separator)"
  />
{/snippet}

{#snippet closeButton()}
  <!-- Discord's round close button with "Esc" under it; the 44 px button holds the 36 px circle. -->
  <div class="flex shrink-0 flex-col items-center gap-(--mw-component-settings-close-label-gap)">
    <button
      type="button"
      aria-label={strings.settings.closeSettings}
      aria-keyshortcuts="Escape"
      class="group grid size-(--mw-size-hit-web-min) place-items-center rounded-full"
      onclick={onclose}
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
  aria-label={strings.settings.userSettings}
  class={[
    "fixed inset-0 m-0 size-full max-h-none max-w-none font-sans text-text-normal",
    // Wide: the navigation on the ground and the page as a sheet beside it (ADR-0022 §1).
    isWide
      ? "sheet-inset-padding bg-(--mw-component-settings-sidebar-background)"
      : "safe-area-padding bg-(--mw-component-settings-content-background)",
  ]}
  onkeydown={(event) => {
    // Handled here rather than in `cancel`, which browsers stop honouring after repeated Esc.
    if (event.key === "Escape") {
      event.preventDefault();
      if (!accountSettings?.closeOpenForm()) {
        onclose();
      }
    }
  }}
  oncancel={(event) => {
    event.preventDefault();
    onclose();
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
            <h1 class="type-heading-md text-(--mw-component-settings-page-title-text)">{strings.settings.userSettings}</h1>
            {@render closeButton()}
          </div>
        {/if}
        <nav
          aria-label={strings.settings.userSettings}
          class={[
            "flex flex-col gap-(--mw-component-settings-nav-item-gap)",
            isWide
              ? "w-(--mw-component-settings-nav-width) pt-(--mw-component-settings-nav-padding-top) pr-(--mw-component-settings-nav-padding-trailing) pl-(--mw-component-settings-nav-padding-leading)"
              : "px-(--mw-component-settings-content-padding-inline-narrow) py-xs",
          ]}
        >
          {@render navGroup(strings.settings.userSettings, ["account", "profile"])}
          {@render separator()}
          {@render navGroup(strings.settings.appSettings, ["notifications", "about"])}
          {@render separator()}
          <button
            type="button"
            class="flex h-(--mw-component-settings-nav-item-height) w-full items-center rounded-(--mw-component-settings-nav-item-radius) px-(--mw-component-settings-nav-item-padding-inline) text-start type-channel-name text-(--mw-component-settings-nav-danger-text) hover:bg-(--mw-component-settings-nav-danger-background-hover) hover:text-(--mw-component-settings-nav-danger-text-hover) pointer-coarse:h-(--mw-size-hit-web-min)"
            onclick={() => void client.signOut()}
          >
            {strings.settings.signOut}
          </button>
          {#if client.signOutError}
            <p role="alert" class="px-(--mw-component-settings-nav-item-padding-inline) type-body-sm text-text-danger">
              {client.signOutError}
            </p>
          {/if}
        </nav>
      </div>
    {/if}

    {#if isPageShown}
      <div
        class={[
          "flex min-w-0 flex-col overflow-y-auto bg-(--mw-component-settings-content-background)",
          // The padding counts in the flex share, as in the mockup's settings sheet.
          isWide
            ? "flex-[2_1_0] rounded-(--mw-component-sheet-radius) ps-(--mw-component-settings-content-padding-inline) pe-lg"
            : "flex-1",
        ]}
      >
        {#if !isWide}
          <div class="flex items-center justify-between gap-xs px-(--mw-component-settings-content-padding-inline-narrow) pt-xs">
            <IconButton
              icon="caret-right"
              label={strings.settings.backToUserSettings}
              class="size-(--mw-size-hit-web-min) text-(--mw-component-settings-close-icon) hover:bg-(--mw-component-settings-close-background-hover) hover:text-(--mw-component-settings-close-icon-hover)"
              iconClass="size-(--mw-component-settings-close-icon-size) rotate-180"
              onclick={() => void backToNavigation()}
            />
            {@render closeButton()}
          </div>
        {/if}
        <div
          class={[
            "flex gap-(--mw-component-settings-close-gap) pb-(--mw-component-settings-content-padding-bottom)",
            isWide
              ? "pt-(--mw-component-settings-content-padding-top)"
              : "px-(--mw-component-settings-content-padding-inline-narrow) pt-xs",
          ]}
        >
          <!-- A section, not <main>: the shell behind keeps the page's only main landmark. -->
          <section
            bind:this={pageHeading}
            tabindex="-1"
            aria-label={SECTION_TITLES[section]}
            class="min-w-0 max-w-(--mw-component-settings-content-max-width) flex-1 outline-none"
          >
            {#if section === "account"}
              <AccountSettings bind:this={accountSettings} {client} />
            {:else if section === "profile"}
              <ProfileSettings {client} />
            {:else if section === "notifications"}
              <NotificationSettings />
            {:else}
              <h2 class="type-heading-lg text-(--mw-component-settings-page-title-text)">{strings.settings.about}</h2>
              <p class="mt-(--mw-component-settings-page-title-gap) type-heading-md text-(--mw-component-settings-app-name-text)">
                {strings.settings.webClient}
              </p>
              <h3 class="mt-(--mw-component-settings-section-gap) type-category text-(--mw-component-settings-section-label-text)">
                {strings.settings.sourceCode}
              </h3>
              <p class="mt-(--mw-component-settings-section-label-gap) type-body-lg text-(--mw-component-settings-body-text)">
                {strings.settings.licence}
              </p>
              <a
                href={SOURCE_CODE_URL}
                target="_blank"
                rel="noopener noreferrer"
                class="mt-(--mw-component-settings-action-gap) inline-flex min-h-(--mw-size-hit-web-min) items-center type-body-lg text-text-link hover:underline"
              >
                {strings.settings.sourceCodeOnGitHub}
              </a>
            {/if}
          </section>
          {#if isWide}
            {@render closeButton()}
          {/if}
        </div>
      </div>
    {/if}
  </div>
</dialog>

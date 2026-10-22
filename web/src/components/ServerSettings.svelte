<script lang="ts">
  import { onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { Snowflake } from "../lib/protocol";
  import BanSettings from "./BanSettings.svelte";
  import DeleteServerDialog from "./DeleteServerDialog.svelte";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import InviteSettings from "./InviteSettings.svelte";
  import MemberSettings from "./MemberSettings.svelte";
  import ModerationLogSettings from "./ModerationLogSettings.svelte";
  import RoleSettings from "./RoleSettings.svelte";
  import ServerOverviewSettings from "./ServerOverviewSettings.svelte";
  import { type ServerSettingsSection, isOwner, mustStayOnPage, visibleSections } from "./server-settings";

  // Server settings (DESIGN.md §5.8), over the whole window like the user settings, with the same
  // layout: the navigation on the ground and the page as a sheet beside it, or one after the other
  // on phones. Each section shows only to members holding its permission; the page follows the
  // gateway, so a lost permission or a deleted server closes what can no longer be used. A page
  // with unsaved changes keeps the reader in it: leaving shakes its bar instead.
  interface Props {
    client: MellowClient;
    guildId: Snowflake;
    isWide: boolean;
    onclose: () => void;
  }

  let { client, guildId, isWide, onclose }: Props = $props();

  const MODERATION_SECTIONS: readonly ServerSettingsSection[] = ["members", "bans", "moderation-log"];

  const id = $props.id();
  let dialog: HTMLDialogElement;
  let pageHeading = $state<HTMLElement | null>(null);
  let isSectionShownOnPhone = $state(false);
  let isDeleteDialogOpen = $state(false);
  /** The page holding unsaved changes, if any; see `mustStayOnPage`. */
  let unsavedSection = $state<ServerSettingsSection | null>(null);
  let attention = $state(0);

  const guild = $derived(client.guilds.find((candidate) => candidate.id === guildId) ?? null);
  const sections = $derived(guild ? visibleSections(guild) : []);
  const canDeleteServer = $derived(guild !== null && isOwner(guild, client.user?.id));
  let chosenSection = $state<ServerSettingsSection | null>(null);
  // A section whose permission was lost gives way to the first one still shown.
  const section = $derived(
    chosenSection !== null && sections.includes(chosenSection) ? chosenSection : (sections[0] ?? null),
  );

  const isNavigationShown = $derived(isWide || !isSectionShownOnPhone);
  const isPageShown = $derived(isWide || isSectionShownOnPhone);

  // The server was deleted, or the reader left it or lost every section: nothing is left to show.
  $effect(() => {
    if (guild === null || (sections.length === 0 && !canDeleteServer)) {
      onclose();
    }
  });

  /** Called by a page's effect, and with false when the page goes away. */
  function noteUnsavedChanges(target: ServerSettingsSection, hasChanges: boolean) {
    if (hasChanges) {
      unsavedSection = target;
    } else if (unsavedSection === target) {
      unsavedSection = null;
    }
  }

  function navItemId(target: ServerSettingsSection): string {
    return `${id}-nav-${target}`;
  }

  /** Refused while the page holds unsaved changes: the bar asks to save or reset first. */
  function canLeavePage(): boolean {
    if (mustStayOnPage(unsavedSection, section)) {
      attention += 1;
      return false;
    }
    return true;
  }

  async function choose(target: ServerSettingsSection) {
    if (target !== section && !canLeavePage()) {
      return;
    }
    chosenSection = target;
    attention = 0;
    if (!isWide) {
      isSectionShownOnPhone = true;
      await tick();
      pageHeading?.focus();
    }
  }

  async function backToNavigation() {
    if (!canLeavePage()) {
      return;
    }
    isSectionShownOnPhone = false;
    await tick();
    if (section) {
      document.getElementById(navItemId(section))?.focus();
    }
  }

  function requestClose() {
    if (canLeavePage()) {
      onclose();
    }
  }

  onMount(() => {
    const opener =
      document.activeElement instanceof HTMLElement && document.activeElement !== document.body
        ? document.activeElement
        : null;
    dialog.showModal();
    if (section) {
      document.getElementById(navItemId(section))?.focus();
    }
    return () => {
      dialog.close();
      if (opener?.isConnected) {
        opener.focus();
      }
    };
  });
</script>

{#snippet navItem(target: ServerSettingsSection)}
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
    <span class="truncate">{strings.serverSettings.sections[target]}</span>
  </button>
{/snippet}

{#snippet navGroup(label: string, targets: ServerSettingsSection[])}
  {#if targets.length > 0}
    <h2
      class="truncate px-(--mw-component-settings-nav-section-padding-inline) py-(--mw-component-settings-nav-section-padding-block) type-category text-(--mw-component-settings-nav-section-text)"
    >
      {label}
    </h2>
    {#each targets as target (target)}
      {@render navItem(target)}
    {/each}
  {/if}
{/snippet}

{#snippet separator()}
  <hr
    class="mx-(--mw-component-settings-nav-separator-margin-inline) my-(--mw-component-settings-nav-separator-margin-block) border-0 border-t border-(--mw-component-settings-nav-separator)"
  />
{/snippet}

{#snippet closeButton()}
  <div class="flex shrink-0 flex-col items-center gap-(--mw-component-settings-close-label-gap)">
    <button
      type="button"
      aria-label={strings.settings.closeSettings}
      aria-keyshortcuts="Escape"
      class="group grid size-(--mw-size-hit-web-min) place-items-center rounded-full"
      onclick={requestClose}
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
  aria-label={guild ? `${strings.serverSettings.serverSettings}, ${guild.name}` : strings.serverSettings.serverSettings}
  class={[
    "fixed inset-0 m-0 size-full max-h-none max-w-none font-sans text-text-normal",
    isWide
      ? "bg-(--mw-component-settings-sidebar-background) p-(--mw-size-sheet-inset)"
      : "bg-(--mw-component-settings-content-background) p-0",
  ]}
  onkeydown={(event) => {
    // Handled here rather than in `cancel`, which browsers stop honouring after repeated Esc. Esc
    // in a dialog opened from here (kick, revoke…) belongs to that dialog.
    if (event.key === "Escape" && event.target instanceof Element && event.target.closest("dialog") === dialog) {
      event.preventDefault();
      requestClose();
    }
  }}
  oncancel={(event) => {
    event.preventDefault();
  }}
>
  {#if guild}
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
              <h1 class="min-w-0 truncate type-heading-md text-(--mw-component-settings-page-title-text)">
                {strings.serverSettings.serverSettings}
              </h1>
              {@render closeButton()}
            </div>
          {/if}
          <nav
            aria-label={strings.serverSettings.serverSettings}
            class={[
              "flex flex-col gap-(--mw-component-settings-nav-item-gap)",
              isWide
                ? "w-(--mw-component-settings-nav-width) pt-(--mw-component-settings-nav-padding-top) pr-(--mw-component-settings-nav-padding-trailing) pl-(--mw-component-settings-nav-padding-leading)"
                : "px-(--mw-component-settings-content-padding-inline-narrow) py-xs",
            ]}
          >
            {@render navGroup(
              guild.name,
              sections.filter((candidate) => !MODERATION_SECTIONS.includes(candidate)),
            )}
            {#if sections.some((candidate) => MODERATION_SECTIONS.includes(candidate))}
              {#if sections.some((candidate) => !MODERATION_SECTIONS.includes(candidate))}
                {@render separator()}
              {/if}
              {@render navGroup(
                strings.serverSettings.moderation,
                sections.filter((candidate) => MODERATION_SECTIONS.includes(candidate)),
              )}
            {/if}
            {#if canDeleteServer}
              {@render separator()}
              <button
                type="button"
                aria-haspopup="dialog"
                class="flex h-(--mw-component-settings-nav-item-height) w-full items-center rounded-(--mw-component-settings-nav-item-radius) px-(--mw-component-settings-nav-item-padding-inline) text-start type-channel-name text-(--mw-component-settings-nav-danger-text) hover:bg-(--mw-component-settings-nav-danger-background-hover) hover:text-(--mw-component-settings-nav-danger-text-hover) pointer-coarse:h-(--mw-size-hit-web-min)"
                onclick={() => {
                  if (canLeavePage()) {
                    isDeleteDialogOpen = true;
                  }
                }}
              >
                {strings.serverSettings.deleteServer}
              </button>
            {/if}
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
                label={strings.serverSettings.backToServerSettings}
                class="size-(--mw-size-hit-web-min) text-(--mw-component-settings-close-icon) hover:bg-(--mw-component-settings-close-background-hover) hover:text-(--mw-component-settings-close-icon-hover)"
                iconClass="size-(--mw-component-settings-close-icon-size) rotate-180"
                onclick={() => void backToNavigation()}
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
            {#if section}
              <section
                bind:this={pageHeading}
                tabindex="-1"
                aria-label={strings.serverSettings.sections[section]}
                class="flex min-w-0 max-w-(--mw-component-settings-content-max-width) flex-1 flex-col outline-none"
              >
                <h2 class="type-heading-lg text-(--mw-component-settings-page-title-text)">
                  {strings.serverSettings.sections[section]}
                </h2>
                <div class="mt-(--mw-component-settings-page-title-gap) flex flex-1 flex-col pb-(--mw-component-settings-content-padding-bottom)">
                  {#if section === "overview"}
                    <ServerOverviewSettings
                      {client}
                      {guild}
                      {attention}
                      onunsavedchange={(hasChanges) => noteUnsavedChanges("overview", hasChanges)}
                    />
                  {:else if section === "roles"}
                    <RoleSettings
                      {client}
                      {guild}
                      {attention}
                      onunsavedchange={(hasChanges) => noteUnsavedChanges("roles", hasChanges)}
                    />
                  {:else if section === "invites"}
                    <InviteSettings {client} {guild} />
                  {:else if section === "members"}
                    <MemberSettings {client} {guild} />
                  {:else if section === "bans"}
                    <BanSettings {client} {guild} />
                  {:else}
                    <ModerationLogSettings {client} {guild} />
                  {/if}
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

    {#if isDeleteDialogOpen}
      <DeleteServerDialog {client} {guild} onclose={() => (isDeleteDialogOpen = false)} />
    {/if}
  {/if}
</dialog>

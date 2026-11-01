<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { Snowflake } from "../lib/protocol";
  import BanSettings from "./BanSettings.svelte";
  import DeleteServerDialog from "./DeleteServerDialog.svelte";
  import EmojiSettings from "./EmojiSettings.svelte";
  import InviteSettings from "./InviteSettings.svelte";
  import MemberSettings from "./MemberSettings.svelte";
  import ModerationLogSettings from "./ModerationLogSettings.svelte";
  import RoleSettings from "./RoleSettings.svelte";
  import ServerOverviewSettings from "./ServerOverviewSettings.svelte";
  import SettingsNavItem from "./SettingsNavItem.svelte";
  import SettingsSheet from "./SettingsSheet.svelte";
  import SoundSettings from "./SoundSettings.svelte";
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
    /** Where it opens (the soundboard's "Add sounds"); the first section shown otherwise. */
    initialSection?: ServerSettingsSection | null;
    onclose: () => void;
  }

  let { client, guildId, isWide, initialSection = null, onclose }: Props = $props();

  const MODERATION_SECTIONS: readonly ServerSettingsSection[] = ["members", "bans", "moderation-log"];

  const id = $props.id();
  let isDeleteDialogOpen = $state(false);
  /** The page holding unsaved changes, if any; see `mustStayOnPage`. */
  let unsavedSection = $state<ServerSettingsSection | null>(null);
  let attention = $state(0);

  const guild = $derived(client.guilds.find((candidate) => candidate.id === guildId) ?? null);
  const sections = $derived(guild ? visibleSections(guild) : []);
  const canDeleteServer = $derived(guild !== null && isOwner(guild, client.user?.id));
  // svelte-ignore state_referenced_locally
  let chosenSection = $state<ServerSettingsSection | null>(initialSection);
  // svelte-ignore state_referenced_locally
  let isSectionShownOnPhone = $state(initialSection !== null && !isWide);
  // A section whose permission was lost gives way to the first one still shown.
  const section = $derived(
    chosenSection !== null && sections.includes(chosenSection) ? chosenSection : (sections[0] ?? null),
  );

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

  function choose(target: ServerSettingsSection) {
    if (target !== section && !canLeavePage()) {
      return;
    }
    chosenSection = target;
    attention = 0;
    if (!isWide) {
      isSectionShownOnPhone = true;
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
</script>

{#snippet navGroup(label: string, targets: ServerSettingsSection[])}
  {#if targets.length > 0}
    <h2
      class="truncate px-(--mw-component-settings-nav-section-padding-inline) py-(--mw-component-settings-nav-section-padding-block) type-category text-(--mw-component-settings-nav-section-text)"
    >
      {label}
    </h2>
    {#each targets as target (target)}
      <SettingsNavItem
        id={navItemId(target)}
        label={strings.serverSettings.sections[target]}
        isSelected={section === target}
        onclick={() => choose(target)}
      />
    {/each}
  {/if}
{/snippet}

{#snippet separator()}
  <hr
    class="mx-(--mw-component-settings-nav-separator-margin-inline) my-(--mw-component-settings-nav-separator-margin-block) border-0 border-t border-(--mw-component-settings-nav-separator)"
  />
{/snippet}

{#if guild}
  <SettingsSheet
    label={`${strings.serverSettings.serverSettings}, ${guild.name}`}
    title={strings.serverSettings.serverSettings}
    backLabel={strings.serverSettings.backToServerSettings}
    {isWide}
    isPageShownOnPhone={isSectionShownOnPhone}
    pageTitle={section ? strings.serverSettings.sections[section] : null}
    initialFocusId={section ? navItemId(section) : null}
    onrequestclose={requestClose}
    onback={() => void backToNavigation()}
  >
    {#snippet navigation()}
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
        <SettingsNavItem
          label={strings.serverSettings.deleteServer}
          isDanger
          onclick={() => {
            if (canLeavePage()) {
              isDeleteDialogOpen = true;
            }
          }}
        />
      {/if}
    {/snippet}

    {#snippet page()}
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
      {:else if section === "emoji"}
        <EmojiSettings {client} {guild} />
      {:else if section === "soundboard"}
        <SoundSettings {client} {guild} isCompact={!isWide} />
      {:else if section === "invites"}
        <InviteSettings {client} {guild} />
      {:else if section === "members"}
        <MemberSettings {client} {guild} />
      {:else if section === "bans"}
        <BanSettings {client} {guild} />
      {:else}
        <ModerationLogSettings {client} {guild} />
      {/if}
    {/snippet}

    {#if isDeleteDialogOpen}
      <DeleteServerDialog {client} {guild} onclose={() => (isDeleteDialogOpen = false)} />
    {/if}
  </SettingsSheet>
{/if}

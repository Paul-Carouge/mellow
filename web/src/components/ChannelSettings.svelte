<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { Snowflake } from "../lib/protocol";
  import ChannelOverviewSettings from "./ChannelOverviewSettings.svelte";
  import ChannelPermissionSettings from "./ChannelPermissionSettings.svelte";
  import { type ChannelSettingsSection, canManageChannel, visibleChannelSections } from "./channel-settings";
  import DeleteChannelDialog from "./DeleteChannelDialog.svelte";
  import { mustStayOnPage } from "./server-settings";
  import SettingsNavItem from "./SettingsNavItem.svelte";
  import SettingsSheet from "./SettingsSheet.svelte";

  // Channel settings (DESIGN.md §5.8, Discord's "Edit channel"): Overview (icon, name, topic) and
  // Permissions (overwrites per role and member), in the server settings' sheet, then "Delete
  // channel" with `MANAGE_CHANNELS` in it, as on Discord. Each section shows
  // only with its permission in this channel; the page follows the gateway, so a channel deleted,
  // hidden, or whose permission was lost closes what can no longer be used.
  interface Props {
    client: MellowClient;
    channelId: Snowflake;
    isWide: boolean;
    onclose: () => void;
  }

  let { client, channelId, isWide, onclose }: Props = $props();

  const id = $props.id();
  let isSectionShownOnPhone = $state(false);
  let unsavedSection = $state<ChannelSettingsSection | null>(null);
  let attention = $state(0);

  const guild = $derived(client.guilds.find((candidate) => candidate.channels.some((channel) => channel.id === channelId)) ?? null);
  const channel = $derived(guild?.channels.find((candidate) => candidate.id === channelId) ?? null);
  const channelPermissions = $derived(channel ? client.myChannelPermissions(channel.id) : null);
  const sections = $derived(channel ? visibleChannelSections(channel, channelPermissions) : []);
  let chosenSection = $state<ChannelSettingsSection | null>(null);
  let isDeleteDialogOpen = $state(false);
  const canDeleteChannel = $derived(canManageChannel(channelPermissions));
  // A section whose permission was lost gives way to the first one still shown.
  const section = $derived(
    chosenSection !== null && sections.includes(chosenSection) ? chosenSection : (sections[0] ?? null),
  );

  $effect(() => {
    if (channel === null || sections.length === 0) {
      onclose();
    }
  });

  function noteUnsavedChanges(target: ChannelSettingsSection, hasChanges: boolean) {
    if (hasChanges) {
      unsavedSection = target;
    } else if (unsavedSection === target) {
      unsavedSection = null;
    }
  }

  function navItemId(target: ChannelSettingsSection): string {
    return `${id}-nav-${target}`;
  }

  function canLeavePage(): boolean {
    if (mustStayOnPage(unsavedSection, section)) {
      attention += 1;
      return false;
    }
    return true;
  }

  function choose(target: ChannelSettingsSection) {
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

{#if guild && channel}
  <SettingsSheet
    label={strings.channelSettings.channelSettingsOf(channel.name)}
    title={strings.channelSettings.channelSettings}
    backLabel={strings.channelSettings.backToChannelSettings}
    {isWide}
    isPageShownOnPhone={isSectionShownOnPhone}
    pageTitle={section ? strings.channelSettings.sections[section] : null}
    initialFocusId={section ? navItemId(section) : null}
    onrequestclose={requestClose}
    onback={() => void backToNavigation()}
  >
    {#snippet navigation()}
      <h2
        class="truncate px-(--mw-component-settings-nav-section-padding-inline) py-(--mw-component-settings-nav-section-padding-block) type-category text-(--mw-component-settings-nav-section-text)"
      >
        #{channel.name}
      </h2>
      {#each sections as target (target)}
        <SettingsNavItem
          id={navItemId(target)}
          label={strings.channelSettings.sections[target]}
          isSelected={section === target}
          onclick={() => choose(target)}
        />
      {/each}
      {#if canDeleteChannel}
        <hr
          class="mx-(--mw-component-settings-nav-separator-margin-inline) my-(--mw-component-settings-nav-separator-margin-block) border-0 border-t border-(--mw-component-settings-nav-separator)"
        />
        <SettingsNavItem
          label={strings.channelRemoval.deleteChannel}
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
        <ChannelOverviewSettings
          {client}
          {channel}
          {attention}
          onunsavedchange={(hasChanges) => noteUnsavedChanges("overview", hasChanges)}
        />
      {:else if section === "permissions"}
        <ChannelPermissionSettings
          {client}
          {guild}
          {channel}
          {channelPermissions}
          {attention}
          onunsavedchange={(hasChanges) => noteUnsavedChanges("permissions", hasChanges)}
        />
      {/if}
    {/snippet}

    {#if isDeleteDialogOpen}
      <!-- The sheet closes with the channel; this dialog goes with it. -->
      <DeleteChannelDialog {client} {channel} onclose={() => (isDeleteDialogOpen = false)} />
    {/if}
  </SettingsSheet>
{/if}

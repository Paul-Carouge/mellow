<script lang="ts">
  import { onMount } from "svelte";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Ban, Guild, Snowflake } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { displayName, fullTimestampLabel } from "./format";
  import Icon from "./Icon.svelte";
  import SettingsListStatus from "./SettingsListStatus.svelte";

  // Bans (DESIGN.md §5.8): who is banned, why and since when, newest first, with "Revoke ban".
  // No gateway event tells about bans, so the list is read when the page opens.
  interface Props {
    client: MellowClient;
    guild: Guild;
  }

  let { client, guild }: Props = $props();

  let bans = $state<Ban[] | null>(null);
  let hasFailed = $state(false);
  let busyUserId = $state<Snowflake | null>(null);
  let errorMessage = $state("");
  let notice = $state("");

  async function load() {
    hasFailed = false;
    bans = null;
    try {
      bans = await client.rest.listBans(guild.id);
    } catch (error) {
      console.warn("loading the bans failed", error);
      hasFailed = true;
    }
  }

  onMount(() => {
    void load();
  });

  async function revoke(ban: Ban) {
    if (busyUserId) {
      return;
    }
    const name = displayName(ban.user);
    busyUserId = ban.user.id;
    errorMessage = "";
    try {
      await client.rest.unbanMember(guild.id, ban.user.id);
      bans = bans?.filter((candidate) => candidate.user.id !== ban.user.id) ?? null;
      notice = strings.serverSettings.bans.revoked(name);
    } catch (error) {
      errorMessage = describeError(error, strings.serverSettings.bans.revokeFailed(name));
    } finally {
      busyUserId = null;
    }
  }
</script>

{#if bans === null}
  <SettingsListStatus status={hasFailed ? "failed" : "loading"} failureText={strings.serverSettings.bans.loadFailed} onretry={() => void load()} />
{:else if bans.length === 0}
  <p class="type-body-lg text-(--mw-component-settings-body-text)">{strings.serverSettings.bans.empty(guild.name)}</p>
{:else}
  <div role="alert" class="contents">
    {#if errorMessage}
      <p class="mb-(--mw-component-settings-action-gap) flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-error-text)">
        <Icon name="warning-circle" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
        <span>{errorMessage}</span>
      </p>
    {/if}
  </div>
  <ul aria-label={strings.serverSettings.sections.bans}>
    {#each bans as ban (ban.user.id)}
      {@const name = displayName(ban.user)}
      <li
        class="flex min-h-(--mw-component-server-settings-row-min-height) items-center gap-(--mw-component-server-settings-row-gap) border-b border-(--mw-component-server-settings-row-divider) py-(--mw-component-server-settings-row-padding-block)"
      >
        <Avatar user={ban.user} sizeToken="--mw-component-server-settings-avatar-size" />
        <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-server-settings-text-gap)">
          <p class="flex min-w-0 flex-wrap items-baseline gap-x-(--mw-component-server-settings-row-gap)">
            <span class="truncate type-member-name text-text-heading">{name}</span>
            <span class="truncate type-body-sm text-(--mw-component-server-settings-secondary-text)">@{ban.user.username}</span>
          </p>
          <p class="type-body text-(--mw-component-settings-body-text)">{ban.reason || strings.serverSettings.bans.noReason}</p>
          <p class="type-body-sm text-(--mw-component-server-settings-secondary-text)">
            {strings.serverSettings.bans.bannedOn(fullTimestampLabel(ban.created_at_unix_milliseconds))}
          </p>
        </div>
        <Button
          aria-label={strings.serverSettings.bans.revokeLabel(name)}
          isDisabled={busyUserId !== null}
          onclick={() => void revoke(ban)}
        >
          {busyUserId === ban.user.id ? strings.serverSettings.bans.revoking : strings.serverSettings.bans.revoke}
        </Button>
      </li>
    {/each}
  </ul>
{/if}
<p role="status" class="sr-only">{notice}</p>

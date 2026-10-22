<script lang="ts">
  import { onMount } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import { AUDIT_LOG_PAGE_SIZE, type AuditLogEntry, type Guild, Permission, type PublicUser } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { fullTimestampLabel, unixMillisecondsOfSnowflake } from "./format";
  import Icon from "./Icon.svelte";
  import { auditChangeLines, auditEntrySentence, holds } from "./server-settings";
  import SettingsListStatus from "./SettingsListStatus.svelte";

  // Moderation log (DESIGN.md §5.8): who did what, newest first, one sentence each in the reader's
  // language, with what changed and the ban reason under it. Older pages load on request. Names
  // come from the member list and the bans; anyone else reads as "a former member".
  interface Props {
    client: MellowClient;
    guild: Guild;
  }

  let { client, guild }: Props = $props();

  let entries = $state<AuditLogEntry[] | null>(null);
  let hasFailed = $state(false);
  let hasReachedStart = $state(false);
  let isLoadingOlder = $state(false);
  let hasOlderFailed = $state(false);

  let bannedUsers = $state<PublicUser[]>([]);
  const people = $derived([...(client.membersByGuild.get(guild.id) ?? []).map((member) => member.user), ...bannedUsers]);
  const context = $derived({ guild, people });

  /** Banned people left the member list; moderators who may list bans see them named too. */
  async function loadBannedUsers() {
    if (!holds(guild, Permission.BAN_MEMBERS)) {
      return;
    }
    try {
      bannedUsers = (await client.rest.listBans(guild.id)).map((ban) => ban.user);
    } catch (error) {
      // Only names are missing then: they read as "a former member".
      console.warn("loading the bans for the moderation log failed", error);
    }
  }

  async function loadPage(before?: string): Promise<AuditLogEntry[]> {
    const page = await client.rest.listAuditLog(guild.id, before);
    hasReachedStart = page.length < AUDIT_LOG_PAGE_SIZE;
    return page;
  }

  async function load() {
    hasFailed = false;
    try {
      entries = await loadPage();
    } catch (error) {
      console.warn("loading the moderation log failed", error);
      entries = null;
      hasFailed = true;
    }
  }

  async function loadOlder() {
    const oldest = entries?.at(-1);
    if (!oldest || isLoadingOlder) {
      return;
    }
    isLoadingOlder = true;
    hasOlderFailed = false;
    try {
      const page = await loadPage(oldest.id);
      entries = [...(entries ?? []), ...page];
    } catch (error) {
      console.warn("loading older moderation log entries failed", error);
      hasOlderFailed = true;
    } finally {
      isLoadingOlder = false;
    }
  }

  onMount(() => {
    void load();
    void loadBannedUsers();
  });

  function actorOf(entry: AuditLogEntry): PublicUser | null {
    return people.find((person) => person.id === entry.actor_id) ?? null;
  }
</script>

{#if entries === null}
  <SettingsListStatus status={hasFailed ? "failed" : "loading"} failureText={strings.serverSettings.moderationLog.loadFailed} onretry={() => void load()} />
{:else if entries.length === 0}
  <p class="type-body-lg text-(--mw-component-settings-body-text)">{strings.serverSettings.moderationLog.empty}</p>
{:else}
  <ol aria-label={strings.serverSettings.sections["moderation-log"]} aria-busy={isLoadingOlder}>
    {#each entries as entry (entry.id)}
      {@const actor = actorOf(entry)}
      {@const details = auditChangeLines(entry)}
      <li
        class="flex min-h-(--mw-component-server-settings-row-min-height) items-start gap-(--mw-component-server-settings-row-gap) border-b border-(--mw-component-server-settings-row-divider) py-(--mw-component-server-settings-row-padding-block)"
      >
        {#if actor}
          <Avatar user={actor} sizeToken="--mw-component-server-settings-avatar-size" />
        {:else}
          <span aria-hidden="true" class="size-(--mw-component-server-settings-avatar-size) shrink-0 rounded-full bg-(--mw-component-server-settings-skeleton-fill)"></span>
        {/if}
        <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-server-settings-text-gap)">
          <p class="type-body-lg break-words text-text-heading">{auditEntrySentence(entry, context)}</p>
          {#each details as line, index (index)}
            <p class="type-body break-words text-(--mw-component-settings-body-text)">{line}</p>
          {/each}
          {#if entry.reason}
            <p class="type-body break-words text-(--mw-component-settings-body-text)">
              {strings.serverSettings.moderationLog.reason(entry.reason)}
            </p>
          {/if}
          <p class="type-body-sm text-(--mw-component-server-settings-secondary-text)">
            <time datetime={new Date(unixMillisecondsOfSnowflake(entry.id)).toISOString()}>
              {fullTimestampLabel(unixMillisecondsOfSnowflake(entry.id))}
            </time>
          </p>
        </div>
      </li>
    {/each}
  </ol>
  {#if !hasReachedStart}
    <div class="mt-(--mw-component-server-settings-toolbar-gap) flex flex-col items-start gap-(--mw-component-settings-section-label-gap)">
      <Button isDisabled={isLoadingOlder} onclick={() => void loadOlder()}>
        {isLoadingOlder ? strings.serverSettings.moderationLog.loadingOlder : strings.serverSettings.moderationLog.loadOlder}
      </Button>
      <div role="alert" class="contents">
        {#if hasOlderFailed}
          <p class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-error-text)">
            <Icon name="warning-circle" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
            <span>{strings.serverSettings.moderationLog.loadOlderFailed}</span>
          </p>
        {/if}
      </div>
    </div>
  {/if}
{/if}

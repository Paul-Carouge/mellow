<script lang="ts">
  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import { type Guild, type Member, Permission, type Snowflake } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { displayName } from "./format";
  import ManageMemberDialog from "./ManageMemberDialog.svelte";
  import RoleChips from "./RoleChips.svelte";
  import { canManageRole, holds, matchesMemberSearch, outranksMember, roleManagerOf } from "./server-settings";
  import SettingsListStatus from "./SettingsListStatus.svelte";
  import TextField from "./TextField.svelte";

  // Members (DESIGN.md §5.8): everyone in the server, searchable, with their roles. "Manage" opens
  // what the reader may do to that member: give or take roles, kick, ban. The list is the one the
  // member sheet uses, kept current by the gateway.
  interface Props {
    client: MellowClient;
    guild: Guild;
  }

  let { client, guild }: Props = $props();

  const id = $props.id();
  let query = $state("");
  let managedUserId = $state<Snowflake | null>(null);
  let notice = $state("");

  const members = $derived(client.membersByGuild.get(guild.id));
  const hasFailed = $derived(client.failedMemberGuildIds.has(guild.id));
  const manager = $derived(roleManagerOf(guild, client.user?.id));
  const shownMembers = $derived(
    (members ?? [])
      .filter((member) => matchesMemberSearch(member, query))
      .sort((left, right) => displayName(left.user).localeCompare(displayName(right.user))),
  );
  const managedMember = $derived(members?.find((member) => member.user.id === managedUserId) ?? null);

  function canManage(member: Member): boolean {
    const canModerate =
      outranksMember(guild, manager, member, client.user?.id) &&
      (holds(guild, Permission.KICK_MEMBERS) || holds(guild, Permission.BAN_MEMBERS));
    const canGiveRoles = guild.roles.some((role) => role.id !== guild.id && canManageRole(manager, role, guild.id));
    return canModerate || canGiveRoles;
  }
</script>

<div class="flex flex-col gap-(--mw-component-server-settings-toolbar-gap)">
  <div class="max-w-(--mw-component-dialog-width)">
    <TextField id="{id}-search" type="search" label={strings.serverSettings.members.search} helper={strings.serverSettings.members.searchHelper} autocomplete="off" bind:value={query} />
  </div>

  {#if members === undefined}
    <SettingsListStatus
      status={hasFailed ? "failed" : "loading"}
      failureText={strings.serverSettings.members.loadFailed}
      onretry={() => client.retryLoadingMembers()}
    />
  {:else if shownMembers.length === 0}
    <p class="type-body text-(--mw-component-settings-body-text)">{strings.serverSettings.members.noMatch(query.trim())}</p>
  {:else}
    <ul aria-label={strings.serverSettings.sections.members}>
      {#each shownMembers as member (member.user.id)}
        {@const name = displayName(member.user)}
        <li
          class="flex min-h-(--mw-component-server-settings-row-min-height) items-center gap-(--mw-component-server-settings-row-gap) border-b border-(--mw-component-server-settings-row-divider) py-(--mw-component-server-settings-row-padding-block)"
        >
          <Avatar user={member.user} sizeToken="--mw-component-server-settings-avatar-size" />
          <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-server-settings-text-gap)">
            <p class="flex min-w-0 flex-wrap items-baseline gap-x-(--mw-component-server-settings-row-gap)">
              <span class="truncate type-member-name text-text-heading">{name}</span>
              <span class="truncate type-body-sm text-(--mw-component-server-settings-secondary-text)">@{member.user.username}</span>
            </p>
            <RoleChips {guild} roleIds={member.role_ids} isOwner={member.user.id === guild.owner_id} label={strings.serverSettings.members.memberRoles(name)} />
          </div>
          {#if canManage(member)}
            <Button aria-label={strings.serverSettings.members.manageLabel(name)} onclick={() => (managedUserId = member.user.id)}>
              {strings.serverSettings.members.manage}
            </Button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  <p role="status" class="sr-only">{notice}</p>
</div>

{#if managedMember}
  <ManageMemberDialog
    {client}
    {guild}
    member={managedMember}
    fallbackFocus={() => document.getElementById(`${id}-search`)?.focus()}
    onclose={() => (managedUserId = null)}
    onremoved={(text) => {
      managedUserId = null;
      notice = text;
    }}
  />
{/if}

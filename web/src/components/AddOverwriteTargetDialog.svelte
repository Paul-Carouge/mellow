<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Channel, Guild, Member } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import { type OverwriteTarget, addableMembers, addableRoles } from "./channel-settings";
  import { displayName } from "./format";
  import ModalDialog from "./ModalDialog.svelte";
  import { NO_ROLE_COLOR, type RoleManager, roleColorHex } from "./server-settings";
  import TextField from "./TextField.svelte";

  // Discord's "Add members or roles" (DESIGN.md §5.8): one search over the roles the reader may
  // target and the members without an overwrite here. Choosing one adds it at once, all Inherit.
  interface Props {
    client: MellowClient;
    guild: Guild;
    channel: Channel;
    manager: RoleManager;
    /** Null while the member list loads. */
    members: readonly Member[] | null;
    onclose: () => void;
    onadded: (target: OverwriteTarget) => void;
  }

  let { client, guild, channel, manager, members, onclose, onadded }: Props = $props();

  const id = $props.id();
  const words = strings.channelSettings.permissions;
  let query = $state("");
  let isBusy = $state(false);
  let errorMessage = $state("");

  const roles = $derived(addableRoles(guild, channel, manager, query));
  const people = $derived(members ? addableMembers(channel, members, query) : []);
  const isEmpty = $derived(roles.length === 0 && people.length === 0 && members !== null);

  async function add(target: OverwriteTarget) {
    if (isBusy) {
      return;
    }
    isBusy = true;
    errorMessage = "";
    try {
      await client.setPermissionOverwrite(channel.id, { ...target, allow: "0", deny: "0" });
      onadded(target);
    } catch (error) {
      errorMessage = describeError(error, words.addFailed);
    } finally {
      isBusy = false;
    }
  }

  const rowClass =
    "flex min-h-(--mw-size-hit-web-min) w-full items-center gap-(--mw-component-roles-row-icon-gap) rounded-(--mw-component-roles-row-radius) px-(--mw-component-roles-row-padding-inline) text-start type-channel-name text-(--mw-component-roles-row-text) hover:bg-(--mw-component-roles-row-background-hover) hover:text-(--mw-component-roles-row-text-hover)";
</script>

<ModalDialog title={words.addTitle(channel.name)} subtitle={words.addSubtitle} {errorMessage} {isBusy} {onclose}>
  <TextField
    id="{id}-search"
    type="search"
    label={words.search}
    leadingIcon="magnifying-glass"
    autocomplete="off"
    data-initial-focus=""
    bind:value={query}
  />
  <div aria-busy={isBusy} class="flex max-h-(--mw-component-pins-max-height) flex-col gap-(--mw-component-settings-section-label-gap) overflow-y-auto">
    {#if roles.length > 0}
      <h3 id="{id}-roles" class="type-category text-(--mw-component-settings-section-label-text)">{words.roles}</h3>
      <ul aria-labelledby="{id}-roles">
        {#each roles as role (role.id)}
          <li>
            <button type="button" class={rowClass} onclick={() => void add({ kind: "role", id: role.id })}>
              <span
                aria-hidden="true"
                class={["size-(--mw-component-roles-row-dot-size) shrink-0 rounded-full", role.color === NO_ROLE_COLOR && "bg-(--mw-component-roles-neutral-color)"]}
                style:background-color={role.color === NO_ROLE_COLOR ? undefined : roleColorHex(role.color)}
              ></span>
              <span class="min-w-0 truncate">{role.name}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    {#if members === null}
      <p role="status" class="type-body text-(--mw-component-settings-body-text)">{words.loadingMembers}</p>
    {:else if people.length > 0}
      <h3 id="{id}-members" class="type-category text-(--mw-component-settings-section-label-text)">{words.members}</h3>
      <ul aria-labelledby="{id}-members">
        {#each people as member (member.user.id)}
          <li>
            <button type="button" class={rowClass} onclick={() => void add({ kind: "member", id: member.user.id })}>
              <Avatar user={member.user} sizeToken="--mw-component-channel-settings-target-avatar-size" />
              <span class="min-w-0 truncate">{displayName(member.user)}</span>
              <span class="min-w-0 truncate type-body-sm text-(--mw-component-server-settings-secondary-text)">@{member.user.username}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    {#if isEmpty}
      <p role="status" class="type-body text-(--mw-component-settings-body-text)">
        {query.trim() ? words.noMatch(query.trim()) : words.nothingToAdd}
      </p>
    {/if}
  </div>
</ModalDialog>

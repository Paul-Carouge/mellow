<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { type Guild, MAXIMUM_BAN_REASON_LENGTH, type Member, Permission, type Role } from "../lib/protocol";
  import Button from "./Button.svelte";
  import { displayName } from "./format";
  import Icon from "./Icon.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import { canManageRole, holds, outranksMember, roleColorHex, roleManagerOf, rolesHighestFirst } from "./server-settings";
  import Switch from "./Switch.svelte";
  import TextField from "./TextField.svelte";

  // What a moderator may do to one member (DESIGN.md §5.8, §8): roles as switches, applied at
  // once as on Discord (locked roles say why), then Kick and Ban, each confirmed in its own dialog.
  interface Props {
    client: MellowClient;
    guild: Guild;
    member: Member;
    onclose: () => void;
    /** Where focus goes once the member, and the button that opened this, are gone. */
    fallbackFocus: () => void;
    /** After a kick or a ban, with the sentence to announce. */
    onremoved: (notice: string) => void;
  }

  let { client, guild, member, onclose, fallbackFocus, onremoved }: Props = $props();

  const id = $props.id();
  const name = $derived(displayName(member.user));
  const manager = $derived(roleManagerOf(guild, client.user?.id));
  const namedRoles = $derived(rolesHighestFirst(guild).filter((role) => role.id !== guild.id));
  const canGiveRoles = $derived(namedRoles.some((role) => canManageRole(manager, role, guild.id)));
  const outranks = $derived(outranksMember(guild, manager, member, client.user?.id));
  const canKick = $derived(outranks && holds(guild, Permission.KICK_MEMBERS));
  const canBan = $derived(outranks && holds(guild, Permission.BAN_MEMBERS));

  let busyRoleId = $state<string | null>(null);
  let roleError = $state("");
  let confirmation = $state<"kick" | "ban" | null>(null);
  let banReason = $state("");
  let isRemoving = $state(false);
  let removeError = $state("");

  async function setRole(role: Role, isGiven: boolean) {
    if (busyRoleId) {
      return;
    }
    busyRoleId = role.id;
    roleError = "";
    try {
      await client.setMemberRole(guild.id, member.user.id, role.id, isGiven);
    } catch (error) {
      roleError = describeError(error, strings.serverSettings.members.roleFailed);
    } finally {
      busyRoleId = null;
    }
  }

  async function remove() {
    const action = confirmation;
    if (!action) {
      return;
    }
    isRemoving = true;
    removeError = "";
    // Read before the member leaves the list and this dialog with it.
    const removedName = name;
    try {
      await client.removeMember(guild.id, member.user.id, action === "ban" ? banReason.trim() : null);
      onremoved(
        action === "ban"
          ? strings.serverSettings.members.banned(removedName)
          : strings.serverSettings.members.kicked(removedName),
      );
    } catch (error) {
      removeError = describeError(
        error,
        action === "ban"
          ? strings.serverSettings.members.banFailed(removedName)
          : strings.serverSettings.members.kickFailed(removedName),
      );
      isRemoving = false;
    }
  }
</script>

<ModalDialog title={name} subtitle="@{member.user.username}" cancelLabel={strings.common.close} {onclose} {fallbackFocus}>
  {#if canGiveRoles}
    <section aria-labelledby="{id}-roles" class="flex flex-col">
      <h3 id="{id}-roles" class="type-category text-(--mw-component-settings-section-label-text)">
        {strings.serverSettings.sections.roles}
      </h3>
      <ul aria-busy={busyRoleId !== null}>
        {#each namedRoles as role (role.id)}
          {@const isLocked = !canManageRole(manager, role, guild.id)}
          <li class="flex min-h-(--mw-size-hit-web-min) items-center gap-(--mw-component-roles-row-icon-gap) border-b border-(--mw-component-server-settings-row-divider)">
            <span
              aria-hidden="true"
              class={["size-(--mw-component-roles-row-dot-size) shrink-0 rounded-full", role.color === 0 && "bg-(--mw-component-roles-neutral-color)"]}
              style:background-color={role.color === 0 ? undefined : roleColorHex(role.color)}
            ></span>
            <span id="{id}-role-{role.id}" class="min-w-0 flex-1 truncate type-channel-name text-text-normal">{role.name}</span>
            {#if isLocked}
              <Icon name="lock" class="size-(--mw-component-roles-row-lock-icon-size) text-(--mw-component-roles-row-lock-icon)" />
            {/if}
            <Switch
              isOn={member.role_ids.includes(role.id)}
              isLocked={isLocked || busyRoleId !== null}
              labelledBy="{id}-role-{role.id}"
              describedBy={isLocked ? `${id}-locked` : undefined}
              onchange={(isGiven) => void setRole(role, isGiven)}
            />
          </li>
        {/each}
      </ul>
      {#if namedRoles.some((role) => !canManageRole(manager, role, guild.id))}
        <p id="{id}-locked" class="mt-(--mw-component-settings-section-label-gap) type-body-sm text-(--mw-component-roles-note-text)">
          {strings.serverSettings.roles.locked}
        </p>
      {/if}
      <div role="alert" class="contents">
        {#if roleError}
          <p class="mt-(--mw-component-settings-section-label-gap) type-body text-(--mw-component-settings-error-text)">{roleError}</p>
        {/if}
      </div>
    </section>
  {/if}
  {#if canKick || canBan}
    <div class="flex flex-wrap gap-(--mw-component-dialog-button-gap)">
      {#if canKick}
        <Button variant="danger" aria-haspopup="dialog" onclick={() => (confirmation = "kick")}>
          {strings.serverSettings.members.kickLabel(name)}
        </Button>
      {/if}
      {#if canBan}
        <Button variant="danger" aria-haspopup="dialog" onclick={() => (confirmation = "ban")}>
          {strings.serverSettings.members.banLabel(name)}
        </Button>
      {/if}
    </div>
  {/if}
</ModalDialog>

{#if confirmation === "kick"}
  <ModalDialog
    title={strings.serverSettings.members.kickTitle(name, guild.name)}
    subtitle={strings.serverSettings.members.kickBody(name)}
    errorMessage={removeError}
    isBusy={isRemoving}
    submitLabel={isRemoving ? strings.serverSettings.members.kicking : strings.serverSettings.members.kickLabel(name)}
    isSubmitDanger
    onclose={() => {
      confirmation = null;
      removeError = "";
    }}
    onsubmit={() => void remove()}
  />
{:else if confirmation === "ban"}
  <ModalDialog
    title={strings.serverSettings.members.banTitle(name, guild.name)}
    subtitle={strings.serverSettings.members.banBody(name)}
    errorMessage={removeError}
    isBusy={isRemoving}
    submitLabel={isRemoving ? strings.serverSettings.members.banning : strings.serverSettings.members.banLabel(name)}
    isSubmitDanger
    onclose={() => {
      confirmation = null;
      removeError = "";
    }}
    onsubmit={() => void remove()}
  >
    <TextField
      id="{id}-reason"
      label={strings.serverSettings.members.banReason}
      maxlength={MAXIMUM_BAN_REASON_LENGTH}
      autocomplete="off"
      bind:value={banReason}
    />
  </ModalDialog>
{/if}

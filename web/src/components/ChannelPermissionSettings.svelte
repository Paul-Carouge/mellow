<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Channel, Guild, Member, PermissionBits, Snowflake } from "../lib/protocol";
  import AddOverwriteTargetDialog from "./AddOverwriteTargetDialog.svelte";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import {
    CHANNEL_PERMISSION_GROUPS,
    OVERWRITE_STATES,
    type OverwriteBits,
    type OverwriteEdit,
    type OverwriteState,
    type OverwriteTarget,
    canChangeChannelPermission,
    canEditTarget,
    canRemoveOverwrite,
    overwriteStateOf,
    overwriteTargets,
    rebaseOverwriteEdit,
    storedOverwriteOf,
    withOverwriteState,
  } from "./channel-settings";
  import { displayName } from "./format";
  import Icon, { type IconName } from "./Icon.svelte";
  import { NO_ROLE_COLOR, type PermissionKey, roleColorHex, roleManagerOf } from "./server-settings";
  import UnsavedChangesBar from "./UnsavedChangesBar.svelte";

  // Channel Permissions (DESIGN.md §5.8): the roles and members with an overwrite here on the
  // leading side (`@everyone` always), the chosen one's Deny / Inherit / Allow per channel
  // permission beside it (below it on narrow pages). Adding a role or member and removing one save
  // at once, as on Discord; changes to the states wait for the unsaved-changes bar, rebased on any
  // change made meanwhile (`rebaseOverwriteEdit`). Locked rows and states say why.
  interface Props {
    client: MellowClient;
    guild: Guild;
    channel: Channel;
    /** The reader's permissions in this channel, overwrites included. */
    channelPermissions: PermissionBits | null;
    attention: number;
    onunsavedchange: (hasChanges: boolean) => void;
  }

  let { client, guild, channel, channelPermissions, attention, onunsavedchange }: Props = $props();

  const STATE_ICONS: Record<OverwriteState, IconName> = { deny: "x", inherit: "minus", allow: "check" };
  const STATE_ICON_COLORS: Record<OverwriteState, string> = {
    deny: "text-(--mw-component-channel-settings-override-deny-icon)",
    inherit: "text-(--mw-component-channel-settings-override-inherit-icon)",
    allow: "text-(--mw-component-channel-settings-override-allow-icon)",
  };
  /** Channel words where the server-wide ones would mislead ("View channels" → "View channel"). */
  const CHANNEL_WORDS: Partial<Record<PermissionKey, { label: string; description: string }>> =
    strings.channelSettings.permissions.channelWords;

  const id = $props.id();
  const words = strings.channelSettings.permissions;
  const manager = $derived(roleManagerOf(guild, client.user?.id));
  const targets = $derived(overwriteTargets(guild, channel));
  const members = $derived(client.membersByGuild.get(guild.id));

  let selectedTargetId = $state<Snowflake | null>(null);
  // A removed target gives way to the first one, `@everyone` at worst.
  const selectedTarget = $derived(targets.find((target) => target.id === selectedTargetId) ?? targets[0] ?? null);
  let edit = $state<OverwriteEdit | null>(null);
  const stored = $derived(selectedTarget ? storedOverwriteOf(channel, selectedTarget.id) : null);
  const draft = $derived(
    edit && stored && selectedTarget && edit.target.id === selectedTarget.id
      ? (rebaseOverwriteEdit(edit, stored)?.draft ?? null)
      : null,
  );
  const shown = $derived(draft ?? stored);
  const isEditable = $derived(selectedTarget !== null && canEditTarget(guild, manager, selectedTarget));
  const hasLockedPermissions = $derived(
    CHANNEL_PERMISSION_GROUPS.some((group) =>
      group.permissions.some((permission) => !canChangeChannelPermission(channelPermissions, permission.bit)),
    ),
  );

  let isAwaitingEcho = $state(false);
  const hasChanges = $derived(draft !== null && !isAwaitingEcho);
  let localAttention = $state(0);
  let busy = $state<"save" | "remove" | null>(null);
  let saveError = $state("");
  let errorMessage = $state("");
  let notice = $state("");
  let isAddDialogOpen = $state(false);

  $effect(() => {
    onunsavedchange(hasChanges);
    return () => onunsavedchange(false);
  });

  // Saved (the gateway's echo), undone, or the target left the list: nothing is left to save.
  $effect(() => {
    if (
      edit &&
      (!targets.some((target) => target.id === edit?.target.id) ||
        rebaseOverwriteEdit(edit, storedOverwriteOf(channel, edit.target.id)) === null)
    ) {
      edit = null;
      isAwaitingEcho = false;
    }
  });

  function member(userId: Snowflake): Member | undefined {
    return members?.find((candidate) => candidate.user.id === userId);
  }

  function targetName(target: OverwriteTarget): string {
    if (target.kind === "role") {
      return guild.roles.find((role) => role.id === target.id)?.name ?? strings.serverSettings.moderationLog.deletedRole;
    }
    const found = member(target.id);
    return found ? displayName(found.user) : strings.serverSettings.moderationLog.formerMember;
  }

  function rowId(targetId: Snowflake): string {
    return `${id}-target-${targetId}`;
  }

  function select(target: OverwriteTarget) {
    if (target.id === selectedTarget?.id) {
      return;
    }
    if (hasChanges) {
      localAttention += 1;
      return;
    }
    selectedTargetId = target.id;
    edit = null;
    errorMessage = "";
    notice = "";
  }

  function change(next: OverwriteBits) {
    if (!selectedTarget || !stored || !isEditable) {
      return;
    }
    edit = { target: selectedTarget, base: stored, draft: next };
    isAwaitingEcho = false;
    saveError = "";
    notice = "";
  }

  async function save() {
    if (!selectedTarget || !draft || !hasChanges || busy) {
      return;
    }
    busy = "save";
    saveError = "";
    try {
      await client.setPermissionOverwrite(channel.id, { ...selectedTarget, ...draft });
      // The edit stays until the gateway brings the saved overwrite, which ends it.
      isAwaitingEcho = true;
      notice = strings.serverSettings.unsaved.saved;
    } catch (error) {
      saveError = describeError(error, words.saveFailed);
    } finally {
      busy = null;
    }
  }

  async function remove(target: OverwriteTarget) {
    if (busy) {
      return;
    }
    busy = "remove";
    errorMessage = "";
    try {
      await client.deletePermissionOverwrite(channel.id, target.id);
      edit = null;
      selectedTargetId = null;
      // The row is gone with the gateway's update: the list's first row takes focus.
      document.getElementById(rowId(guild.id))?.focus();
    } catch (error) {
      errorMessage = describeError(error, words.removeFailed);
    } finally {
      busy = null;
    }
  }
</script>

{#snippet targetMark(target: OverwriteTarget)}
  {#if target.kind === "role"}
    {@const color = guild.roles.find((role) => role.id === target.id)?.color ?? NO_ROLE_COLOR}
    <span
      aria-hidden="true"
      class={[
        "grid size-(--mw-component-channel-settings-target-avatar-size) shrink-0 place-items-center",
      ]}
    >
      <span
        class={["size-(--mw-component-roles-row-dot-size) rounded-full", color === NO_ROLE_COLOR && "bg-(--mw-component-roles-neutral-color)"]}
        style:background-color={color === NO_ROLE_COLOR ? undefined : roleColorHex(color)}
      ></span>
    </span>
  {:else}
    {@const found = member(target.id)}
    {#if found}
      <Avatar user={found.user} sizeToken="--mw-component-channel-settings-target-avatar-size" />
    {:else}
      <span aria-hidden="true" class="size-(--mw-component-channel-settings-target-avatar-size) shrink-0 rounded-full bg-(--mw-component-roles-neutral-color)"></span>
    {/if}
  {/if}
{/snippet}

<div class="flex flex-1 flex-col">
  <p class="type-body text-(--mw-component-settings-body-text)">{words.intro(channel.name)}</p>
  <div class="mt-(--mw-component-settings-action-gap)">
    <Button variant="primary" isDisabled={busy !== null} aria-haspopup="dialog" onclick={() => {
      if (hasChanges) {
        localAttention += 1;
      } else {
        isAddDialogOpen = true;
      }
    }}>
      <Icon name="plus" class="me-2xs size-(--mw-component-roles-row-lock-icon-size)" />{words.add}
    </Button>
  </div>
  <div role="alert" class="contents">
    {#if errorMessage}
      <p class="mt-(--mw-component-settings-action-gap) flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-roles-error-text)">
        <Icon name="warning-circle" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
        <span>{errorMessage}</span>
      </p>
    {/if}
  </div>

  <div class="mt-(--mw-component-settings-page-title-gap) flex flex-wrap items-start gap-(--mw-component-roles-list-gap)">
    <div class="flex min-w-0 flex-[1_1_var(--mw-component-roles-list-width)] flex-col gap-(--mw-component-roles-row-gap)">
      <h3 id="{id}-list-title" class="type-category text-(--mw-component-settings-section-label-text)">{words.targets}</h3>
      <ul aria-labelledby="{id}-list-title" class="mt-(--mw-component-settings-section-label-gap) flex flex-col gap-(--mw-component-roles-row-gap)">
        {#each targets as target (target.id)}
          {@const isSelected = target.id === selectedTarget?.id}
          {@const isLocked = !canEditTarget(guild, manager, target)}
          {@const name = targetName(target)}
          <li>
            <button
              id={rowId(target.id)}
              type="button"
              aria-current={isSelected ? "true" : undefined}
              aria-label={isLocked ? words.lockedTargetRow(name) : undefined}
              class={[
                "flex min-h-(--mw-component-roles-row-height) w-full items-center gap-(--mw-component-roles-row-icon-gap) rounded-(--mw-component-roles-row-radius) px-(--mw-component-roles-row-padding-inline) text-start type-channel-name pointer-coarse:min-h-(--mw-size-hit-web-min)",
                isSelected
                  ? "bg-(--mw-component-roles-row-background-selected) text-(--mw-component-roles-row-text-selected)"
                  : "text-(--mw-component-roles-row-text) hover:bg-(--mw-component-roles-row-background-hover) hover:text-(--mw-component-roles-row-text-hover)",
              ]}
              onclick={() => select(target)}
            >
              {@render targetMark(target)}
              <span class="min-w-0 flex-1 truncate">{name}</span>
              {#if isLocked}
                <Icon name="lock" class="size-(--mw-component-roles-row-lock-icon-size) text-(--mw-component-roles-row-lock-icon)" />
              {/if}
            </button>
          </li>
        {/each}
      </ul>
    </div>

    {#if selectedTarget && shown}
      {@const selectedName = targetName(selectedTarget)}
      <div
        class="flex min-w-0 flex-[3_1_var(--mw-component-profile-popover-width)] flex-col gap-(--mw-component-roles-editor-gap)"
        aria-busy={busy !== null}
      >
        <div class="flex flex-wrap items-center gap-(--mw-component-settings-account-button-gap)">
          <h3 class="me-auto flex min-w-0 items-center gap-(--mw-component-roles-row-icon-gap) type-heading-md text-(--mw-component-settings-page-title-text)">
            {@render targetMark(selectedTarget)}
            <span class="truncate">{selectedName}</span>
          </h3>
          {#if canRemoveOverwrite(guild, manager, channelPermissions, channel, selectedTarget)}
            <Button
              variant="ghost"
              aria-label={words.removeLabel(selectedName)}
              isDisabled={busy !== null}
              onclick={() => void remove(selectedTarget)}
            >
              {busy === "remove" ? words.removing : words.remove}
            </Button>
          {/if}
        </div>

        {#if selectedTarget.id === guild.id}
          <p class="type-body text-(--mw-component-settings-body-text)">{words.everyoneNote}</p>
        {/if}
        {#if !isEditable}
          <p class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-roles-note-text)">
            <Icon name="lock" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
            <span>{words.lockedTarget}</span>
          </p>
        {:else if hasLockedPermissions}
          <p id="{id}-locked-permissions" class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-roles-note-text)">
            <Icon name="lock" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
            <span>{words.lockedPermissions}</span>
          </p>
        {/if}

        {#each CHANNEL_PERMISSION_GROUPS as group, groupIndex (group.group)}
          <section aria-labelledby="{id}-group-{group.group}" class={[groupIndex > 0 && "mt-(--mw-component-roles-permission-group-gap)"]}>
            <h4 id="{id}-group-{group.group}" class="type-category text-(--mw-component-settings-section-label-text)">
              {strings.serverSettings.roles.groups[group.group]}
            </h4>
            <ul>
              {#each group.permissions as permission (permission.key)}
                {@const permissionWords = CHANNEL_WORDS[permission.key] ?? strings.serverSettings.permissions[permission.key]}
                {@const isPermissionLocked = !isEditable || !canChangeChannelPermission(channelPermissions, permission.bit)}
                {@const current = overwriteStateOf(shown, permission.bit)}
                <li class="flex flex-wrap items-center gap-x-(--mw-component-roles-permission-gap) gap-y-(--mw-component-settings-section-label-gap) border-b border-(--mw-component-roles-permission-divider) py-(--mw-component-roles-permission-padding-block)">
                  <div class="min-w-0 flex-[1_1_var(--mw-component-roles-list-width)]">
                    <p id="{id}-permission-{permission.key}" class="type-channel-name text-(--mw-component-roles-permission-label-text)">
                      {permissionWords.label}
                    </p>
                    <p id="{id}-permission-{permission.key}-description" class="type-body text-(--mw-component-roles-permission-description-text)">
                      {permissionWords.description}
                    </p>
                  </div>
                  <!-- Native radios: arrow keys move between the three, the group is read with its permission. -->
                  <div
                    role="radiogroup"
                    aria-labelledby="{id}-permission-{permission.key}"
                    aria-describedby={[
                      `${id}-permission-${permission.key}-description`,
                      isEditable && isPermissionLocked ? `${id}-locked-permissions` : "",
                    ].filter(Boolean).join(" ")}
                    class={[
                      "flex shrink-0 overflow-hidden rounded-(--mw-component-channel-settings-override-radius) border-(length:--mw-component-channel-settings-override-border-width) border-(--mw-component-channel-settings-override-border)",
                      isPermissionLocked && "opacity-(--mw-component-switch-disabled-opacity)",
                    ]}
                  >
                    {#each OVERWRITE_STATES as state (state)}
                      {@const isChosen = current === state}
                      <label
                        class={[
                          "flex h-(--mw-component-channel-settings-override-height) items-center gap-(--mw-component-channel-settings-override-icon-gap) px-(--mw-component-channel-settings-override-padding-inline) type-body-sm transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) has-[:focus-visible]:focus-ring-inset pointer-coarse:h-(--mw-size-hit-web-min)",
                          isChosen
                            ? "bg-(--mw-component-channel-settings-override-background-selected) font-semibold text-(--mw-component-channel-settings-override-text-selected)"
                            : "text-(--mw-component-channel-settings-override-text)",
                          !isChosen && !isPermissionLocked && "cursor-pointer hover:bg-(--mw-component-channel-settings-override-background-hover) hover:text-(--mw-component-channel-settings-override-text-hover)",
                        ]}
                      >
                        <input
                          type="radio"
                          class="sr-only"
                          name="{id}-{permission.key}"
                          value={state}
                          checked={isChosen}
                          disabled={isPermissionLocked}
                          onchange={() => change(withOverwriteState(shown, permission.bit, state))}
                        />
                        <Icon
                          name={STATE_ICONS[state]}
                          class={["size-(--mw-component-channel-settings-override-icon-size)", isChosen && STATE_ICON_COLORS[state]].filter(Boolean).join(" ")}
                        />
                        {words.states[state]}
                      </label>
                    {/each}
                  </div>
                </li>
              {/each}
            </ul>
          </section>
        {/each}
      </div>
    {/if}
  </div>

  <p role="status" class="sr-only">{notice}</p>
  <div class="flex-1"></div>
  {#if hasChanges}
    <UnsavedChangesBar
      attention={attention + localAttention}
      isSaving={busy === "save"}
      errorMessage={saveError}
      onreset={() => {
        edit = null;
        isAwaitingEcho = false;
        saveError = "";
      }}
      onsave={() => void save()}
    />
  {/if}
</div>

{#if isAddDialogOpen}
  <AddOverwriteTargetDialog
    {client}
    {guild}
    {channel}
    {manager}
    members={members ?? null}
    onclose={() => (isAddDialogOpen = false)}
    onadded={(target) => {
      selectedTargetId = target.id;
      isAddDialogOpen = false;
    }}
  />
{/if}

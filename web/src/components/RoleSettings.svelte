<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import { prefersDarkColorScheme } from "../lib/color-scheme.svelte";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { type Guild, MAXIMUM_ROLE_NAME_LENGTH, type Role, type Snowflake } from "../lib/protocol";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import { readableRoleColor } from "./role-colors";
  import {
    type MoveDirection,
    NO_ROLE_COLOR,
    PERMISSION_GROUPS,
    ROLE_COLOR_PRESETS,
    type RoleDraft,
    type RoleEdit,
    canManageRole,
    canTogglePermission,
    draftOf,
    rebaseRoleEdit,
    roleColorHex,
    roleIdsAfterMove,
    roleManagerOf,
    roleMemberCount,
    roleUpdateBody,
    rolesHighestFirst,
    togglePermission,
  } from "./server-settings";
  import Switch from "./Switch.svelte";
  import TextField from "./TextField.svelte";
  import { colorTokenValue } from "./tokens";
  import UnsavedChangesBar from "./UnsavedChangesBar.svelte";

  // Roles (DESIGN.md §5.8): the hierarchy on the leading side, the selected role's editor beside
  // it (below it on narrow pages). Roles above the reader's highest are shown but locked, with the
  // reason. Changes wait in a draft for the unsaved-changes bar; a change made meanwhile by someone
  // else is kept under the draft (`rebaseRoleEdit`). Moving a role saves at once, as on Discord.
  interface Props {
    client: MellowClient;
    guild: Guild;
    attention: number;
    hasUnsavedChanges: boolean;
  }

  let { client, guild, attention, hasUnsavedChanges = $bindable() }: Props = $props();

  type Tab = "display" | "permissions";

  const id = $props.id();
  const manager = $derived(roleManagerOf(guild, client.user?.id));
  const roles = $derived(rolesHighestFirst(guild));
  const members = $derived(client.membersByGuild.get(guild.id) ?? []);

  let selectedRoleId = $state<Snowflake | null>(null);
  // A deleted role gives way to the highest one the reader can edit.
  const selectedRole = $derived(
    roles.find((role) => role.id === selectedRoleId) ??
      roles.find((role) => canManageRole(manager, role, guild.id)) ??
      roles[0] ??
      null,
  );
  let edit = $state<RoleEdit | null>(null);
  const draft = $derived(
    edit && selectedRole && edit.roleId === selectedRole.id ? (rebaseRoleEdit(edit, selectedRole)?.draft ?? null) : null,
  );
  const shown = $derived(draft ?? (selectedRole ? draftOf(selectedRole) : null));
  const isEveryone = $derived(selectedRole?.id === guild.id);
  const isEditable = $derived(selectedRole !== null && canManageRole(manager, selectedRole, guild.id));
  const hasLockedPermissions = $derived(
    PERMISSION_GROUPS.some((group) => group.permissions.some((permission) => !canTogglePermission(manager, permission.bit))),
  );

  let tab = $state<Tab>("display");
  const shownTab = $derived<Tab>(isEveryone ? "permissions" : tab);
  let localAttention = $state(0);
  let busy = $state<"create" | "save" | "move" | "delete" | null>(null);
  let errorMessage = $state("");
  let saveError = $state("");
  let deleteError = $state("");
  let notice = $state("");
  let isDeleteDialogOpen = $state(false);
  let nameField = $state<HTMLInputElement | null>(null);

  const previewColor = $derived.by(() => {
    void prefersDarkColorScheme.current;
    return shown && shown.color !== NO_ROLE_COLOR ? readableRoleColor(shown.color, colorTokenValue("color-surface-chat")) : null;
  });

  $effect(() => {
    hasUnsavedChanges = draft !== null;
  });

  function rowId(roleId: Snowflake): string {
    return `${id}-role-${roleId}`;
  }

  function change(next: RoleDraft) {
    if (!selectedRole || !isEditable) {
      return;
    }
    edit = { roleId: selectedRole.id, base: draftOf(selectedRole), draft: next };
    saveError = "";
    notice = "";
  }

  function select(role: Role) {
    if (role.id === selectedRole?.id) {
      return;
    }
    if (draft) {
      localAttention += 1;
      return;
    }
    selectedRoleId = role.id;
    edit = null;
    errorMessage = "";
    notice = "";
  }

  async function createRole() {
    if (busy || draft) {
      if (draft) {
        localAttention += 1;
      }
      return;
    }
    busy = "create";
    errorMessage = "";
    try {
      const role = await client.createRole(guild.id, strings.serverSettings.roles.newRoleName);
      selectedRoleId = role.id;
      edit = null;
      tab = "display";
      await tick();
      nameField?.focus();
      nameField?.select();
    } catch (error) {
      errorMessage = describeError(error, strings.serverSettings.roles.createFailed);
    } finally {
      busy = null;
    }
  }

  async function save() {
    if (!selectedRole || !draft || busy) {
      return;
    }
    const body = roleUpdateBody(selectedRole, draft, isEveryone);
    if (body === null) {
      saveError = strings.serverSettings.roles.nameRequired;
      return;
    }
    busy = "save";
    saveError = "";
    try {
      await client.updateRole(guild.id, selectedRole.id, body);
      edit = null;
      notice = strings.serverSettings.unsaved.saved;
    } catch (error) {
      saveError = describeError(error, strings.serverSettings.roles.saveFailed);
    } finally {
      busy = null;
    }
  }

  async function move(role: Role, direction: MoveDirection) {
    const roleIds = roleIdsAfterMove(guild, manager, role.id, direction);
    if (!roleIds || busy) {
      return;
    }
    busy = "move";
    errorMessage = "";
    try {
      await client.reorderRoles(guild.id, roleIds);
    } catch (error) {
      errorMessage = describeError(error, strings.serverSettings.roles.moveFailed);
    } finally {
      busy = null;
    }
  }

  async function deleteRole() {
    if (!selectedRole) {
      return;
    }
    busy = "delete";
    deleteError = "";
    try {
      await client.deleteRole(guild.id, selectedRole.id);
      edit = null;
      selectedRoleId = null;
      isDeleteDialogOpen = false;
    } catch (error) {
      deleteError = describeError(error, strings.serverSettings.roles.deleteFailed);
    } finally {
      busy = null;
    }
  }

  function chooseTabWithArrows(event: KeyboardEvent) {
    if (event.key === "ArrowLeft" || event.key === "ArrowRight" || event.key === "Home" || event.key === "End") {
      event.preventDefault();
      tab = tab === "display" ? "permissions" : "display";
      document.getElementById(`${id}-tab-${tab}`)?.focus();
    }
  }
</script>

{#snippet roleDot(color: number)}
  <span
    aria-hidden="true"
    class={[
      "size-(--mw-component-roles-row-dot-size) shrink-0 rounded-full",
      color === NO_ROLE_COLOR && "bg-(--mw-component-roles-neutral-color)",
    ]}
    style:background-color={color === NO_ROLE_COLOR ? undefined : roleColorHex(color)}
  ></span>
{/snippet}

{#snippet swatch(color: number)}
  {@const isChosen = shown?.color === color}
  <!-- Inside its <label>: the radio carries the name, the tile shows the colour. -->
  <span class="grid size-(--mw-size-hit-web-min) shrink-0 place-items-center">
    <input
      type="radio"
      name="{id}-color"
      class="sr-only"
      value={color}
      checked={isChosen}
      aria-label={color === NO_ROLE_COLOR ? strings.serverSettings.roles.noColor : strings.serverSettings.roles.colorOption(roleColorHex(color))}
      onchange={() => shown && change({ ...shown, color })}
    />
    <span
      aria-hidden="true"
      class={[
        "grid size-(--mw-component-roles-swatch-size) place-items-center rounded-(--mw-component-roles-swatch-radius)",
        color === NO_ROLE_COLOR && "bg-(--mw-component-roles-neutral-color)",
        isChosen &&
          "outline-(length:--mw-component-roles-swatch-ring-width) outline-offset-(--mw-component-roles-swatch-ring-offset) outline-(--mw-component-roles-swatch-ring) outline-solid",
      ]}
      style:background-color={color === NO_ROLE_COLOR ? undefined : roleColorHex(color)}
    >
      {#if isChosen}
        <span class="grid size-(--mw-component-roles-swatch-check-plate-size) place-items-center rounded-full bg-(--mw-component-roles-swatch-check-plate) text-(--mw-component-roles-swatch-check)">
          <Icon name="check" class="size-(--mw-component-roles-swatch-check-size)" />
        </span>
      {/if}
    </span>
  </span>
{/snippet}

<div class="flex flex-1 flex-col">
  <div class="flex flex-wrap items-center gap-(--mw-component-settings-action-gap)">
    <Button variant="primary" isDisabled={busy !== null} onclick={() => void createRole()}>
      {busy === "create" ? strings.serverSettings.roles.creating : strings.serverSettings.roles.createRole}
    </Button>
    {#if roles.length === 1}
      <p class="type-body text-(--mw-component-settings-body-text)">{strings.serverSettings.roles.empty}</p>
    {/if}
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
    <!-- The hierarchy: Ctrl+Shift+Up/Down moves the focused role, as on desktop. -->
    <div class="flex min-w-0 flex-[1_1_var(--mw-component-roles-list-width)] flex-col gap-(--mw-component-roles-row-gap)">
      <h3 id="{id}-list-title" class="sr-only">{strings.serverSettings.roles.roleList}</h3>
      <ul aria-labelledby="{id}-list-title" aria-describedby="{id}-move-hint" class="flex flex-col gap-(--mw-component-roles-row-gap)">
        {#each roles as role (role.id)}
          {@const isSelected = role.id === selectedRole?.id}
          {@const isLocked = !canManageRole(manager, role, guild.id)}
          <li>
            <button
              id={rowId(role.id)}
              type="button"
              aria-current={isSelected ? "true" : undefined}
              aria-label={isLocked ? strings.serverSettings.roles.lockedRole(role.name) : undefined}
              aria-describedby="{id}-count-{role.id}"
              class={[
                "flex min-h-(--mw-component-roles-row-height) w-full items-center gap-(--mw-component-roles-row-icon-gap) rounded-(--mw-component-roles-row-radius) px-(--mw-component-roles-row-padding-inline) text-start type-channel-name pointer-coarse:min-h-(--mw-size-hit-web-min)",
                isSelected
                  ? "bg-(--mw-component-roles-row-background-selected) text-(--mw-component-roles-row-text-selected)"
                  : "text-(--mw-component-roles-row-text) hover:bg-(--mw-component-roles-row-background-hover) hover:text-(--mw-component-roles-row-text-hover)",
              ]}
              onclick={() => select(role)}
              onkeydown={(event) => {
                if ((event.ctrlKey || event.metaKey) && event.shiftKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
                  event.preventDefault();
                  void move(role, event.key === "ArrowUp" ? "up" : "down").then(() =>
                    document.getElementById(rowId(role.id))?.focus(),
                  );
                }
              }}
            >
              {@render roleDot(role.color)}
              <span class="min-w-0 flex-1 truncate">{role.name}</span>
              <span
                id="{id}-count-{role.id}"
                class={[
                  "shrink-0 type-body-sm",
                  isSelected ? "text-(--mw-component-roles-row-count-text-selected)" : "text-(--mw-component-roles-row-count-text)",
                ]}
              >
                {strings.serverSettings.roles.memberCount(roleMemberCount(guild, role, members))}
              </span>
              {#if isLocked}
                <Icon name="lock" class="size-(--mw-component-roles-row-lock-icon-size) text-(--mw-component-roles-row-lock-icon)" />
              {/if}
            </button>
          </li>
        {/each}
      </ul>
      <p id="{id}-move-hint" class="mt-(--mw-component-settings-section-label-gap) type-body-sm text-(--mw-component-roles-note-text)">
        {strings.serverSettings.roles.moveHint}
      </p>
    </div>

    {#if selectedRole && shown}
      <div
        class="flex min-w-0 flex-[3_1_var(--mw-component-profile-popover-width)] flex-col gap-(--mw-component-roles-editor-gap)"
        aria-busy={busy !== null}
      >
        <div class="flex flex-wrap items-center gap-(--mw-component-settings-account-button-gap)">
          <h3 class="me-auto flex min-w-0 items-center gap-(--mw-component-roles-row-icon-gap) type-heading-md text-(--mw-component-settings-page-title-text)">
            {@render roleDot(shown.color)}
            <span class="truncate">{selectedRole.name}</span>
          </h3>
          {#if !isEveryone}
            {@const canMoveUp = roleIdsAfterMove(guild, manager, selectedRole.id, "up") !== null}
            {@const canMoveDown = roleIdsAfterMove(guild, manager, selectedRole.id, "down") !== null}
            <Button variant="ghost" isDisabled={!canMoveUp || busy !== null} aria-keyshortcuts="Control+Shift+ArrowUp" onclick={() => void move(selectedRole, "up")}>
              <Icon name="caret-up" class="me-2xs size-(--mw-component-roles-row-lock-icon-size)" />{strings.serverSettings.roles.moveUp}
            </Button>
            <Button variant="ghost" isDisabled={!canMoveDown || busy !== null} aria-keyshortcuts="Control+Shift+ArrowDown" onclick={() => void move(selectedRole, "down")}>
              <Icon name="caret-down" class="me-2xs size-(--mw-component-roles-row-lock-icon-size)" />{strings.serverSettings.roles.moveDown}
            </Button>
          {/if}
        </div>

        {#if !isEditable}
          <p class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-roles-note-text)">
            <Icon name="lock" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
            <span>{strings.serverSettings.roles.locked}</span>
          </p>
        {/if}

        {#if isEveryone}
          <p class="type-body text-(--mw-component-settings-body-text)">{strings.serverSettings.roles.everyoneNote}</p>
        {:else}
          <div role="tablist" aria-label={selectedRole.name} class="flex gap-(--mw-component-tabs-gap) border-b border-(--mw-component-tabs-divider)">
            {#each ["display", "permissions"] as const as target (target)}
              {@const isSelected = shownTab === target}
              <button
                id="{id}-tab-{target}"
                type="button"
                role="tab"
                aria-selected={isSelected}
                aria-controls="{id}-panel"
                tabindex={isSelected ? 0 : -1}
                class={[
                  "-mb-px flex h-(--mw-component-tabs-height) items-center border-b-(length:--mw-component-tabs-indicator-height) type-button pointer-coarse:h-(--mw-size-hit-web-min)",
                  isSelected
                    ? "border-(--mw-component-tabs-indicator) text-(--mw-component-tabs-text-selected)"
                    : "border-transparent text-(--mw-component-tabs-text) hover:text-(--mw-component-tabs-text-hover)",
                ]}
                onclick={() => (tab = target)}
                onkeydown={chooseTabWithArrows}
              >
                {strings.serverSettings.roles[target]}
              </button>
            {/each}
          </div>
        {/if}

        <div id="{id}-panel" role={isEveryone ? undefined : "tabpanel"} aria-labelledby={isEveryone ? undefined : `${id}-tab-${shownTab}`} class="flex flex-col gap-(--mw-component-roles-editor-gap)">
          {#if shownTab === "display"}
            <div class="max-w-(--mw-component-dialog-width)">
              <TextField
                id="{id}-name"
                label={strings.serverSettings.roles.roleName}
                maxlength={MAXIMUM_ROLE_NAME_LENGTH}
                autocomplete="off"
                readonly={!isEditable}
                bind:element={nameField}
                bind:value={() => shown.name, (name) => change({ ...shown, name })}
              />
            </div>

            <fieldset class="flex min-w-0 flex-col gap-(--mw-component-settings-section-label-gap)" disabled={!isEditable}>
              <legend class="type-category text-(--mw-component-settings-section-label-text)">{strings.serverSettings.roles.color}</legend>
              <!-- "No colour" keeps its name in view; the presets fill rows of up to ten. -->
              <label class="flex w-fit cursor-pointer items-center gap-(--mw-component-roles-swatch-gap) rounded-(--mw-component-roles-swatch-radius) pe-(--mw-component-roles-swatch-gap) has-[:focus-visible]:focus-ring">
                {@render swatch(NO_ROLE_COLOR)}
                <span class="type-body text-(--mw-component-settings-body-text)">{strings.serverSettings.roles.noColor}</span>
              </label>
              <div class="grid max-w-[calc(10*var(--mw-size-hit-web-min))] grid-cols-[repeat(auto-fill,var(--mw-size-hit-web-min))]">
                {#each ROLE_COLOR_PRESETS as color (color)}
                  <label
                    class="grid cursor-pointer place-items-center rounded-(--mw-component-roles-swatch-radius) has-[:focus-visible]:focus-ring"
                    title={roleColorHex(color)}
                  >
                    {@render swatch(color)}
                  </label>
                {/each}
              </div>
              <p class="type-body text-(--mw-component-roles-note-text)">
                {strings.serverSettings.roles.colorPreview}
                <span class="type-member-name text-text-heading" style:color={previewColor}>{shown.name.trim() || selectedRole.name}</span>
              </p>
            </fieldset>

            {#if isEditable}
              <div>
                <Button variant="danger" isDisabled={busy !== null} onclick={() => (isDeleteDialogOpen = true)}>
                  {strings.serverSettings.roles.deleteRole}
                </Button>
              </div>
            {/if}
          {:else}
            {#if isEditable && hasLockedPermissions}
              <p id="{id}-locked-permissions" class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-roles-note-text)">
                <Icon name="lock" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
                <span>{strings.serverSettings.roles.permissionLocked}</span>
              </p>
            {/if}
            {#each PERMISSION_GROUPS as group, groupIndex (group.group)}
              <section aria-labelledby="{id}-group-{group.group}" class={[groupIndex > 0 && "mt-(--mw-component-roles-permission-group-gap)"]}>
                <h4 id="{id}-group-{group.group}" class="type-category text-(--mw-component-settings-section-label-text)">
                  {strings.serverSettings.roles.groups[group.group]}
                </h4>
                <ul>
                  {#each group.permissions as permission (permission.key)}
                    {@const words = strings.serverSettings.permissions[permission.key]}
                    {@const isPermissionLocked = !isEditable || !canTogglePermission(manager, permission.bit)}
                    <li class="flex items-center gap-(--mw-component-roles-permission-gap) border-b border-(--mw-component-roles-permission-divider) py-(--mw-component-roles-permission-padding-block)">
                      <div class="min-w-0 flex-1">
                        <p id="{id}-permission-{permission.key}" class="flex flex-wrap items-center gap-x-(--mw-component-roles-sensitive-gap) type-channel-name text-(--mw-component-roles-permission-label-text)">
                          {words.label}
                          {#if permission.isSensitive}
                            <span class="inline-flex items-center gap-(--mw-component-roles-sensitive-gap) type-body-sm text-(--mw-component-roles-sensitive-text)">
                              <Icon name="warning-circle" class="size-(--mw-component-roles-sensitive-icon-size)" />
                              {strings.serverSettings.roles.sensitive}
                            </span>
                          {/if}
                        </p>
                        <p id="{id}-permission-{permission.key}-description" class="type-body text-(--mw-component-roles-permission-description-text)">
                          {words.description}
                        </p>
                      </div>
                      <Switch
                        isOn={(BigInt(shown.permissions) & permission.bit) === permission.bit}
                        isLocked={isPermissionLocked}
                        labelledBy="{id}-permission-{permission.key}"
                        describedBy={[
                          `${id}-permission-${permission.key}-description`,
                          isEditable && isPermissionLocked ? `${id}-locked-permissions` : "",
                        ].filter(Boolean).join(" ")}
                        onchange={() => change(togglePermission(shown, permission.bit))}
                      />
                    </li>
                  {/each}
                </ul>
              </section>
            {/each}
          {/if}
        </div>
      </div>
    {/if}
  </div>

  <p role="status" class="sr-only">{notice}</p>
  <div class="flex-1"></div>
  {#if draft}
    <UnsavedChangesBar
      attention={attention + localAttention}
      isSaving={busy === "save"}
      errorMessage={saveError}
      onreset={() => {
        edit = null;
        saveError = "";
      }}
      onsave={() => void save()}
    />
  {/if}
</div>

{#if isDeleteDialogOpen && selectedRole}
  <ModalDialog
    title={strings.serverSettings.roles.deleteTitle(selectedRole.name)}
    subtitle={strings.serverSettings.roles.deleteBody(selectedRole.name)}
    errorMessage={deleteError}
    isBusy={busy === "delete"}
    submitLabel={busy === "delete" ? strings.serverSettings.roles.deleting : strings.serverSettings.roles.deleteRole}
    isSubmitDanger
    onclose={() => {
      isDeleteDialogOpen = false;
      deleteError = "";
    }}
    onsubmit={() => void deleteRole()}
  />
{/if}

<script lang="ts">
  import { strings } from "../i18n";
  import type { Guild, Role, Snowflake } from "../lib/protocol";
  import Icon from "./Icon.svelte";
  import RoleIcon from "./RoleIcon.svelte";
  import { hasRoleIcon } from "./role-icons";
  import { roleColorHex } from "./server-settings";

  // A member's roles as small chips, highest first, each with its colour dot and a quiet wash of
  // that colour (decorative: the name carries the meaning). The owner gets an "Owner" chip with a
  // crown before them. In a profile (DESIGN.md §5.7.1) a role's icon takes its dot's place.
  interface Props {
    guild: Guild;
    roleIds: readonly Snowflake[];
    isOwner: boolean;
    label: string;
    /** A role's icon (ADR-0050) instead of its colour dot, where it has one. */
    areIconsShown?: boolean;
    /** At most this many chips (the owner's counts), the rest counted in a last "+2" chip. */
    limit?: number;
  }

  let { guild, roleIds, isOwner, label, areIconsShown = false, limit = Number.POSITIVE_INFINITY }: Props = $props();

  // `@everyone` (the guild's own id) is every member's: never a chip.
  const roles = $derived(
    guild.roles
      .filter((role) => role.id !== guild.id && roleIds.includes(role.id))
      .sort((left, right) => right.position - left.position),
  );
  const shownRoles = $derived(roles.slice(0, Math.max(0, limit - (isOwner ? 1 : 0))));
  const hiddenCount = $derived(roles.length - shownRoles.length);
</script>

{#snippet chip(name: string, color: number | null, role: Role | null)}
  {@const isTinted = color !== null && color !== 0}
  <li
    class={[
      "inline-flex h-(--mw-component-server-settings-chip-height) max-w-full items-center gap-(--mw-component-server-settings-chip-gap) rounded-(--mw-component-server-settings-chip-radius) bg-(--mw-component-server-settings-chip-background) px-(--mw-component-server-settings-chip-padding-inline) type-body-sm text-(--mw-component-server-settings-chip-text)",
      isTinted && "role-chip-tinted",
    ]}
    style:--role-color={isTinted ? roleColorHex(color) : undefined}
  >
    {#if areIconsShown && role && hasRoleIcon(role)}
      <RoleIcon {role} sizeToken="--mw-component-server-settings-chip-icon-size" isDecorative />
    {:else if color === null}
      <Icon name="crown" class="size-(--mw-component-server-settings-chip-icon-size) text-(--mw-component-server-settings-chip-owner-icon)" />
    {:else}
      <span
        aria-hidden="true"
        class={["size-(--mw-component-server-settings-chip-dot-size) shrink-0 rounded-full", color === 0 && "bg-(--mw-component-roles-neutral-color)"]}
        style:background-color={color === 0 ? undefined : roleColorHex(color)}
      ></span>
    {/if}
    <span class="truncate">{name}</span>
  </li>
{/snippet}

{#if isOwner || roles.length > 0}
  <ul aria-label={label} class="flex flex-wrap gap-(--mw-component-server-settings-chip-gap)">
    {#if isOwner}
      {@render chip(strings.serverSettings.members.owner, null, null)}
    {/if}
    {#each shownRoles as role (role.id)}
      {@render chip(role.name, role.color, role)}
    {/each}
    {#if hiddenCount > 0}
      <li
        aria-label={strings.profileCard.moreRoles(hiddenCount)}
        class="inline-flex h-(--mw-component-server-settings-chip-height) items-center rounded-(--mw-component-server-settings-chip-radius) bg-(--mw-component-server-settings-chip-background) px-(--mw-component-server-settings-chip-padding-inline) type-body-sm text-(--mw-component-server-settings-chip-text) tabular-nums"
      >
        +{hiddenCount}
      </li>
    {/if}
  </ul>
{/if}

<style>
  /* The role's own hue, mixed in at a share low enough that the chip text keeps over 6:1 for any
     colour (design/tokens.json, chip-role-tint). */
  .role-chip-tinted {
    background-color: color-mix(
      in oklab,
      var(--role-color) calc(var(--mw-component-server-settings-chip-role-tint) * 100%),
      var(--mw-component-server-settings-chip-background)
    );
  }
</style>

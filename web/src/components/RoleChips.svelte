<script lang="ts">
  import { strings } from "../i18n";
  import type { Guild, Snowflake } from "../lib/protocol";
  import { roleColorHex } from "./server-settings";

  // A member's roles as small chips, highest first, each with its colour dot (decorative: the name
  // carries the meaning). The owner gets an "Owner" chip before them.
  interface Props {
    guild: Guild;
    roleIds: readonly Snowflake[];
    isOwner: boolean;
    label: string;
  }

  let { guild, roleIds, isOwner, label }: Props = $props();

  const roles = $derived(
    guild.roles.filter((role) => roleIds.includes(role.id)).sort((left, right) => right.position - left.position),
  );
</script>

{#snippet chip(name: string, color: number | null)}
  <li
    class="inline-flex h-(--mw-component-server-settings-chip-height) max-w-full items-center gap-(--mw-component-server-settings-chip-gap) rounded-(--mw-component-server-settings-chip-radius) bg-(--mw-component-server-settings-chip-background) px-(--mw-component-server-settings-chip-padding-inline) type-body-sm text-(--mw-component-server-settings-chip-text)"
  >
    {#if color !== null}
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
      {@render chip(strings.serverSettings.members.owner, null)}
    {/if}
    {#each roles as role (role.id)}
      {@render chip(role.name, role.color)}
    {/each}
  </ul>
{/if}

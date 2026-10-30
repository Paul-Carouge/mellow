<script lang="ts">
  import { strings } from "../i18n";
  import type { InviteLimits } from "../lib/api";
  import Icon from "./Icon.svelte";
  import {
    INVITE_LIFETIME_PRESETS_IN_SECONDS,
    INVITE_MAXIMUM_USES_PRESETS,
    lifetimeOptionLabel,
    maximumUsesOptionLabel,
  } from "./invites";

  // DESIGN.md §6.57: "Expire after" and "Maximum uses", Discord's presets in native selects, so
  // phones show their own picker and keyboards and screen readers get a real list box. Choosing
  // only changes the limits: the dialog creates the invite when asked to.
  interface Props {
    limits: InviteLimits;
    isDisabled?: boolean;
  }

  let { limits = $bindable(), isDisabled = false }: Props = $props();

  const id = $props.id();
</script>

{#snippet selectField(
  fieldId: string,
  label: string,
  options: readonly (number | null)[],
  optionLabel: (value: number | null) => string,
  value: number | null,
  onchange: (value: number | null) => void,
)}
  <div class="flex min-w-0 flex-[1_1_var(--mw-component-invite-field-basis)] flex-col gap-(--mw-component-text-field-label-gap)">
    <label for={fieldId} class="type-category text-(--mw-component-text-field-label-text)">{label}</label>
    <div class="relative">
      <select
        id={fieldId}
        disabled={isDisabled}
        class="h-(--mw-component-text-field-height) w-full cursor-pointer appearance-none rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) ps-(--mw-component-text-field-padding-inline) pe-[calc(2*var(--mw-component-text-field-padding-inline)_+_var(--mw-component-text-field-icon-size))] type-body-lg text-(--mw-component-text-field-text) focus-visible:focus-ring disabled:cursor-default disabled:opacity-(--mw-component-button-disabled-opacity)"
        onchange={(event) => onchange(options[event.currentTarget.selectedIndex] ?? null)}
      >
        {#each options as option (option)}
          <option value={option ?? ""} selected={option === value}>{optionLabel(option)}</option>
        {/each}
      </select>
      <Icon
        name="caret-down"
        class="pointer-events-none absolute end-(--mw-component-text-field-padding-inline) top-1/2 size-(--mw-component-text-field-icon-size) -translate-y-1/2 text-(--mw-component-invite-select-icon)"
      />
    </div>
  </div>
{/snippet}

<div class="flex flex-wrap gap-(--mw-component-text-field-field-gap)">
  {@render selectField(
    `${id}-lifetime`,
    strings.inviteLimits.expireAfter,
    INVITE_LIFETIME_PRESETS_IN_SECONDS,
    (seconds) => lifetimeOptionLabel(seconds),
    limits.expiresInSeconds,
    (expiresInSeconds) => (limits = { ...limits, expiresInSeconds }),
  )}
  {@render selectField(
    `${id}-uses`,
    strings.inviteLimits.maximumUses,
    INVITE_MAXIMUM_USES_PRESETS,
    (maximumUses) => maximumUsesOptionLabel(maximumUses),
    limits.maxUses,
    (maxUses) => (limits = { ...limits, maxUses }),
  )}
</div>

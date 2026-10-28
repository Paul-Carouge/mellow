<script lang="ts">
  import Icon, { type IconName } from "./Icon.svelte";

  // DESIGN.md §6.55: one radio row of a dialog's choice (a channel type, a server template, where
  // a channel goes). The radio is native, so arrows, Space, "checked" and forced colours come free;
  // the whole row is its label, and a checked row is washed and edged, not only dotted.
  interface Props {
    /** The radio group's name, shared by its rows. */
    name: string;
    value: string;
    /** The group's chosen value. */
    group: string;
    label: string;
    description?: string;
    icon?: IconName;
    /** The first row of a dialog's choice may take focus first. */
    isInitialFocus?: boolean;
  }

  let { name, value, group = $bindable(), label, description = "", icon, isInitialFocus = false }: Props = $props();

  const id = $props.id();
  const isChecked = $derived(group === value);
</script>

<label
  class={[
    "flex min-h-(--mw-component-choice-min-height) cursor-pointer items-center gap-(--mw-component-choice-gap) rounded-(--mw-component-choice-radius) border-(length:--mw-component-choice-border-width) px-(--mw-component-choice-padding-inline) py-(--mw-component-choice-padding-block) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) has-[input:focus-visible]:focus-ring",
    isChecked
      ? "border-(--mw-component-choice-border-checked) bg-(--mw-component-choice-background-checked)"
      : "border-(--mw-component-choice-border) hover:bg-(--mw-component-choice-background-hover)",
  ]}
>
  <input
    type="radio"
    {name}
    {value}
    checked={isChecked}
    aria-describedby={description ? `${id}-description` : undefined}
    data-initial-focus={isInitialFocus ? "" : undefined}
    class="size-(--mw-component-choice-indicator-size) shrink-0 accent-(--mw-component-choice-indicator) focus-visible:outline-none"
    onchange={() => (group = value)}
  />
  {#if icon}
    <Icon name={icon} class="size-(--mw-component-choice-icon-size) shrink-0 text-(--mw-component-choice-icon)" />
  {/if}
  <span class="flex min-w-0 flex-col">
    <span class="type-channel-name text-(--mw-component-choice-name-text)">{label}</span>
    {#if description}
      <span
        id="{id}-description"
        class={[
          "type-body-sm",
          isChecked ? "text-(--mw-component-choice-description-text-checked)" : "text-(--mw-component-choice-description-text)",
        ]}>{description}</span
      >
    {/if}
  </span>
</label>

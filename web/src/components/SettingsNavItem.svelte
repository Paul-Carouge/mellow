<script lang="ts">
  // DESIGN.md §6.43: a section of the settings navigation, or its danger item (filled red on hover).
  interface Props {
    id?: string;
    label: string;
    isSelected?: boolean;
    isDanger?: boolean;
    onclick: () => void;
  }

  let { id, label, isSelected = false, isDanger = false, onclick }: Props = $props();
</script>

<button
  {id}
  type="button"
  aria-current={isSelected ? "page" : undefined}
  aria-haspopup={isDanger ? "dialog" : undefined}
  class={[
    "flex h-(--mw-component-settings-nav-item-height) w-full items-center rounded-(--mw-component-settings-nav-item-radius) px-(--mw-component-settings-nav-item-padding-inline) text-start type-channel-name pointer-coarse:h-(--mw-size-hit-web-min)",
    isDanger
      ? "text-(--mw-component-settings-nav-danger-text) hover:bg-(--mw-component-settings-nav-danger-background-hover) hover:text-(--mw-component-settings-nav-danger-text-hover)"
      : isSelected
        ? "bg-(--mw-component-settings-nav-item-background-selected) text-(--mw-component-settings-nav-item-text-selected)"
        : "text-(--mw-component-settings-nav-item-text) hover:bg-(--mw-component-settings-nav-item-background-hover) hover:text-(--mw-component-settings-nav-item-text-hover)",
  ]}
  {onclick}
>
  <span class="truncate">{label}</span>
</button>

<script lang="ts">
  import { strings } from "../i18n";
  import { VOICE_USER_LIMIT_CHOICES, type VoiceUserLimitChoice } from "./channel-settings";
  import Icon from "./Icon.svelte";

  // A voice channel's "User limit" (DESIGN.md §5.3): "Unlimited" or 1 to 25 people, in a native
  // select like the invite limits (§6.57), so phones show their own picker.
  interface Props {
    value: VoiceUserLimitChoice;
    onchange: (value: VoiceUserLimitChoice) => void;
    isDisabled?: boolean;
  }

  let { value, onchange, isDisabled = false }: Props = $props();

  const id = $props.id();
</script>

<div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
  <label for="{id}-limit" class="type-category text-(--mw-component-text-field-label-text)">{strings.voice.userLimit}</label>
  <div class="relative">
    <select
      id="{id}-limit"
      disabled={isDisabled}
      aria-describedby="{id}-helper"
      class="h-(--mw-component-text-field-height) w-full cursor-pointer appearance-none rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) ps-(--mw-component-text-field-padding-inline) pe-[calc(2*var(--mw-component-text-field-padding-inline)_+_var(--mw-component-text-field-icon-size))] type-body-lg text-(--mw-component-text-field-text) focus-visible:focus-ring disabled:cursor-default disabled:opacity-(--mw-component-button-disabled-opacity)"
      onchange={(event) => onchange(VOICE_USER_LIMIT_CHOICES[event.currentTarget.selectedIndex] ?? "unlimited")}
    >
      {#each VOICE_USER_LIMIT_CHOICES as choice (choice)}
        <option value={choice} selected={choice === value}>
          {choice === "unlimited" ? strings.voice.unlimited : strings.voice.userLimitOption(choice)}
        </option>
      {/each}
    </select>
    <Icon
      name="caret-down"
      class="pointer-events-none absolute end-(--mw-component-text-field-padding-inline) top-1/2 size-(--mw-component-text-field-icon-size) -translate-y-1/2 text-(--mw-component-invite-select-icon)"
    />
  </div>
  <p id="{id}-helper" class="type-body text-(--mw-component-text-field-helper-text)">{strings.voice.userLimitHelper}</p>
</div>

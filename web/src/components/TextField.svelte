<script lang="ts">
  import type { HTMLInputAttributes } from "svelte/elements";

  import { strings } from "../i18n";
  import Icon, { type IconName } from "./Icon.svelte";

  // DESIGN.md §6.3: uppercase label above (rendered by the type token, the source stays sentence
  // case for screen readers), field with a 3:1 outline, optional helper below.
  interface Props extends Omit<HTMLInputAttributes, "class" | "value"> {
    id: string;
    label: string;
    value: string;
    helper?: string;
    leadingIcon?: IconName;
    /** Password fields get a show/hide toggle. */
    isPassword?: boolean;
    isMonospace?: boolean;
    element?: HTMLInputElement | null;
  }

  let {
    id,
    label,
    value = $bindable(),
    helper = "",
    leadingIcon,
    isPassword = false,
    isMonospace = false,
    element = $bindable(null),
    ...rest
  }: Props = $props();

  let isPasswordShown = $state(false);
</script>

<div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
  <label for={id} class="type-category text-(--mw-component-text-field-label-text)">{label}</label>
  <div
    class="flex h-(--mw-component-text-field-height) items-center gap-xs rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) px-(--mw-component-text-field-padding-inline) focus-within:focus-ring"
  >
    {#if leadingIcon}
      <Icon
        name={leadingIcon}
        class="size-(--mw-component-text-field-icon-size) text-(--mw-component-text-field-icon)"
      />
    {/if}
    <input
      bind:this={element}
      bind:value
      {...rest}
      {id}
      type={isPassword && !isPasswordShown ? "password" : (rest.type ?? "text")}
      aria-describedby={helper ? `${id}-helper` : undefined}
      class={[
        "h-full min-w-0 flex-1 bg-transparent type-body-lg text-(--mw-component-text-field-text) outline-none placeholder:text-(--mw-component-text-field-placeholder)",
        isMonospace && "font-mono",
      ]}
    />
    {#if isPassword}
      <button
        type="button"
        class="-me-(--mw-component-text-field-padding-inline) grid size-(--mw-size-hit-web-min) shrink-0 place-items-center text-(--mw-component-text-field-icon) hover:text-(--mw-component-text-field-icon-hover) focus-visible:focus-ring-inset"
        aria-label={strings.signIn.showPassword}
        aria-pressed={isPasswordShown}
        onclick={() => (isPasswordShown = !isPasswordShown)}
      >
        <Icon name={isPasswordShown ? "eye-slash" : "eye"} class="size-(--mw-component-text-field-icon-size)" />
      </button>
    {/if}
  </div>
  {#if helper}
    <p id="{id}-helper" class="type-body text-(--mw-component-text-field-helper-text)">{helper}</p>
  {/if}
</div>

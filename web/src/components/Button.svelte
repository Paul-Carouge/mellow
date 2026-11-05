<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  // DESIGN.md §6.1 at the web size (44 px). Disabled and busy buttons stay focusable
  // (`aria-disabled`), so focus is not lost while an action runs. A filled one takes the disabled
  // wash instead of its fill at half opacity, which made a dimmed apricot read as a brown button.
  interface Props extends Omit<HTMLButtonAttributes, "disabled" | "class"> {
    variant?: "primary" | "secondary" | "danger" | "ghost";
    isDisabled?: boolean;
    isFullWidth?: boolean;
    children: Snippet;
    onclick?: (event: MouseEvent) => void;
  }

  let {
    variant = "secondary",
    isDisabled = false,
    isFullWidth = false,
    type = "button",
    children,
    onclick,
    ...rest
  }: Props = $props();

  const variantClasses = $derived([
    variant === "primary" &&
      "bg-(--mw-component-button-primary-background) text-(--mw-component-button-primary-text) enabled:hover:bg-(--mw-component-button-primary-background-hover) enabled:active:bg-(--mw-component-button-primary-background-pressed)",
    variant === "secondary" &&
      "bg-(--mw-component-button-secondary-background) text-(--mw-component-button-secondary-text) hover:bg-(--mw-component-button-secondary-background-hover)",
    // Cancel and other quiet actions beside a primary one (as in the Dusk mockup's dialogs).
    variant === "ghost" && "text-text-normal hover:bg-state-hover",
    variant === "danger" &&
      "bg-(--mw-component-button-danger-background) text-(--mw-component-button-danger-text) hover:bg-(--mw-component-button-danger-background-hover)",
    // Ghost controls have no fill to swap, so they dim.
    variant === "ghost" && isDisabled && "opacity-(--mw-component-button-disabled-opacity)",
  ]);
</script>

<button
  {...rest}
  {type}
  aria-disabled={isDisabled || undefined}
  class={[
    "inline-flex h-(--mw-component-button-height-large) min-w-(--mw-size-hit-web-min) shrink-0 items-center justify-center rounded-(--mw-component-button-radius) px-(--mw-component-button-padding-inline) type-button transition-colors duration-(--mw-motion-duration-fast)",
    isFullWidth && "w-full",
    isDisabled && "pointer-events-none",
    isDisabled && variant !== "ghost"
      ? "bg-(--mw-component-button-disabled-background) text-(--mw-component-button-disabled-text)"
      : variantClasses,
  ]}
  onclick={(event) => {
    if (isDisabled) {
      event.preventDefault();
      return;
    }
    onclick?.(event);
  }}
>
  {@render children()}
</button>

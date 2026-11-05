// Fill and text of `Button.svelte` per variant (DESIGN.md §6.1). A disabled or busy filled button
// takes the disabled wash instead of its own fill at half opacity, which made a dimmed apricot read
// as a brown button; a ghost one has no fill to swap, so it dims.

export type ButtonVariant = "primary" | "secondary" | "danger" | "ghost";

const ENABLED_CLASSES_BY_VARIANT: Record<ButtonVariant, string> = {
  primary:
    "bg-(--mw-component-button-primary-background) text-(--mw-component-button-primary-text) enabled:hover:bg-(--mw-component-button-primary-background-hover) enabled:active:bg-(--mw-component-button-primary-background-pressed)",
  secondary:
    "bg-(--mw-component-button-secondary-background) text-(--mw-component-button-secondary-text) hover:bg-(--mw-component-button-secondary-background-hover)",
  danger:
    "bg-(--mw-component-button-danger-background) text-(--mw-component-button-danger-text) hover:bg-(--mw-component-button-danger-background-hover)",
  // Cancel and other quiet actions beside a primary one (as in the Dusk mockup's dialogs).
  ghost: "text-text-normal hover:bg-state-hover",
};

const DISABLED_FILLED_CLASSES = "bg-(--mw-component-button-disabled-background) text-(--mw-component-button-disabled-text)";
const DISABLED_GHOST_CLASSES = `${ENABLED_CLASSES_BY_VARIANT.ghost} opacity-(--mw-component-button-disabled-opacity)`;

export function buttonVariantClasses(variant: ButtonVariant, isDisabled: boolean): string {
  if (!isDisabled) {
    return ENABLED_CLASSES_BY_VARIANT[variant];
  }
  return variant === "ghost" ? DISABLED_GHOST_CLASSES : DISABLED_FILLED_CLASSES;
}

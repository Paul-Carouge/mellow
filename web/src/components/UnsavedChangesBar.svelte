<script lang="ts">
  import { strings } from "../i18n";
  import { prefersReducedMotion } from "../lib/motion.svelte";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import { durationTokenInMilliseconds, lengthTokenInPixels } from "./tokens";

  // DESIGN.md §5.8: slides up at the bottom of the settings page while a form differs from what is
  // stored. Leaving then is refused: the bar shakes (not under reduced motion), turns red-bordered
  // and asks to save or reset first, as Discord does.
  interface Props {
    /** Raised by each refused attempt to leave; 0 until the first. */
    attention: number;
    isSaving: boolean;
    errorMessage?: string;
    isSaveEnabled?: boolean;
    onreset: () => void;
    onsave: () => void;
  }

  let { attention, isSaving, errorMessage = "", isSaveEnabled = true, onreset, onsave }: Props = $props();

  let bar = $state<HTMLElement | null>(null);

  $effect(() => {
    if (attention === 0 || !bar || prefersReducedMotion.current) {
      return;
    }
    const distance = lengthTokenInPixels("component-unsaved-bar-shake-distance");
    const swings = Number(
      getComputedStyle(document.documentElement).getPropertyValue("--mw-component-unsaved-bar-shake-swing-count"),
    );
    // Each swing smaller than the last, alternating sides.
    const keyframes = Array.from({ length: swings }, (_, index) => ({
      transform: `translateX(${(index % 2 === 0 ? 1 : -1) * distance * (1 - index / swings)}px)`,
    }));
    const animation = bar.animate([{ transform: "none" }, ...keyframes, { transform: "none" }], {
      duration: durationTokenInMilliseconds("component-unsaved-bar-shake-step-duration") * swings,
    });
    return () => animation.cancel();
  });
</script>

<div class="sticky bottom-0 pb-(--mw-component-unsaved-bar-margin-bottom)">
  <div
    bind:this={bar}
    class={[
      "flex flex-wrap items-center justify-end gap-(--mw-component-unsaved-bar-button-gap) rounded-(--mw-component-unsaved-bar-radius) bg-(--mw-component-unsaved-bar-background) py-(--mw-component-unsaved-bar-padding-block) ps-(--mw-component-unsaved-bar-padding-leading) pe-(--mw-component-unsaved-bar-padding-trailing) shadow-high motion-safe:animate-[mw-rise_var(--mw-component-unsaved-bar-slide-duration)_var(--mw-motion-easing-enter)_both]",
      attention > 0
        ? "outline-(length:--mw-component-unsaved-bar-border-attention-width) outline-(--mw-component-unsaved-bar-border-attention) outline-solid"
        : "outline-(length:--mw-component-unsaved-bar-border-width) outline-(--mw-component-unsaved-bar-border) outline-solid",
    ]}
  >
    <p role="alert" class="me-auto min-w-0 flex-[1_1_var(--mw-component-roles-list-width)] type-body text-(--mw-component-unsaved-bar-text)">
      {#if errorMessage}
        <span class="flex gap-(--mw-component-settings-error-icon-gap) text-(--mw-component-settings-error-text)">
          <Icon name="warning-circle" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
          <span>{errorMessage}</span>
        </span>
      {:else}
        {attention > 0 ? strings.serverSettings.unsaved.saveOrReset : strings.serverSettings.unsaved.careful}
      {/if}
    </p>
    <Button variant="ghost" isDisabled={isSaving} onclick={onreset}>{strings.serverSettings.unsaved.reset}</Button>
    <Button variant="primary" isDisabled={isSaving || !isSaveEnabled} onclick={onsave}>
      {isSaving ? strings.serverSettings.unsaved.saving : strings.serverSettings.unsaved.saveChanges}
    </Button>
  </div>
</div>

<script lang="ts">
  import { untrack } from "svelte";
  import type { Snippet } from "svelte";

  import { liveAnnouncer } from "../lib/checklists";

  // The level of a checklist or a progress bar (docs/design/web-checklists-and-progress.md,
  // ADR-0047): a `progressbar` named by its card's heading, a track filled from its leading side,
  // and its figure. A checklist's level is segmented, one segment per item, so "3 of 5" reads as
  // three lit items. A change is also said in a polite live region, since screen readers do not
  // announce a progress bar's new value on their own; the first value is not, so a page of
  // history stays quiet.
  interface Props {
    labelledBy: string;
    value: number;
    maximum: number;
    /** 0 to 100, for the continuous fill. */
    percent: number;
    /** "3/5" or "40 %", shown beside the track. */
    figure: string;
    /** "3 of 5 done": the value a screen reader says. */
    spoken: string;
    /** One segment per item (a checklist); without it, one continuous fill (a progress bar). */
    isSegmented?: boolean;
    /** Replaces the figure (a checklist's "All done"). */
    figureOverride?: Snippet;
  }

  let { labelledBy, value, maximum, percent, figure, spoken, isSegmented = false, figureOverride }: Props = $props();

  const announce = liveAnnouncer(untrack(() => spoken));
  const announcement = $derived(announce(spoken));
  const segments = $derived(isSegmented ? Array.from({ length: Math.max(maximum, 1) }, (_, index) => index < value) : []);
</script>

<div class="flex items-center gap-(--mw-component-poll-answer-inner-gap)">
  {#if isSegmented}
    <div
      role="progressbar"
      aria-labelledby={labelledBy}
      aria-valuemin={0}
      aria-valuemax={maximum}
      aria-valuenow={value}
      aria-valuetext={spoken}
      class="flex h-(--mw-component-checklist-meter-height) min-w-0 flex-1 gap-(--mw-component-checklist-meter-segment-gap)"
    >
      {#each segments as isLit, index (index)}
        <span
          aria-hidden="true"
          class={[
            "segment h-full min-w-0 flex-1 rounded-full",
            isLit ? "bg-(--mw-component-checklist-meter-fill)" : "bg-(--mw-component-checklist-meter-track)",
          ]}
        ></span>
      {/each}
    </div>
  {:else}
    <div
      role="progressbar"
      aria-labelledby={labelledBy}
      aria-valuemin={0}
      aria-valuemax={maximum}
      aria-valuenow={value}
      aria-valuetext={spoken}
      class="relative h-(--mw-component-checklist-meter-height) min-w-0 flex-1 overflow-hidden rounded-full bg-(--mw-component-checklist-meter-track)"
    >
      <span aria-hidden="true" class="meter-fill absolute inset-y-0 start-0 rounded-full bg-(--mw-component-checklist-meter-fill)" style:--meter-share="{percent}%"
      ></span>
    </div>
  {/if}
  {#if figureOverride}
    {@render figureOverride()}
  {:else}
    <span aria-hidden="true" class="shrink-0 type-body font-semibold tabular-nums text-(--mw-component-checklist-figure-text)">{figure}</span>
  {/if}
  <span class="sr-only" aria-live="polite">{announcement}</span>
</div>

<style>
  /* Animated only when motion is allowed: the duration tokens are 0 under reduced motion. */
  .meter-fill {
    inline-size: var(--meter-share);
    transition: inline-size var(--mw-motion-duration-slow) var(--mw-motion-easing-standard);
  }

  .segment {
    transition: background-color var(--mw-motion-duration-base) var(--mw-motion-easing-standard);
  }
</style>

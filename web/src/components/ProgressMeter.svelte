<script lang="ts">
  import { untrack } from "svelte";

  import { liveAnnouncer } from "../lib/checklists";

  // The level of a checklist or a progress bar (docs/design/web-checklists-and-progress.md,
  // ADR-0047): a `progressbar` named by its card's heading, a track filled from its leading side
  // in the poll level's colour, and its figure. A change is also said in a polite live region,
  // since screen readers do not announce a progress bar's new value on their own; the first value
  // is not, so a page of history stays quiet.
  interface Props {
    labelledBy: string;
    value: number;
    maximum: number;
    /** 0 to 100, for the fill. */
    percent: number;
    /** "3/5" or "40 %", shown beside the track. */
    figure: string;
    /** "3 of 5 done": the value a screen reader says. */
    spoken: string;
  }

  let { labelledBy, value, maximum, percent, figure, spoken }: Props = $props();

  const announce = liveAnnouncer(untrack(() => spoken));
  const announcement = $derived(announce(spoken));
</script>

<div class="flex items-center gap-(--mw-component-poll-answer-inner-gap)">
  <div
    role="progressbar"
    aria-labelledby={labelledBy}
    aria-valuemin={0}
    aria-valuemax={maximum}
    aria-valuenow={value}
    aria-valuetext={spoken}
    class="relative h-(--mw-component-poll-mark-glyph-size) min-w-0 flex-1 overflow-hidden rounded-full border-(length:--mw-component-poll-answer-border-width) border-(--mw-component-poll-answer-border) bg-(--mw-component-poll-answer-background)"
  >
    <span aria-hidden="true" class="meter-fill absolute inset-y-0 start-0 bg-(--mw-component-poll-bar-mine)" style:--meter-share="{percent}%"
    ></span>
  </div>
  <span aria-hidden="true" class="shrink-0 type-body-sm font-semibold tabular-nums text-(--mw-component-poll-percent-text)">{figure}</span>
  <span class="sr-only" aria-live="polite">{announcement}</span>
</div>

<style>
  /* Animated only when motion is allowed: the duration token is 0 under reduced motion. */
  .meter-fill {
    inline-size: var(--meter-share);
    transition: inline-size var(--mw-motion-duration-slow) var(--mw-motion-easing-standard);
  }
</style>

<script lang="ts">
  import { strings } from "../i18n";
  import { progressPercent } from "../lib/checklists";
  import type { ProgressBar } from "../lib/protocol";
  import Icon from "./Icon.svelte";
  import ProgressMeter from "./ProgressMeter.svelte";
  import { lengthTokenInPixels } from "./tokens";

  // A progress bar in a message (docs/design/web-checklists-and-progress.md, ADR-0047): the
  // checklist's frame and level, its label; its author and moderators update it from the card.
  interface Props {
    bar: ProgressBar;
    isPending?: boolean;
    canEdit: boolean;
    onupdate?: () => void;
  }

  let { bar, isPending = false, canEdit, onupdate }: Props = $props();

  const id = $props.id();
  // Phones: the card and its rows give their insets back to the text.
  let cardWidth: number | undefined = $state();
  const isCompact = $derived(cardWidth !== undefined && cardWidth < lengthTokenInPixels("component-checklist-compact-width"));
  const percent = $derived(progressPercent(bar.value, bar.maximum));
</script>

<section
  bind:clientWidth={cardWidth}
  aria-labelledby="{id}-label"
  class="mt-3xs flex w-full max-w-(--mw-component-checklist-max-width) flex-col gap-(--mw-component-checklist-gap) rounded-(--mw-component-poll-radius) border-(length:--mw-component-poll-border-width) border-(--mw-component-poll-border) bg-(--mw-component-poll-background) p-(--card-padding)"
  style:--card-padding="var(--mw-component-checklist-padding{isCompact ? '-compact' : ''})"
  style:--card-inset="var(--mw-component-checklist-row-padding-inline{isCompact ? '-compact' : ''})"
>
  <div class="flex flex-col gap-xs px-(--card-inset) py-2xs">
    <h4 id="{id}-label" class="flex items-start gap-(--mw-component-checklist-header-gap) type-heading-md break-words text-(--mw-component-poll-question-text)">
      <Icon
        name="gauge"
        class="mt-[calc((var(--mw-type-heading-md-font-size)*var(--mw-type-heading-md-line-height)-var(--mw-component-checklist-mark-size))/2)] size-(--mw-component-checklist-mark-size) shrink-0 text-(--mw-component-poll-hint-text)"
      />
      <span class="min-w-0">{bar.label}</span>
    </h4>
    <ProgressMeter
      labelledBy="{id}-label"
      value={bar.value}
      maximum={bar.maximum}
      {percent}
      figure={strings.progressBars.figure(bar.value, bar.maximum, percent)}
      spoken={strings.progressBars.spoken(bar.value, bar.maximum, percent)}
    />
  </div>
  {#if canEdit && !isPending}
    <div class="flex">
      <button
        type="button"
        class="inline-flex h-(--mw-component-button-height-small) items-center gap-2xs rounded-(--mw-component-checklist-row-radius) px-(--card-inset) type-body font-medium text-text-secondary transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-checklist-row-background-hover) hover:text-text-heading focus-visible:focus-ring pointer-coarse:h-(--mw-size-hit-web-min)"
        onclick={() => onupdate?.()}
      >
        <Icon name="pencil-simple" class="size-(--mw-component-checklist-mark-glyph-size)" />
        {strings.progressBars.update}
      </button>
    </div>
  {/if}
</section>

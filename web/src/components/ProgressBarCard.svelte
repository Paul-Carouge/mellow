<script lang="ts">
  import { strings } from "../i18n";
  import { progressPercent } from "../lib/checklists";
  import type { ProgressBar } from "../lib/protocol";
  import Icon from "./Icon.svelte";
  import ProgressMeter from "./ProgressMeter.svelte";

  // A progress bar in a message (docs/design/web-checklists-and-progress.md, ADR-0047): the poll
  // card's frame, its label and its level; its author and moderators update it from the card.
  interface Props {
    bar: ProgressBar;
    isPending?: boolean;
    canEdit: boolean;
    onupdate?: () => void;
  }

  let { bar, isPending = false, canEdit, onupdate }: Props = $props();

  const id = $props.id();
  const percent = $derived(progressPercent(bar.value, bar.maximum));
</script>

<section
  aria-labelledby="{id}-label"
  class="mt-3xs flex w-full max-w-(--mw-component-poll-max-width) flex-col gap-(--mw-component-poll-header-gap) rounded-(--mw-component-poll-radius) border-(length:--mw-component-poll-border-width) border-(--mw-component-poll-border) bg-(--mw-component-poll-background) p-(--mw-component-poll-padding)"
>
  <h4 id="{id}-label" class="flex items-center gap-2xs type-heading-md break-words text-(--mw-component-poll-question-text)">
    <Icon name="gauge" class="size-(--mw-component-poll-mark-size) shrink-0 text-(--mw-component-poll-hint-text)" />
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
  {#if canEdit && !isPending}
    <div class="flex">
      <button
        type="button"
        class="inline-flex min-h-(--mw-size-hit-desktop-min) items-center gap-3xs rounded-xs type-body-sm font-semibold text-text-link hover:underline focus-visible:focus-ring pointer-coarse:min-h-(--mw-size-hit-web-min)"
        onclick={() => onupdate?.()}
      >
        <Icon name="pencil-simple" class="size-(--mw-component-poll-mark-glyph-size)" />
        {strings.progressBars.update}
      </button>
    </div>
  {/if}
</section>

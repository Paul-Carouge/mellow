<script lang="ts">
  import type { HTMLInputAttributes } from "svelte/elements";

  // A labelled native slider (DESIGN.md §6.5, stage 16.3): the label and the value on one line,
  // the browser's own range input under them, 44 px tall so it is easy to grab and keyboard
  // arrows, Page Up/Down, Home and End work as everywhere. The value is read as `valueText`.
  interface Props {
    label: string;
    value: number;
    min: number;
    max: number;
    step?: number;
    valueText: (value: number) => string;
    describedBy?: string;
    /** Extra attributes for the input (a menu marks its slider as one of its stops). */
    inputAttributes?: HTMLInputAttributes;
    oninput: (value: number) => void;
  }

  let { label, value, min, max, step = 1, valueText, describedBy, inputAttributes = {}, oninput }: Props = $props();

  const id = $props.id();
</script>

<div class="flex min-w-0 flex-col">
  <div class="flex items-baseline justify-between gap-xs">
    <label for="{id}-range" class="type-category text-(--mw-component-text-field-label-text)">{label}</label>
    <span aria-hidden="true" class="shrink-0 type-body-sm tabular-nums text-(--mw-component-slider-value-text)">{valueText(value)}</span>
  </div>
  <input
    {...inputAttributes}
    id="{id}-range"
    type="range"
    {min}
    {max}
    {step}
    {value}
    aria-valuetext={valueText(value)}
    aria-describedby={describedBy}
    class="h-(--mw-size-hit-web-min) w-full min-w-0 cursor-pointer accent-(--mw-component-slider-accent) focus-visible:focus-ring"
    oninput={(event) => oninput(Number(event.currentTarget.value))}
  />
</div>

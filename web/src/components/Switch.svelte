<script lang="ts">
  import Icon from "./Icon.svelte";

  // DESIGN.md §6.5: a check on the knob when on, a cross when off, so the state never relies on
  // colour. The 44 px button holds the 40 × 24 track; its row gives the label and description.
  // A locked switch stays focusable (`aria-disabled`), so the row's reason can be read on it.
  interface Props {
    isOn: boolean;
    isLocked?: boolean;
    labelledBy: string;
    describedBy?: string;
    onchange: (isOn: boolean) => void;
  }

  let { isOn, isLocked = false, labelledBy, describedBy, onchange }: Props = $props();
</script>

<button
  type="button"
  role="switch"
  aria-checked={isOn}
  aria-disabled={isLocked || undefined}
  aria-labelledby={labelledBy}
  aria-describedby={describedBy}
  class={["grid size-(--mw-size-hit-web-min) shrink-0 place-items-center rounded-sm", isLocked && "cursor-not-allowed"]}
  onclick={() => {
    if (!isLocked) {
      onchange(!isOn);
    }
  }}
>
  <span
    class={[
      "relative h-(--mw-component-switch-height) w-(--mw-component-switch-width) rounded-full transition-colors duration-(--mw-motion-duration-fast)",
      isOn ? "bg-(--mw-component-switch-track-on)" : "bg-(--mw-component-switch-track-off)",
      isLocked && "opacity-(--mw-component-switch-disabled-opacity)",
    ]}
  >
    <span
      class={[
        "absolute top-(--mw-component-switch-knob-inset) grid size-(--mw-component-switch-knob-size) place-items-center rounded-full bg-(--mw-component-switch-knob) transition-[inset-inline-start] duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard)",
        isOn
          ? "start-[calc(var(--mw-component-switch-width)-var(--mw-component-switch-knob-size)-var(--mw-component-switch-knob-inset))] text-(--mw-component-switch-glyph-on)"
          : "start-(--mw-component-switch-knob-inset) text-(--mw-component-switch-glyph-off)",
      ]}
    >
      <Icon name={isOn ? "check" : "x"} class="size-(--mw-component-switch-glyph-size)" />
    </span>
  </span>
</button>

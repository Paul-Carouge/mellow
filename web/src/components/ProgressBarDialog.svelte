<script lang="ts">
  import { tick, untrack } from "svelte";

  import { strings } from "../i18n";
  import {
    MAXIMUM_PROGRESS_BAR_LABEL_LENGTH,
    MAXIMUM_PROGRESS_BAR_MAXIMUM,
    type NewProgressBar,
    type ProgressBar,
  } from "../lib/protocol";
  import ModalDialog from "./ModalDialog.svelte";
  import { characterCount } from "./poll-form";
  import {
    type ProgressBarDraft,
    hasProgressBarDraftProblems,
    newProgressBarOf,
    progressBarDraftOf,
    progressBarDraftProblems,
  } from "./tracker-form";

  // Creating a progress bar from the "/" menu, or updating one from its card
  // (docs/design/web-checklists-and-progress.md, ADR-0047): a label, a value and what it is out
  // of; out of 100 it reads as a percentage. Problems are said once "Send" or "Save" is pressed.
  interface Props {
    /** The bar being updated; `null` to create one. */
    bar: ProgressBar | null;
    onsend: (bar: NewProgressBar) => void;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { bar, onsend, onclose, fallbackFocus }: Props = $props();

  const FIELD_CLASS =
    "flex h-(--mw-component-text-field-height) min-w-0 items-center gap-xs rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) px-(--mw-component-text-field-padding-inline) focus-within:focus-ring has-[input[aria-invalid=true]]:border-(--mw-component-dialog-error-text)";
  const INPUT_CLASS =
    "h-full min-w-0 flex-1 bg-transparent type-body-lg text-(--mw-component-text-field-text) outline-none placeholder:text-(--mw-component-text-field-placeholder)";

  const id = $props.id();
  // The bar as it was when the dialog opened: a later event does not move what is being typed.
  const draft = $state<ProgressBarDraft>(progressBarDraftOf(untrack(() => bar)));
  let hasTriedSending = $state(false);
  const problems = $derived(progressBarDraftProblems(draft));
  const labelMessage = $derived(
    problems.label === "tooLong"
      ? strings.polls.textTooLong(MAXIMUM_PROGRESS_BAR_LABEL_LENGTH)
      : problems.label === "empty" && hasTriedSending
        ? strings.progressBars.labelEmpty
        : "",
  );
  const maximumMessage = $derived(
    problems.isMaximumInvalid && hasTriedSending ? strings.progressBars.wholeNumberUpTo(1, MAXIMUM_PROGRESS_BAR_MAXIMUM) : "",
  );
  const valueMessage = $derived(
    problems.isValueInvalid && hasTriedSending ? strings.progressBars.wholeNumberUpTo(0, Number(draft.maximum)) : "",
  );

  async function send() {
    if (hasProgressBarDraftProblems(problems)) {
      hasTriedSending = true;
      await tick();
      const firstWithProblem =
        problems.label !== null ? "label" : problems.isMaximumInvalid ? "maximum" : "value";
      document.getElementById(`${id}-${firstWithProblem}`)?.focus();
      return;
    }
    onsend(newProgressBarOf($state.snapshot(draft)));
  }
</script>

{#snippet fieldMessage(fieldId: string, message: string)}
  <p id="{fieldId}-message" role="status" class="type-body-sm text-(--mw-component-dialog-error-text) empty:hidden">{message}</p>
{/snippet}

<ModalDialog
  title={bar ? strings.progressBars.edit : strings.progressBars.create}
  subtitle={bar ? "" : strings.progressBars.createSubtitle}
  submitLabel={bar ? strings.progressBars.save : strings.progressBars.send}
  {onclose}
  onsubmit={() => void send()}
  {fallbackFocus}
>
  <div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
    <div class="flex items-baseline justify-between gap-xs">
      <label for="{id}-label" class="type-category text-(--mw-component-text-field-label-text)">{strings.progressBars.label}</label>
      <span
        class={[
          "shrink-0 type-body-sm tabular-nums",
          characterCount(draft.label) > MAXIMUM_PROGRESS_BAR_LABEL_LENGTH
            ? "text-(--mw-component-dialog-error-text)"
            : "text-(--mw-component-text-field-helper-text)",
        ]}
        id="{id}-label-count"
        ><span aria-hidden="true">{strings.polls.charactersUsed(characterCount(draft.label), MAXIMUM_PROGRESS_BAR_LABEL_LENGTH)}</span
        ><span class="sr-only">{strings.polls.charactersUsedSpoken(characterCount(draft.label), MAXIMUM_PROGRESS_BAR_LABEL_LENGTH)}</span
        ></span
      >
    </div>
    <div class={FIELD_CLASS}>
      <input
        id="{id}-label"
        bind:value={draft.label}
        class={INPUT_CLASS}
        placeholder={strings.progressBars.labelPlaceholder}
        autocomplete="off"
        aria-invalid={labelMessage !== "" || undefined}
        aria-describedby="{id}-label-count {id}-label-message"
        data-initial-focus={bar ? undefined : ""}
      />
    </div>
    {@render fieldMessage(`${id}-label`, labelMessage)}
  </div>

  <div class="grid grid-cols-2 gap-sm">
    <div class="flex min-w-0 flex-col gap-(--mw-component-text-field-label-gap)">
      <label for="{id}-value" class="type-category text-(--mw-component-text-field-label-text)">{strings.progressBars.value}</label>
      <div class={FIELD_CLASS}>
        <input
          id="{id}-value"
          bind:value={draft.value}
          class={[INPUT_CLASS, "tabular-nums"]}
          inputmode="numeric"
          autocomplete="off"
          aria-invalid={valueMessage !== "" || undefined}
          aria-describedby="{id}-value-message"
          data-initial-focus={bar ? "" : undefined}
        />
      </div>
      {@render fieldMessage(`${id}-value`, valueMessage)}
    </div>
    <div class="flex min-w-0 flex-col gap-(--mw-component-text-field-label-gap)">
      <label for="{id}-maximum" class="type-category text-(--mw-component-text-field-label-text)">{strings.progressBars.maximum}</label>
      <div class={FIELD_CLASS}>
        <input
          id="{id}-maximum"
          bind:value={draft.maximum}
          class={[INPUT_CLASS, "tabular-nums"]}
          inputmode="numeric"
          autocomplete="off"
          aria-invalid={maximumMessage !== "" || undefined}
          aria-describedby="{id}-maximum-hint {id}-maximum-message"
        />
      </div>
      {@render fieldMessage(`${id}-maximum`, maximumMessage)}
    </div>
  </div>
  <p id="{id}-maximum-hint" class="type-body-sm text-(--mw-component-text-field-helper-text)">{strings.progressBars.maximumHint}</p>
</ModalDialog>

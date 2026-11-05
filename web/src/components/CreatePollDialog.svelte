<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import {
    MAXIMUM_POLL_ANSWERS,
    MAXIMUM_POLL_ANSWER_LENGTH,
    MAXIMUM_POLL_QUESTION_LENGTH,
    MINIMUM_POLL_ANSWERS,
    type NewPoll,
    POLL_DURATIONS_IN_HOURS,
  } from "../lib/protocol";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import {
    type PollDraft,
    canAddAnswer,
    canRemoveAnswer,
    characterCount,
    emptyPollDraft,
    hasPollDraftProblems,
    newPollOf,
    pollDraftProblems,
  } from "./poll-form";
  import Switch from "./Switch.svelte";

  // "Create a poll" (DESIGN.md §6.82, docs/design/web-polls.md, ADR-0043), opened from the "/" menu:
  // the question, two to ten answers, one or several choices and a duration. A repeated or too long
  // text is said as it is typed; a missing question or answer only once "Send poll" is pressed.
  // Sending goes through the outbox, so the dialog closes at once and a failure shows on the poll.
  interface Props {
    /** Says who votes: a channel's readers, or a conversation's people. */
    isConversation: boolean;
    onsend: (poll: NewPoll) => void;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { isConversation, onsend, onclose, fallbackFocus }: Props = $props();

  /** Past this many characters of an answer, its counter shows (the question's always does). */
  const ANSWER_COUNTER_THRESHOLD = MAXIMUM_POLL_ANSWER_LENGTH - 10;
  const FIELD_CLASS =
    "flex h-(--mw-component-text-field-height) min-w-0 items-center gap-xs rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) px-(--mw-component-text-field-padding-inline) focus-within:focus-ring has-[input[aria-invalid=true]]:border-(--mw-component-dialog-error-text)";
  const INPUT_CLASS =
    "h-full min-w-0 flex-1 bg-transparent type-body-lg text-(--mw-component-text-field-text) outline-none placeholder:text-(--mw-component-text-field-placeholder)";

  const id = $props.id();
  const draft = $state<PollDraft>(emptyPollDraft());
  /** Keys for the answer rows, so a removed row takes its own field and focus with it. */
  let answerKeys = $state(draft.answers.map((_, index) => index));
  let nextAnswerKey = draft.answers.length;
  let hasTriedSending = $state(false);
  let answerList = $state<HTMLElement | null>(null);

  const problems = $derived(pollDraftProblems(draft));
  const questionLength = $derived(characterCount(draft.question));
  const questionMessage = $derived(
    problems.question === "tooLong"
      ? strings.polls.textTooLong(MAXIMUM_POLL_QUESTION_LENGTH)
      : problems.question === "empty" && hasTriedSending
        ? strings.polls.questionEmpty
        : "",
  );
  const answersMessage = $derived(
    problems.hasTooFewAnswers && hasTriedSending ? strings.polls.tooFewAnswers(MINIMUM_POLL_ANSWERS) : "",
  );

  function answerMessage(index: number): string {
    const problem = problems.answers[index];
    if (problem === "tooLong") {
      return strings.polls.textTooLong(MAXIMUM_POLL_ANSWER_LENGTH);
    }
    return problem === "duplicate" ? strings.polls.duplicateAnswer((draft.answers[index] ?? "").trim()) : "";
  }

  function answerInputs(): HTMLInputElement[] {
    return [...(answerList?.querySelectorAll<HTMLInputElement>("input") ?? [])];
  }

  async function addAnswer() {
    if (!canAddAnswer(draft)) {
      return;
    }
    draft.answers.push("");
    answerKeys.push(nextAnswerKey);
    nextAnswerKey += 1;
    await tick();
    answerInputs().at(-1)?.focus();
  }

  async function removeAnswer(index: number) {
    if (!canRemoveAnswer(draft)) {
      return;
    }
    draft.answers.splice(index, 1);
    answerKeys.splice(index, 1);
    await tick();
    // The row that took its place, else the one before; the Remove button went with its row.
    const inputs = answerInputs();
    (inputs[index] ?? inputs[index - 1])?.focus();
  }

  /** Enter goes to the next answer, adding one after the last, as on Discord; never sends. */
  function handleAnswerKeydown(event: KeyboardEvent, index: number) {
    if (event.key !== "Enter" || event.isComposing) {
      return;
    }
    event.preventDefault();
    const next = answerInputs()[index + 1];
    if (next) {
      next.focus();
    } else if (canAddAnswer(draft)) {
      void addAnswer();
    }
  }

  function send() {
    if (hasPollDraftProblems(problems)) {
      hasTriedSending = true;
      focusFirstProblem();
      return;
    }
    onsend(newPollOf($state.snapshot(draft)));
  }

  async function focusFirstProblem() {
    await tick();
    if (problems.question !== null) {
      document.getElementById(`${id}-question`)?.focus();
      return;
    }
    const inputs = answerInputs();
    const withProblem = problems.answers.findIndex((problem) => problem !== null);
    const firstEmpty = draft.answers.findIndex((text) => text.trim() === "");
    (inputs[withProblem >= 0 ? withProblem : firstEmpty] ?? inputs[0])?.focus();
  }
</script>

{#snippet counter(fieldId: string, used: number, maximum: number)}
  <span
    id="{fieldId}-count"
    class={[
      "shrink-0 type-body-sm tabular-nums",
      used > maximum ? "text-(--mw-component-dialog-error-text)" : "text-(--mw-component-text-field-helper-text)",
    ]}
    ><span aria-hidden="true">{strings.polls.charactersUsed(used, maximum)}</span><span class="sr-only"
      >{strings.polls.charactersUsedSpoken(used, maximum)}</span
    ></span
  >
{/snippet}

{#snippet fieldMessage(fieldId: string, message: string)}
  <!-- Kept in the page, so a message that appears is announced. -->
  <p id="{fieldId}-message" role="status" class="type-body-sm text-(--mw-component-dialog-error-text) empty:hidden">{message}</p>
{/snippet}

<ModalDialog
  title={strings.polls.create}
  subtitle={isConversation ? strings.polls.createSubtitleConversation : strings.polls.createSubtitle}
  submitLabel={strings.polls.send}
  {onclose}
  onsubmit={send}
  {fallbackFocus}
>
  <div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
    <div class="flex items-baseline justify-between gap-xs">
      <label for="{id}-question" class="type-category text-(--mw-component-text-field-label-text)">{strings.polls.question}</label>
      {@render counter(`${id}-question`, questionLength, MAXIMUM_POLL_QUESTION_LENGTH)}
    </div>
    <div class={FIELD_CLASS}>
      <input
        id="{id}-question"
        bind:value={draft.question}
        class={INPUT_CLASS}
        placeholder={strings.polls.questionPlaceholder}
        autocomplete="off"
        enterkeyhint="next"
        aria-invalid={questionMessage !== "" || undefined}
        aria-describedby="{id}-question-count {id}-question-message"
        data-initial-focus=""
        onkeydown={(event) => {
          if (event.key === "Enter" && !event.isComposing) {
            event.preventDefault();
            answerInputs()[0]?.focus();
          }
        }}
      />
    </div>
    {@render fieldMessage(`${id}-question`, questionMessage)}
  </div>

  <fieldset class="flex min-w-0 flex-col gap-(--mw-component-text-field-label-gap)">
    <legend class="mb-(--mw-component-text-field-label-gap) type-category text-(--mw-component-text-field-label-text)">
      {strings.polls.answers}
    </legend>
    <ol bind:this={answerList} class="flex flex-col gap-(--mw-component-text-field-helper-gap)">
      {#each draft.answers as answer, index (answerKeys[index])}
        {@const fieldId = `${id}-answer-${answerKeys[index]}`}
        {@const message = answerMessage(index)}
        {@const length = characterCount(answer)}
        <li class="flex flex-col gap-3xs">
          <div class="flex items-center gap-2xs">
            <div class={[FIELD_CLASS, "flex-1"]}>
              <input
                id={fieldId}
                bind:value={draft.answers[index]}
                class={INPUT_CLASS}
                placeholder={strings.polls.answer(index + 1)}
                aria-label={strings.polls.answer(index + 1)}
                autocomplete="off"
                enterkeyhint="next"
                aria-invalid={message !== "" || undefined}
                aria-describedby="{fieldId}-message{length > ANSWER_COUNTER_THRESHOLD ? ` ${fieldId}-count` : ''}"
                onkeydown={(event) => handleAnswerKeydown(event, index)}
              />
              {#if length > ANSWER_COUNTER_THRESHOLD}
                {@render counter(fieldId, length, MAXIMUM_POLL_ANSWER_LENGTH)}
              {/if}
            </div>
            {#if canRemoveAnswer(draft)}
              <IconButton
                icon="x"
                label={strings.polls.removeAnswer(index + 1)}
                class="size-(--mw-size-hit-web-min) text-(--mw-component-text-field-icon) hover:bg-state-hover hover:text-(--mw-component-text-field-icon-hover) focus-visible:focus-ring"
                iconClass="size-(--mw-component-text-field-icon-size)"
                onclick={() => void removeAnswer(index)}
              />
            {/if}
          </div>
          {@render fieldMessage(fieldId, message)}
        </li>
      {/each}
    </ol>
    {@render fieldMessage(`${id}-answers`, answersMessage)}
    <div class="flex flex-wrap items-center gap-x-sm gap-y-2xs">
      {#if canAddAnswer(draft)}
        <!-- Its label lines up with the fields' edge; the hover wash still reaches past it. -->
        <div class="-ms-(--mw-component-button-padding-inline)">
          <Button variant="ghost" onclick={() => void addAnswer()}>
            <Icon name="plus" class="me-2xs size-(--mw-component-text-field-icon-size)" />
            {strings.polls.addAnswer}
          </Button>
        </div>
      {/if}
      <p class="type-body-sm text-(--mw-component-text-field-helper-text)" aria-live="polite">
        {strings.polls.answersLeft(MAXIMUM_POLL_ANSWERS - draft.answers.length)}
      </p>
    </div>
  </fieldset>

  <div class="flex items-center gap-md">
    <div class="min-w-0 flex-1">
      <p id="{id}-multiple" class="type-channel-name text-text-heading">{strings.polls.allowMultiple}</p>
      <p id="{id}-multiple-hint" class="type-body text-text-muted">{strings.polls.allowMultipleHint}</p>
    </div>
    <Switch
      isOn={draft.allowsMultipleAnswers}
      labelledBy="{id}-multiple"
      describedBy="{id}-multiple-hint"
      onchange={(isOn) => (draft.allowsMultipleAnswers = isOn)}
    />
  </div>

  <div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
    <label for="{id}-duration" class="type-category text-(--mw-component-text-field-label-text)">{strings.polls.duration}</label>
    <div class="relative">
      <select
        id="{id}-duration"
        class="h-(--mw-component-text-field-height) w-full cursor-pointer appearance-none rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) ps-(--mw-component-text-field-padding-inline) pe-[calc(2*var(--mw-component-text-field-padding-inline)_+_var(--mw-component-text-field-icon-size))] type-body-lg text-(--mw-component-text-field-text) focus-visible:focus-ring"
        bind:value={draft.durationHours}
      >
        {#each POLL_DURATIONS_IN_HOURS as hours (hours)}
          <option value={hours}>{strings.polls.durationOption(hours)}</option>
        {/each}
      </select>
      <Icon
        name="caret-down"
        class="pointer-events-none absolute end-(--mw-component-text-field-padding-inline) top-1/2 size-(--mw-component-text-field-icon-size) -translate-y-1/2 text-(--mw-component-invite-select-icon)"
      />
    </div>
  </div>
</ModalDialog>

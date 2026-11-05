<script lang="ts">
  import { strings } from "../i18n";
  import { isPollEnded } from "../lib/polls";
  import type { Poll } from "../lib/protocol";
  import { fullTimestampLabel } from "./format";
  import Icon from "./Icon.svelte";
  import { pollClockNow, watchPollClock } from "./poll-clock.svelte";
  import { pollEndUnixMilliseconds, pollResults, pollTimeLeft, pollTimeLeftLabel } from "./poll-view";

  // A poll in a message (DESIGN.md §5.4, docs/design/web-polls.md, ADR-0043): the question, one row
  // per answer, and a footer. Results stay hidden until the reader votes, asks to see them, or the
  // poll ends, as on Discord; then each row fills from its leading side like a level.
  interface Props {
    poll: Poll;
    /** Still in the outbox: shown as it will arrive, nothing to press yet. */
    isPending?: boolean;
    /** Whoever reads a server channel, or writes in a conversation (ADR-0043). */
    canVote: boolean;
    onchoose?: (answerId: number) => void;
    onremovevote?: () => void;
    onviewvoters?: (answerId: number) => void;
  }

  let { poll, isPending = false, canVote, onchoose, onremovevote, onviewvoters }: Props = $props();

  const id = $props.id();
  const results = $derived(pollResults(poll));
  const isEnded = $derived(isPollEnded(poll, pollClockNow()));
  const timeLeft = $derived(pollTimeLeft(poll, pollClockNow()));
  const canVoteNow = $derived(canVote && !isPending && !isEnded);
  let isRevealed = $state(false);
  const areResultsShown = $derived(!isPending && (isEnded || results.hasReaderVoted || isRevealed));
  const role = $derived(poll.allows_multiple_answers ? "checkbox" : "radio");
  // A radio group is one Tab stop: the checked answer, else the first.
  const tabStopAnswerId = $derived(
    poll.answers.find((answer) => answer.me_voted)?.id ?? poll.answers[0]?.id ?? null,
  );

  // Only an open poll follows the shared minute clock; one that ended has nothing left to count.
  $effect(() => {
    if (isPending || isEnded) {
      return;
    }
    return watchPollClock();
  });

  function choose(answerId: number) {
    if (canVoteNow) {
      onchoose?.(answerId);
    }
  }

  /** Arrows move between the answers without voting: a vote is a request, so Space or Enter casts it. */
  function moveRadioFocus(event: KeyboardEvent & { currentTarget: HTMLElement }) {
    if (poll.allows_multiple_answers) {
      return;
    }
    const step =
      event.key === "ArrowDown" || event.key === "ArrowRight" ? 1 : event.key === "ArrowUp" || event.key === "ArrowLeft" ? -1 : 0;
    if (step === 0) {
      return;
    }
    const radios = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("[role=radio]")];
    const current = radios.indexOf(document.activeElement as HTMLButtonElement);
    // The voters buttons sit in the group too: arrows there keep their usual meaning.
    if (current === -1) {
      return;
    }
    event.preventDefault();
    radios[(current + step + radios.length) % radios.length]?.focus();
  }
</script>

<section
  aria-labelledby="{id}-question"
  class="mt-3xs flex w-full max-w-(--mw-component-poll-max-width) flex-col gap-(--mw-component-poll-gap) rounded-(--mw-component-poll-radius) border-(length:--mw-component-poll-border-width) border-(--mw-component-poll-border) bg-(--mw-component-poll-background) p-(--mw-component-poll-padding)"
>
  <div class="flex flex-col gap-(--mw-component-poll-header-gap)">
    <h4 id="{id}-question" class="type-heading-md break-words text-(--mw-component-poll-question-text)">{poll.question}</h4>
    {#if !isEnded}
      <p id="{id}-hint" class="type-body-sm text-(--mw-component-poll-hint-text)">
        {poll.allows_multiple_answers ? strings.polls.chooseSeveral : strings.polls.chooseOne}
      </p>
    {/if}
  </div>

  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <div
    role={poll.allows_multiple_answers ? "group" : "radiogroup"}
    aria-labelledby="{id}-question"
    aria-describedby={isEnded ? undefined : `${id}-hint`}
    class="flex flex-col gap-(--mw-component-poll-answer-gap)"
    onkeydown={moveRadioFocus}
  >
    {#each results.answers as { answer, percent, isLeading } (answer.id)}
      {@const isWinner = isEnded && isLeading}
      <div
        class={[
          "relative flex min-h-(--mw-component-poll-answer-min-height) items-stretch overflow-hidden rounded-(--mw-component-poll-answer-radius) border-(length:--mw-component-poll-answer-border-width) bg-(--mw-component-poll-answer-background) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) pointer-coarse:min-h-(--mw-size-hit-web-min)",
          answer.me_voted
            ? "border-(--mw-component-poll-answer-border-mine)"
            : canVoteNow
              ? "border-(--mw-component-poll-answer-border) hover:border-(--mw-component-poll-answer-border-hover)"
              : "border-(--mw-component-poll-answer-border)",
        ]}
      >
        {#if areResultsShown}
          <span
            aria-hidden="true"
            class={[
              "poll-bar absolute inset-y-0 start-0",
              answer.me_voted ? "bg-(--mw-component-poll-bar-mine)" : "bg-(--mw-component-poll-bar)",
            ]}
            style:--poll-share="{percent}%"
          ></span>
        {/if}
        <button
          type="button"
          {role}
          aria-checked={answer.me_voted}
          aria-disabled={!canVoteNow || undefined}
          aria-describedby={areResultsShown ? `${id}-results-${answer.id}` : undefined}
          tabindex={poll.allows_multiple_answers || answer.id === tabStopAnswerId ? 0 : -1}
          class={[
            "relative flex min-w-0 flex-1 items-center gap-(--mw-component-poll-answer-inner-gap) px-(--mw-component-poll-answer-padding-inline) py-(--mw-component-poll-answer-padding-block) text-start focus-visible:focus-ring-inset",
            canVoteNow ? "cursor-pointer" : "cursor-default",
          ]}
          onclick={() => choose(answer.id)}
        >
          <span
            aria-hidden="true"
            class={[
              "grid size-(--mw-component-poll-mark-size) shrink-0 place-items-center",
              poll.allows_multiple_answers ? "rounded-(--mw-component-poll-mark-radius-checkbox)" : "rounded-full",
              answer.me_voted
                ? "bg-(--mw-component-poll-mark-checked) text-(--mw-component-poll-mark-glyph)"
                : "border-(length:--mw-component-poll-mark-border-width) border-(--mw-component-poll-mark-border)",
            ]}
          >
            {#if answer.me_voted}
              {#if poll.allows_multiple_answers}
                <Icon name="check" class="size-(--mw-component-poll-mark-glyph-size)" />
              {:else}
                <span class="size-[calc(var(--mw-component-poll-mark-glyph-size)/2)] rounded-full bg-current"></span>
              {/if}
            {/if}
          </span>
          <span class={["min-w-0 type-message break-words text-(--mw-component-poll-answer-text)", isWinner && "font-semibold"]}
            >{answer.text}</span
          >
          {#if isWinner}
            <Icon name="crown" class="size-(--mw-component-poll-leader-icon-size) shrink-0 text-(--mw-component-poll-leader-icon)" />
            <span class="sr-only">{strings.polls.winner}</span>
          {/if}
        </button>
        {#if areResultsShown}
          {@const count = strings.polls.votes(answer.vote_count)}
          {@const share = strings.polls.percent(percent)}
          <span id="{id}-results-{answer.id}" class="sr-only">{strings.polls.answerResults(answer.vote_count, percent)}</span>
          {#snippet figures()}
            <span aria-hidden="true" class="type-body-sm text-(--mw-component-poll-count-text)">{count}</span>
            <span aria-hidden="true" class="type-body font-semibold text-(--mw-component-poll-percent-text)">{share}</span>
          {/snippet}
          {#if answer.vote_count > 0 && onviewvoters}
            <button
              type="button"
              aria-label="{strings.polls.answerResults(answer.vote_count, percent)}{strings.polls.seeVoters(answer.text)}"
              class="relative flex min-w-(--mw-component-poll-count-min-width) shrink-0 flex-col items-end justify-center rounded-(--mw-component-poll-answer-radius) px-(--mw-component-poll-answer-padding-inline) tabular-nums hover:underline focus-visible:focus-ring-inset"
              onclick={() => onviewvoters?.(answer.id)}>{@render figures()}</button
            >
          {:else}
            <span
              class="relative flex min-w-(--mw-component-poll-count-min-width) shrink-0 flex-col items-end justify-center px-(--mw-component-poll-answer-padding-inline) tabular-nums"
              >{@render figures()}</span
            >
          {/if}
        {/if}
      </div>
    {/each}
  </div>

  <div class="flex flex-wrap items-center justify-between gap-x-(--mw-component-poll-footer-gap) gap-y-3xs">
    <p class="type-body-sm text-(--mw-component-poll-footer-text)">
      {strings.polls.votes(results.totalVotes)} ·
      <time
        datetime={new Date(pollEndUnixMilliseconds(poll)).toISOString()}
        title={(isEnded ? strings.polls.endedAt : strings.polls.endsAt)(fullTimestampLabel(pollEndUnixMilliseconds(poll)))}
        >{pollTimeLeftLabel(timeLeft)}</time
      >
    </p>
    {#if !isPending && !isEnded}
      {@const actionClass =
        "inline-flex min-h-(--mw-size-hit-desktop-min) items-center rounded-xs type-body-sm font-semibold text-text-link hover:underline focus-visible:focus-ring pointer-coarse:min-h-(--mw-size-hit-web-min)"}
      <div class="flex flex-wrap gap-x-(--mw-component-poll-footer-gap)">
        {#if results.hasReaderVoted}
          {#if canVote}
            <button type="button" class={actionClass} onclick={() => onremovevote?.()}>{strings.polls.removeVote}</button>
          {/if}
        {:else}
          <button type="button" class={actionClass} onclick={() => (isRevealed = !isRevealed)}
            >{isRevealed ? strings.polls.hideResults : strings.polls.showResults}</button
          >
        {/if}
      </div>
    {/if}
  </div>
</section>

<style>
  /* The level follows each vote, and grows from nothing when results appear; at once under
     reduced motion, where the duration token is 0. */
  .poll-bar {
    inline-size: var(--poll-share);
    transition: inline-size var(--mw-motion-duration-slow) var(--mw-motion-easing-standard);
  }
  @starting-style {
    .poll-bar {
      inline-size: 0;
    }
  }
</style>

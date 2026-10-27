<script lang="ts">
  import { onDestroy } from "svelte";

  import { strings } from "../i18n";
  import type { PublicUser, ReactionSummary, Snowflake } from "../lib/protocol";
  import { displayEmoji } from "../lib/reactions";
  import { displayName } from "./format";
  import Icon from "./Icon.svelte";
  import { createPopoverToggle } from "./popover-toggle";
  import { durationTokenInMilliseconds, lengthTokenInPixels } from "./tokens";

  // Reaction chips under a message (DESIGN.md §5.4): emoji and count, pressed when the user is
  // among them; a click adds or takes back their reaction. Hovering or focusing a chip names who
  // reacted, asked from the instance only then.
  interface Props {
    reactions: readonly ReactionSummary[];
    currentUserId: Snowflake;
    /** Joining an existing reaction: the user sees the message's history. */
    canJoin: boolean;
    /** A new emoji also needs `ADD_REACTIONS`. */
    canAddNew: boolean;
    /** The picker it opened is open: the add button stays, for focus to come back to. */
    isHeld: boolean;
    ontoggle: (emoji: string) => void;
    onopenpicker: (anchor: HTMLElement) => void;
    loadUsers: (emoji: string) => Promise<PublicUser[]>;
  }

  let { reactions, currentUserId, canJoin, canAddNew, isHeld, ontoggle, onopenpicker, loadUsers }: Props = $props();

  /** Others named before "and 3 others", as on Discord. */
  const NAMED_PEOPLE = 3;
  const id = $props.id();
  let tooltip: HTMLElement;
  let tooltipText = $state("");
  let tooltipEmoji = $state<string | null>(null);
  let tooltipPosition = $state({ top: 0, left: 0 });
  let showTimer: ReturnType<typeof setTimeout> | undefined;
  /** Who reacted, per emoji, for the count it had: a new count asks again. */
  const reactorSentenceByEmoji = new Map<string, { count: number; text: string }>();
  let list = $state<HTMLElement | null>(null);
  const pickerToggle = createPopoverToggle(() => isHeld);

  // A chip that went (its last reaction removed, here or elsewhere) takes its tooltip with it.
  $effect(() => {
    if (tooltipEmoji !== null && !reactions.some((reaction) => reaction.emoji === tooltipEmoji)) {
      hideTooltip();
    }
  });

  onDestroy(hideTooltip);

  /**
   * Taking back the last reaction of an emoji removes its chip, focus with it: focus moves first
   * to the add button, or to the message.
   */
  function toggle(chip: HTMLElement, reaction: ReactionSummary) {
    if (reaction.me && reaction.count === 1 && chip.matches(":focus")) {
      const addButton = list?.querySelector<HTMLElement>("[data-add-reaction]");
      (addButton ?? chip.closest<HTMLElement>("[data-row-key]"))?.focus();
      hideTooltip();
    }
    ontoggle(reaction.emoji);
  }

  function sentenceFor(reaction: ReactionSummary, users: readonly PublicUser[]): string {
    const others = users.filter((user) => user.id !== currentUserId);
    const named = others.slice(0, NAMED_PEOPLE).map(displayName);
    const unnamed = Math.max(reaction.count - (reaction.me ? 1 : 0) - named.length, 0);
    return strings.reactions.reactedWith(reaction.me, named, unnamed, displayEmoji(reaction.emoji));
  }

  function showTooltip(chip: HTMLElement, reaction: ReactionSummary) {
    clearTimeout(showTimer);
    showTimer = setTimeout(() => void openTooltip(chip, reaction), durationTokenInMilliseconds("motion-delay-tooltip"));
  }

  async function openTooltip(chip: HTMLElement, reaction: ReactionSummary) {
    tooltipEmoji = reaction.emoji;
    const cached = reactorSentenceByEmoji.get(reaction.emoji);
    tooltipText = cached?.count === reaction.count ? cached.text : strings.reactions.loadingWho;
    if (!tooltip.matches(":popover-open")) {
      tooltip.showPopover();
    }
    const chipBounds = chip.getBoundingClientRect();
    const tooltipBounds = tooltip.getBoundingClientRect();
    tooltipPosition = {
      top: Math.max(chipBounds.top - tooltipBounds.height - lengthTokenInPixels("space-2xs"), 0),
      left: Math.min(
        Math.max(chipBounds.left + chipBounds.width / 2 - tooltipBounds.width / 2, 0),
        window.innerWidth - tooltipBounds.width,
      ),
    };
    if (cached?.count === reaction.count) {
      return;
    }
    try {
      const text = sentenceFor(reaction, await loadUsers(reaction.emoji));
      reactorSentenceByEmoji.set(reaction.emoji, { count: reaction.count, text });
      if (tooltipEmoji === reaction.emoji) {
        tooltipText = text;
      }
    } catch (error) {
      console.warn("loading who reacted failed", error);
      if (tooltipEmoji === reaction.emoji) {
        tooltipText = strings.reactions.whoFailed;
      }
    }
  }

  function hideTooltip() {
    clearTimeout(showTimer);
    tooltipEmoji = null;
    if (tooltip?.matches(":popover-open")) {
      tooltip.hidePopover();
    }
  }
</script>

{#if reactions.length > 0}
  <ul
    bind:this={list}
    aria-label={strings.reactions.reactions}
    class="mt-(--mw-component-reaction-margin-top) flex flex-wrap items-center gap-(--mw-component-reaction-row-gap)"
  >
    {#each reactions as reaction (reaction.emoji)}
      {@const isUsable = reaction.me || canJoin}
      <li>
        <button
          type="button"
          aria-pressed={reaction.me}
          aria-disabled={!isUsable}
          aria-label={strings.reactions.chip(displayEmoji(reaction.emoji), reaction.count)}
          aria-describedby={tooltipEmoji === reaction.emoji ? `${id}-tooltip` : undefined}
          class={[
            "flex h-(--mw-component-reaction-height) items-center gap-(--mw-component-reaction-gap) rounded-(--mw-component-reaction-radius) border-(length:--mw-component-reaction-border-width) px-(--mw-component-reaction-padding-inline) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) focus-visible:focus-ring pointer-coarse:h-(--mw-size-hit-web-min)",
            reaction.me
              ? "border-(--mw-component-reaction-border-me) bg-(--mw-component-reaction-background-me) text-(--mw-component-reaction-count-text-me)"
              : "border-(--mw-component-reaction-border) bg-(--mw-component-reaction-background) text-(--mw-component-reaction-count-text)",
            isUsable && !reaction.me && "hover:border-(--mw-component-reaction-border-hover) hover:bg-(--mw-component-reaction-background-hover)",
          ]}
          onclick={(event) => isUsable && toggle(event.currentTarget, reaction)}
          onpointerenter={(event) => showTooltip(event.currentTarget, reaction)}
          onpointerleave={hideTooltip}
          onfocus={(event) => showTooltip(event.currentTarget, reaction)}
          onblur={hideTooltip}
        >
          <span aria-hidden="true" class="text-(length:--mw-component-reaction-emoji-size) leading-none"
            >{displayEmoji(reaction.emoji)}</span
          >
          <span aria-hidden="true" class="type-body-sm font-semibold tabular-nums">{reaction.count}</span>
        </button>
      </li>
    {/each}
    {#if canAddNew}
      <li class={["invisible group-focus-within:visible group-hover:visible pointer-coarse:visible", isHeld && "visible"]}>
        <button
          type="button"
          data-add-reaction
          aria-label={strings.reactions.addReaction}
          title={strings.reactions.addReaction}
          class="grid h-(--mw-component-reaction-height) place-items-center rounded-(--mw-component-reaction-radius) border-(length:--mw-component-reaction-border-width) border-(--mw-component-reaction-border) bg-(--mw-component-reaction-background) px-(--mw-component-reaction-padding-inline) text-(--mw-component-reaction-add-icon) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:border-(--mw-component-reaction-border-hover) hover:text-(--mw-component-reaction-add-icon-hover) focus-visible:focus-ring pointer-coarse:h-(--mw-size-hit-web-min) pointer-coarse:min-w-(--mw-size-hit-web-min)"
          onpointerdown={pickerToggle.notePress}
          onclick={(event) => pickerToggle.shouldOpenOnClick() && onopenpicker(event.currentTarget)}
        >
          <Icon name="smiley" class="size-(--mw-component-reaction-emoji-size)" />
        </button>
      </li>
    {/if}
  </ul>
{/if}

<div
  bind:this={tooltip}
  id="{id}-tooltip"
  popover="manual"
  role="tooltip"
  class="fixed inset-auto m-0 max-w-(--mw-component-tooltip-max-width) rounded-(--mw-component-tooltip-radius) bg-(--mw-component-tooltip-background) px-(--mw-component-tooltip-padding-inline) py-(--mw-component-tooltip-padding-block) type-tooltip text-(--mw-component-tooltip-text) shadow-(--mw-component-tooltip-shadow)"
  style:top="{tooltipPosition.top}px"
  style:left="{tooltipPosition.left}px"
>
  {tooltipText}
</div>

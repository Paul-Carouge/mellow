<script lang="ts">
  import { strings } from "../i18n";
  import { describeError } from "../lib/client.svelte";
  import type { PublicUser, ReactionSummary, Snowflake } from "../lib/protocol";
  import { displayEmoji } from "../lib/reactions";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { displayName } from "./format";
  import ModalDialog from "./ModalDialog.svelte";

  // Who reacted with what (DESIGN.md §5.4), one emoji at a time as tabs; moderators can remove
  // someone's reaction from here.
  interface Props {
    reactions: readonly ReactionSummary[];
    canRemoveOthers: boolean;
    loadUsers: (emoji: string) => Promise<PublicUser[]>;
    onremove: (emoji: string, userId: Snowflake) => Promise<void>;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { reactions, canRemoveOthers, loadUsers, onremove, onclose, fallbackFocus }: Props = $props();

  const id = $props.id();
  let chosenEmoji = $state<string | null>(null);
  // A removed emoji falls back to the first one left.
  const selectedEmoji = $derived(
    reactions.find((reaction) => reaction.emoji === chosenEmoji)?.emoji ?? reactions[0]?.emoji ?? null,
  );
  const selectedCount = $derived(reactions.find((reaction) => reaction.emoji === selectedEmoji)?.count ?? 0);
  let users = $state<PublicUser[]>([]);
  let loadState = $state<"loading" | "loaded" | "failed">("loading");
  let errorMessage = $state("");

  // Asked again whenever the emoji or its count changes, so removals and new reactions show.
  $effect(() => {
    const emoji = selectedEmoji;
    void selectedCount;
    if (!emoji) {
      return;
    }
    let isCurrent = true;
    loadState = "loading";
    loadUsers(emoji).then(
      (loaded) => {
        if (isCurrent) {
          users = loaded;
          loadState = "loaded";
        }
      },
      (error: unknown) => {
        console.warn("loading who reacted failed", error);
        if (isCurrent) {
          loadState = "failed";
        }
      },
    );
    return () => {
      isCurrent = false;
    };
  });

  async function remove(emoji: string, user: PublicUser) {
    errorMessage = "";
    try {
      await onremove(emoji, user.id);
    } catch (error) {
      errorMessage = describeError(error, strings.reactions.removeFailed);
    }
  }

  function moveTabFocus(event: KeyboardEvent) {
    const tabs = [...(event.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>("[role=tab]")];
    const current = tabs.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === "ArrowRight"
        ? (current + 1) % tabs.length
        : event.key === "ArrowLeft"
          ? (current - 1 + tabs.length) % tabs.length
          : null;
    if (next !== null) {
      event.preventDefault();
      tabs[next]?.focus();
      tabs[next]?.click();
    }
  }
</script>

<ModalDialog title={strings.reactions.reactions} cancelLabel={strings.common.close} {errorMessage} {onclose} {fallbackFocus}>
  {#if reactions.length === 0}
    <p class="type-body text-text-muted">{strings.reactions.noReactions}</p>
  {:else}
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div role="tablist" aria-label={strings.reactions.reactions} class="flex flex-wrap gap-(--mw-component-reaction-row-gap)" onkeydown={moveTabFocus}>
      {#each reactions as reaction (reaction.emoji)}
        {@const isSelected = reaction.emoji === selectedEmoji}
        <button
          type="button"
          role="tab"
          id="{id}-tab-{reaction.emoji}"
          aria-selected={isSelected}
          aria-controls="{id}-panel"
          aria-label={strings.reactions.chip(displayEmoji(reaction.emoji), reaction.count)}
          tabindex={isSelected ? 0 : -1}
          class={[
            "flex h-(--mw-component-reaction-height) items-center gap-(--mw-component-reaction-gap) rounded-(--mw-component-reaction-radius) border-(length:--mw-component-reaction-border-width) px-(--mw-component-reaction-padding-inline) focus-visible:focus-ring pointer-coarse:h-(--mw-size-hit-web-min)",
            isSelected
              ? "border-(--mw-component-reaction-border-me) bg-(--mw-component-reaction-background-me) text-(--mw-component-reaction-count-text-me)"
              : "border-(--mw-component-reaction-border) bg-(--mw-component-reaction-background) text-(--mw-component-reaction-count-text) hover:border-(--mw-component-reaction-border-hover)",
          ]}
          onclick={() => (chosenEmoji = reaction.emoji)}
        >
          <span aria-hidden="true" class="text-(length:--mw-component-reaction-emoji-size) leading-none">{displayEmoji(reaction.emoji)}</span>
          <span aria-hidden="true" class="type-body-sm font-semibold tabular-nums">{reaction.count}</span>
        </button>
      {/each}
    </div>
    <div id="{id}-panel" role="tabpanel" aria-labelledby="{id}-tab-{selectedEmoji}" class="min-h-0">
      {#if loadState === "loading"}
        <p role="status" class="type-body text-text-muted">{strings.reactions.loadingWho}</p>
      {:else if loadState === "failed"}
        <p class="type-body text-text-muted">{strings.reactions.whoFailed}</p>
      {:else}
        <ul class="flex flex-col">
          {#each users as user (user.id)}
            <li class="flex min-h-(--mw-component-member-row-height) items-center gap-sm pointer-coarse:min-h-(--mw-size-hit-web-min)">
              <Avatar {user} sizeToken="--mw-component-member-row-avatar-size" />
              <span class="min-w-0 flex-1 truncate type-member-name text-text-normal">{displayName(user)}</span>
              {#if canRemoveOthers && selectedEmoji}
                {@const emoji = selectedEmoji}
                <Button variant="ghost" aria-label={strings.reactions.remove(displayName(user))} onclick={() => void remove(emoji, user)}
                  >{strings.reactions.removeShort}</Button
                >
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</ModalDialog>

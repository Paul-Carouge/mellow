<script lang="ts">
  import { untrack } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { Call, DirectMessageChannel, Snowflake } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { ringKeyOf } from "../lib/calls";
  import { callPersonOf, incomingCallSentence, incomingCardFocus } from "./calls-view";
  import { displayName } from "./format";
  import GroupAvatar from "./GroupAvatar.svelte";
  import { conversationDisplayName, isGroupConversation, otherParticipants } from "./group-conversations";
  import Icon from "./Icon.svelte";
  import Melli from "./Melli.svelte";

  // The incoming call card (DESIGN.md §5.6.3, ADR-0062): floating at the top of the window wherever
  // the reader is, the caller's face with Melli ringing on it, then Decline and Answer. It does not
  // block the app: a call is an interruption, not a question that must be answered first. Focus
  // goes to Answer unless the reader is typing (Enter would then answer by accident); Ctrl+Enter
  // answers from anywhere and the announcement is read out (both the shell's, which loads this card
  // only while a call rings); Esc in the card declines. Under Do not disturb it comes without sound
  // and without taking focus. The ringtone and the notification are `call-alerts.svelte.ts`'s.
  interface Props {
    client: MellowClient;
  }

  let { client }: Props = $props();

  const call = $derived(client.incomingCalls[0] ?? null);
  const conversation = $derived(
    call ? (client.conversations.find((candidate) => candidate.id === call.channel_id) ?? null) : null,
  );
  const currentUserId = $derived(client.user?.id ?? null);
  const isDoNotDisturb = $derived(client.user?.status === "do_not_disturb");
  /** Answering moves the reader's voice out of where it is now: said before they press it. */
  const placeLeftByAnswering = $derived.by(() => {
    if (client.voice.phase === "idle") {
      return null;
    }
    const conversationInCall = client.callConversation;
    if (conversationInCall) {
      return conversationDisplayName(conversationInCall, currentUserId);
    }
    return client.voiceChannel?.channel.name ?? null;
  });

  let isAnswering = $state(false);

  function isTyping(element: Element | null): boolean {
    return element instanceof HTMLElement && (element.isContentEditable || element.matches("input, textarea, select"));
  }

  /**
   * Shows the card for one ringing call: over an open dialog or drawer as a stacked modal (anything
   * else under a modal is inert), otherwise in place without taking the page. Focus goes to Answer,
   * stays where it was while the reader types or under Do not disturb, or, over a modal (which
   * would otherwise focus Decline), rests on the card itself so Space and Enter press nothing. It
   * comes back where it was when the card goes.
   */
  function openCard(dialog: HTMLDialogElement, ringing: Call, isQuiet: boolean): () => void {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const isOverModal = document.querySelector("dialog:modal") !== null;
    if (isOverModal) {
      dialog.showModal();
    } else {
      dialog.setAttribute("open", "");
    }
    const focus = incomingCardFocus({ isOverModal, isTyping: isTyping(opener), isQuiet });
    if (focus === "answer") {
      dialog.querySelector<HTMLButtonElement>("[data-answer]")?.focus();
    } else if (focus === "card") {
      dialog.focus();
    }
    return () => {
      // The card may already be out of the page, which drops its focus to the body.
      const hadFocus = dialog.contains(document.activeElement) || document.activeElement === document.body;
      dialog.close();
      if (hadFocus && opener?.isConnected) {
        opener.focus();
      }
    };
  }

  async function answer(channelId: Snowflake) {
    if (isAnswering) {
      return;
    }
    isAnswering = true;
    try {
      await client.answerCall(channelId);
    } finally {
      isAnswering = false;
    }
  }

  function decline(channelId: Snowflake) {
    void client.declineCall(channelId);
  }

  /** Esc declines only from inside the card: elsewhere it keeps closing what it closes. */
  function declineOnEscape(event: KeyboardEvent & { currentTarget: HTMLDialogElement }, channelId: Snowflake) {
    if (event.key === "Escape" && event.currentTarget.contains(document.activeElement)) {
      event.preventDefault();
      decline(channelId);
    }
  }

</script>

{#if call && conversation}
  {@render card(call, conversation)}
{/if}

{#snippet card(ringing: Call, place: DirectMessageChannel)}
  {@const isGroup = isGroupConversation(place)}
  {@const caller = callPersonOf(place, ringing.caller_id)}
  <!-- One card per ringing call and quiet state: a CALL_UPDATE (someone else answering) leaves it be. -->
  {#key `${ringKeyOf(ringing)}:${isDoNotDisturb}`}
    <dialog
      tabindex="-1"
      {@attach (dialog) => untrack(() => openCard(dialog, ringing, isDoNotDisturb))}
      aria-label={strings.calls.incoming}
      aria-describedby="incoming-call-sentence"
      class="incoming-call fixed inset-x-0 bottom-auto top-[calc(env(safe-area-inset-top)+var(--mw-component-incoming-call-margin))] z-50 mx-auto w-[min(var(--mw-component-incoming-call-width),calc(100%-2*var(--mw-component-incoming-call-margin)))] flex-col gap-(--mw-component-incoming-call-gap) rounded-(--mw-component-incoming-call-radius) border-(length:--mw-component-incoming-call-border-width) border-(--mw-component-incoming-call-border) bg-(--mw-component-incoming-call-background) p-(--mw-component-incoming-call-padding) text-text-normal shadow-(--mw-component-incoming-call-shadow) open:flex"
      oncancel={(event) => event.preventDefault()}
      onkeydown={(event) => declineOnEscape(event, ringing.channel_id)}
    >
      <div class="flex min-w-0 items-center gap-sm">
        <span class="shrink-0">
          {#if isGroup}
            <GroupAvatar
              people={otherParticipants(place, currentUserId)}
              sizeToken="--mw-component-incoming-call-avatar-size"
              ringColorToken="--mw-component-incoming-call-background"
            />
          {:else}
            <Avatar user={caller} sizeToken="--mw-component-incoming-call-avatar-size" animation="always" />
          {/if}
        </span>
        <div class="flex min-w-0 flex-1 flex-col">
          <p class="truncate type-heading-md text-(--mw-component-incoming-call-name-text)" title={displayName(caller)}>
            {displayName(caller)}
          </p>
          <p id="incoming-call-sentence" class="truncate type-body text-(--mw-component-incoming-call-sentence-text)">
            <span class="sr-only">{incomingCallSentence(ringing, place, currentUserId)}</span>
            <span aria-hidden="true">{isGroup ? strings.calls.inGroup(conversationDisplayName(place, currentUserId)) : strings.calls.isCallingYou}</span>
          </p>
        </div>
        <!-- Melli rings the phone: still and quiet under Do not disturb. -->
        <Melli state="incoming-call" sizeToken="--mw-component-incoming-call-melli-size" isStill={isDoNotDisturb} />
      </div>
      <div class="flex gap-(--mw-component-incoming-call-action-gap) *:flex-1">
        <button
          type="button"
          aria-keyshortcuts="Escape"
          class="decline inline-flex h-(--mw-component-button-height-large) items-center justify-center gap-2xs rounded-(--mw-component-button-radius) px-(--mw-component-button-padding-inline) type-button"
          onclick={() => decline(ringing.channel_id)}
        >
          <Icon name="phone-disconnect" class="size-(--mw-component-incoming-call-hint-icon-size) shrink-0" />
          {strings.calls.decline}
        </button>
        <Button
          data-answer
          variant="primary"
          aria-keyshortcuts="Control+Enter"
          isDisabled={isAnswering}
          onclick={() => void answer(ringing.channel_id)}
        >
          <Icon name="phone" class="me-2xs size-(--mw-component-incoming-call-hint-icon-size) shrink-0" />
          {strings.calls.answer}
        </Button>
      </div>
      {#if isDoNotDisturb || placeLeftByAnswering}
        <p class="-mt-2xs flex items-center gap-2xs type-body-sm text-(--mw-component-incoming-call-sentence-text)">
          <Icon name={isDoNotDisturb ? "bell-slash" : "phone-call"} class="size-(--mw-component-incoming-call-hint-icon-size) shrink-0" />
          <span class="min-w-0">
            {isDoNotDisturb ? strings.calls.silentForDoNotDisturb : strings.calls.answerLeaves(placeLeftByAnswering ?? "")}
          </span>
        </p>
      {/if}
    </dialog>
  {/key}
{/snippet}

<style>
  .incoming-call {
    /* A <dialog> shown over another modal keeps the page behind it as it was. */
    &::backdrop {
      background: transparent;
    }
  }
  .incoming-call[open] {
    animation: incoming-call-land var(--mw-component-incoming-call-enter-duration) var(--mw-motion-easing-settle);
  }
  @keyframes incoming-call-land {
    from {
      opacity: 0;
      transform: translateY(var(--mw-component-incoming-call-enter-offset)) scale(var(--mw-component-dialog-enter-scale));
    }
  }
  .decline {
    background: var(--mw-component-incoming-call-decline-background);
    color: var(--mw-component-incoming-call-decline-text);
    transition:
      background-color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard),
      color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard);
    &:hover {
      background: var(--mw-component-incoming-call-decline-background-hover);
      color: var(--mw-component-incoming-call-decline-text-hover);
    }
  }
</style>

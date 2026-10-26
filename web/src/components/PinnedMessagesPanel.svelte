<script lang="ts">
  import { onMount } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { Message, Snowflake } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import {
    attachmentCountLabel,
    displayName,
    fullTimestampLabel,
    messageTimestampLabel,
    unixMillisecondsOfSnowflake,
  } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";

  // The pinned messages of the open channel (DESIGN.md §6.53), from the chat header's pin button:
  // newest pin first, with Jump for the ones loaded in the list and Unpin for those who may.
  interface Props {
    client: MellowClient;
    channelId: Snowflake;
    anchor: HTMLElement;
    isCompact: boolean;
    canManagePins: boolean;
    /** Scrolls to the message in the list and focuses it. */
    onjump: (messageId: Snowflake) => void;
    onunpin: (message: Message) => void;
    /**
     * Closed by Esc, a click outside, the close button or Jump; the caller puts focus back only
     * if it was in the panel (or nowhere): a click elsewhere and Jump keep the focus they gave.
     */
    onclose: (shouldReturnFocus: boolean) => void;
  }

  let { client, channelId, anchor, isCompact, canManagePins, onjump, onunpin, onclose }: Props = $props();

  const id = $props.id();
  let panel: HTMLElement;
  let position = $state({ top: 0, right: 0 });
  const pins = $derived(client.pinsByChannel.get(channelId) ?? { status: "loading" as const });
  const loadedIds = $derived(new Set(client.messages(channelId).map((message) => message.id)));

  // Asked again whenever the list is gone, as after a `Ready` while the panel is open.
  $effect(() => {
    if (!client.pinsByChannel.has(channelId)) {
      void client.loadPins(channelId);
    }
  });

  onMount(() => {
    const bounds = anchor.getBoundingClientRect();
    position = { top: bounds.bottom, right: Math.max(window.innerWidth - bounds.right, 0) };
    panel.showPopover();
    panel.focus();
  });

  function pinnedMessageSummaryText(message: Message): string {
    // Folded like the list and reply previews: Jump still shows it on request (ADR-0025).
    if (client.isBlocked(message.author.id)) {
      return strings.messages.blockedReply;
    }
    return [message.content, attachmentCountLabel(message.attachments?.length ?? 0)].filter(Boolean).join(" ");
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  bind:this={panel}
  popover="auto"
  role="dialog"
  tabindex="-1"
  aria-labelledby="{id}-title"
  class={[
    "fixed m-0 flex-col border-(length:--mw-component-pins-border-width) border-(--mw-component-pins-border) bg-(--mw-component-pins-background) shadow-(--mw-component-pins-shadow) outline-none [&:popover-open]:flex",
    isCompact
      ? "inset-x-0 top-auto bottom-0 max-h-[80dvh] w-full max-w-none rounded-t-(--mw-component-pins-radius) [&:popover-open]:animate-[mw-rise_var(--mw-motion-duration-base)_var(--mw-motion-easing-enter)_both]"
      : "inset-auto max-h-[min(var(--mw-component-pins-max-height),calc(100dvh-2*var(--mw-component-chat-header-height)))] w-(--mw-component-pins-width) max-w-[calc(100vw-2*var(--mw-space-xs))] rounded-(--mw-component-pins-radius) [&:popover-open]:animate-[mw-drop-in_var(--mw-motion-duration-fast)_var(--mw-motion-easing-enter)_both]",
  ]}
  style:top={isCompact ? undefined : `${position.top}px`}
  style:right={isCompact ? undefined : `${position.right}px`}
  ontoggle={(event) => {
    if (event.newState === "closed") {
      onclose(panel.contains(document.activeElement) || document.activeElement === document.body);
    }
  }}
>
  <header class="flex h-(--mw-component-pins-header-height) shrink-0 items-center gap-xs ps-sm pe-2xs">
    <Icon name="push-pin" class="size-(--mw-component-chat-header-icon-size) text-text-muted" />
    <h2 id="{id}-title" class="min-w-0 flex-1 truncate type-heading-md text-(--mw-component-pins-title-text)">
      {strings.pins.pinnedMessages}
    </h2>
    <IconButton
      icon="x"
      label={strings.common.close}
      class="size-(--mw-component-chat-header-button-size) text-(--mw-component-chat-header-icon) hover:bg-state-hover hover:text-text-normal"
      iconClass="size-(--mw-component-chat-header-icon-size)"
      onclick={() => panel.hidePopover()}
    />
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto px-(--mw-component-pins-padding) pb-(--mw-component-pins-padding) [scrollbar-width:thin]">
    {#if pins.status === "loading"}
      <p role="status" class="p-sm type-body text-(--mw-component-pins-empty-text)">{strings.pins.loading}</p>
    {:else if pins.status === "failed"}
      <div role="alert" class="flex flex-col items-start gap-sm p-sm">
        <p class="type-body text-(--mw-component-pins-empty-text)">{strings.pins.loadFailed}</p>
        <Button onclick={() => void client.loadPins(channelId, true)}>{strings.common.tryAgain}</Button>
      </div>
    {:else if pins.messages.length === 0}
      <div class="flex flex-col items-center gap-xs px-md py-xl text-center">
        <Icon name="push-pin" class="size-(--mw-component-pins-empty-icon-size) text-(--mw-component-pins-empty-icon)" />
        <p class="type-heading-md text-text-heading">{strings.pins.emptyTitle}</p>
        <p class="type-body text-(--mw-component-pins-empty-text)">{strings.pins.emptyBody}</p>
      </div>
    {:else}
      <ul class="flex flex-col gap-(--mw-component-pins-item-gap)">
        {#each pins.messages as message (message.id)}
          {@const sentAt = unixMillisecondsOfSnowflake(message.id)}
          <li
            class="flex gap-sm rounded-(--mw-component-pins-item-radius) bg-(--mw-component-pins-item-background) p-(--mw-component-pins-item-padding)"
          >
            <Avatar user={message.author} sizeToken="--mw-component-pins-item-avatar-size" />
            <div class="flex min-w-0 flex-1 flex-col gap-2xs">
              <p class="flex flex-wrap items-baseline gap-x-xs">
                <span class="type-message-author text-(--mw-component-message-author-text)">{displayName(message.author)}</span>
                <time
                  datetime={new Date(sentAt).toISOString()}
                  title={fullTimestampLabel(sentAt)}
                  class="type-message-timestamp text-(--mw-component-message-timestamp-text)"
                  >{messageTimestampLabel(sentAt, new Date())}</time
                >
              </p>
              <p class="line-clamp-4 type-message break-words whitespace-pre-wrap text-(--mw-component-message-body-text)">
                {pinnedMessageSummaryText(message)}
              </p>
              <div class="flex flex-wrap gap-xs">
                {#if loadedIds.has(message.id)}
                  <Button
                    variant="secondary"
                    aria-label={strings.pins.jumpTo(displayName(message.author))}
                    onclick={() => {
                      panel.hidePopover();
                      onjump(message.id);
                    }}>{strings.pins.jump}</Button
                  >
                {/if}
                {#if canManagePins}
                  <Button variant="ghost" onclick={() => onunpin(message)}>{strings.pins.unpinSubmit}</Button>
                {/if}
              </div>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

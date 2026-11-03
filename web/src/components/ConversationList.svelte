<script lang="ts">
  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { DirectMessageChannel, PublicUser, Snowflake } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import { contextMenuTrigger } from "./context-menu-trigger";
  import { nameWithUnreadState } from "./format";
  import GroupAvatar from "./GroupAvatar.svelte";
  import {
    conversationDisplayName,
    isGroupConversation,
    otherParticipants,
    participantCount,
  } from "./group-conversations";
  import Icon from "./Icon.svelte";
  import MentionBadge from "./MentionBadge.svelte";
  import type { MenuOrigin } from "./menu-placement";
  import { isTargetMuted } from "./notification-preferences.svelte";
  import { mutedMark, sidebarRowClasses, unreadPill } from "./SidebarRowParts.svelte";

  // Home's list in the navigator card (ADR-0025, ADR-0026): Friends, then the conversations.
  interface Props {
    client: MellowClient;
    /** Right-click, long press or Shift+F10 on a row: the person's menu, or the group's. */
    onopenmenu: (
      conversation: DirectMessageChannel,
      recipient: PublicUser | undefined,
      origin: MenuOrigin,
      invoker: HTMLElement,
    ) => boolean;
    /** After Friends or a conversation was chosen (the narrow layout closes its drawer). */
    onnavigate: () => void;
  }

  let { client, onopenmenu, onnavigate }: Props = $props();

  const user = $derived(client.user);
  const incomingRequests = $derived(client.incomingRequestCount());

  /** `null` opens the Friends space. */
  function selectConversation(channelId: Snowflake | null) {
    client.selectGuild(null, channelId);
    client.voice.closeCallView();
    client.voice.watchStream(null);
    onnavigate();
  }
</script>

<!-- Friends sits above the conversations, as on Discord (ADR-0025); its badge counts the
     requests waiting for an answer. -->
<div class="mt-xs me-(--mw-component-channel-row-margin-inline)">
  <button
    type="button"
    aria-current={client.isFriendsSpaceOpen ? "page" : undefined}
    aria-label={strings.sidebar.friendsWithRequests(incomingRequests)}
    class={[sidebarRowClasses(client.isFriendsSpaceOpen, false), "h-(--mw-component-member-row-height) gap-sm pointer-coarse:h-(--mw-size-hit-web-min)"]}
    onclick={() => selectConversation(null)}
  >
    <span class="grid size-(--mw-component-member-row-avatar-size) shrink-0 place-items-center">
      <Icon name="users" class="size-(--mw-component-channel-row-icon-size)" />
    </span>
    <span class="min-w-0 flex-1 truncate type-channel-name">{strings.sidebar.friends}</span>
    {#if incomingRequests > 0}<MentionBadge mentionCount={incomingRequests} />{/if}
  </button>
</div>
{#if client.conversations.length === 0}
  <p class="mt-xs me-(--mw-component-channel-row-margin-inline) px-(--mw-component-channel-row-padding-inline) type-body text-text-muted">
    {strings.sidebar.noConversations}
  </p>
{:else}
  <ul class="mt-xs">
    {#each client.conversations as conversation (conversation.id)}
      {@const isSelected = conversation.id === client.selectedChannelId}
      {@const isConversationMuted = isTargetMuted({ kind: "conversation", channelId: conversation.id })}
      {@const hasUnread = !isSelected && !isConversationMuted && client.hasUnread(conversation.id)}
      {@const mentionCount = isSelected || isConversationMuted ? 0 : client.mentionCount(conversation.id)}
      {@const others = otherParticipants(conversation, user?.id ?? null)}
      {@const [recipient] = others}
      {@const name = conversationDisplayName(conversation, user?.id ?? null)}
      {@const isGroup = isGroupConversation(conversation)}
      {@const memberCount = participantCount(conversation, user?.id ?? null)}
      {@const rowName = isGroup ? strings.groups.rowLabel(name, memberCount) : name}
      <li class="relative mt-(--mw-component-member-row-gap) me-(--mw-component-channel-row-margin-inline)">
        {#if hasUnread}{@render unreadPill()}{/if}
        <!-- Right-click, long press or Shift+F10: the person's menu, or the group's. -->
        <button
          type="button"
          aria-current={isSelected ? "page" : undefined}
          aria-label={nameWithUnreadState(
            isConversationMuted ? strings.notifications.withMuted(rowName) : rowName,
            hasUnread,
            mentionCount,
          )}
          class={[sidebarRowClasses(isSelected, hasUnread, isConversationMuted), "h-(--mw-component-member-row-height) gap-sm pointer-coarse:h-(--mw-size-hit-web-min)"]}
          onclick={() => selectConversation(conversation.id)}
          {@attach contextMenuTrigger((origin, invoker) => onopenmenu(conversation, recipient, origin, invoker))}
        >
          {#if isGroup}
            <!-- No presence on a group: its second line counts its people, as on Discord. -->
            <GroupAvatar
              people={others}
              sizeToken="--mw-component-member-row-avatar-size"
              ringColorToken="--mw-component-navigator-background"
            />
          {:else if recipient}
            <Avatar
              user={recipient}
              sizeToken="--mw-component-member-row-avatar-size"
              presence={client.presenceOf(recipient.id)}
              ringColorToken="--mw-component-navigator-background"
            />
          {/if}
          <span class="flex min-w-0 flex-1 flex-col">
            <span class="truncate type-channel-name">{name}</span>
            {#if isGroup}
              <span class="truncate type-body-sm text-(--mw-component-group-avatar-member-count-text)">
                {strings.groups.memberCount(memberCount)}
              </span>
            {/if}
          </span>
          {#if isConversationMuted}{@render mutedMark()}{/if}
          {#if mentionCount > 0}<MentionBadge {mentionCount} />{/if}
        </button>
      </li>
    {/each}
  </ul>
{/if}

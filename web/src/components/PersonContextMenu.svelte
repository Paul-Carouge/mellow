<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { PublicUser, Snowflake } from "../lib/protocol";
  import ContextMenu, { type ContextMenuOutcome } from "./ContextMenu.svelte";
  import {
    type MenuItemHandler,
    type PersonMenuAction,
    copyIdentifier,
    menuEntries,
    personMenuActions,
  } from "./context-menus";
  import { displayName } from "./format";
  import type { MenuOrigin } from "./menu-placement";
  import { isTargetMuted, notificationMenuHandler } from "./notification-preferences.svelte";
  import RelationshipConfirmDialog from "./RelationshipConfirmDialog.svelte";

  // A person's context menu (DESIGN.md §6.33): a member row, a message's author, a friend row, a
  // one-to-one conversation's row. Friend requests and Unblock wait in the menu; Remove friend and
  // Block ask first in the destructive dialog of DESIGN.md §8, as in the Friends space.
  interface Props {
    client: MellowClient;
    /** The member list shows profiles: "Profile" opens one there. */
    onshowprofile?: (user: PublicUser, invoker: HTMLElement) => void;
    /** After "Message" opened the conversation (the composer takes focus, an overlay closes). */
    onopenedconversation: () => void;
    /** Where focus goes when the person's row left meanwhile (a friend removed or blocked). */
    fallbackFocus?: () => void;
  }

  let { client, onshowprofile, onopenedconversation, fallbackFocus }: Props = $props();

  let menu = $state<ContextMenu | null>(null);
  let target = $state<{
    user: PublicUser;
    invoker: HTMLElement;
    canOpenConversation: boolean;
    conversationChannelId: Snowflake | null;
  } | null>(null);
  let confirming = $state<{ action: "remove" | "block"; user: PublicUser; invoker: HTMLElement } | null>(null);
  let announcement = $state("");

  const entries = $derived.by(() => {
    if (!target) {
      return [];
    }
    const { user, invoker, conversationChannelId } = target;
    const lines = personMenuActions({
      isSelf: user.id === client.user?.id,
      relationship: client.relationshipByUserId.get(user.id)?.kind ?? null,
      canShowProfile: onshowprofile !== undefined,
      canOpenConversation: target.canOpenConversation,
      conversationChannel: conversationChannelId !== null,
      isConversationMuted:
        conversationChannelId !== null && isTargetMuted({ kind: "conversation", channelId: conversationChannelId }),
    });
    return menuEntries(lines, (action) => handlerOf(action, user, invoker, conversationChannelId));
  });

  /** Opens the menu about `user`; `false` when there is no menu to show (never: the ID is always there). */
  export function openFor(
    user: PublicUser,
    origin: MenuOrigin,
    invoker: HTMLElement,
    options: { canOpenConversation?: boolean; conversationChannelId?: Snowflake } = {},
  ): boolean {
    target = {
      user,
      invoker,
      canOpenConversation: options.canOpenConversation ?? true,
      conversationChannelId: options.conversationChannelId ?? null,
    };
    menu?.open(origin, invoker);
    return true;
  }

  async function waitFor(change: () => Promise<unknown>, done: string, failed: string): Promise<ContextMenuOutcome> {
    try {
      await change();
      return { announcement: done };
    } catch (error) {
      return { error: describeError(error, failed) };
    }
  }

  function handlerOf(
    action: PersonMenuAction,
    user: PublicUser,
    invoker: HTMLElement,
    conversationChannelId: Snowflake | null,
  ): MenuItemHandler {
    const person = displayName(user);
    switch (action) {
      case "profile":
        return { onselect: () => onshowprofile?.(user, invoker) };
      case "message":
        return {
          onselectAndWait: async () => {
            try {
              await client.openDirectMessage(user.id);
            } catch (error) {
              return { error: describeError(error, strings.members.openConversationFailed) };
            }
            onopenedconversation();
            return { announcement: "" };
          },
        };
      case "add-friend":
        return {
          onselectAndWait: async () => {
            try {
              const relationship = await client.sendFriendRequest(user.username);
              return {
                announcement:
                  relationship.kind === "friend" ? strings.friends.acceptedTheirs(person) : strings.friends.sent(person),
              };
            } catch (error) {
              return { error: describeError(error, strings.members.friendRequestFailed) };
            }
          },
        };
      case "accept-request":
        return {
          onselectAndWait: () =>
            waitFor(() => client.acceptFriendRequest(user.id), strings.friends.accepted(person), strings.friends.acceptFailed(person)),
        };
      case "ignore-request":
        return {
          onselectAndWait: () =>
            waitFor(() => client.removeRelationship(user.id), strings.friends.ignored(person), strings.friends.ignoreFailed(person)),
        };
      case "cancel-request":
        return {
          onselectAndWait: () =>
            waitFor(() => client.removeRelationship(user.id), strings.friends.cancelled(person), strings.friends.cancelFailed(person)),
        };
      case "unblock":
        return {
          onselectAndWait: () =>
            waitFor(() => client.removeRelationship(user.id), strings.friends.unblocked(person), strings.friends.unblockFailed(person)),
        };
      case "copy-user-id":
        return { onselectAndWait: () => copyIdentifier(user.id, strings.contextMenu.userIdCopied) };
      case "copy-channel-id":
        return {
          onselectAndWait: () => copyIdentifier(conversationChannelId ?? "", strings.contextMenu.channelIdCopied),
        };
      case "remove-friend":
        return { onselect: () => (confirming = { action: "remove", user, invoker }) };
      case "block":
        return { onselect: () => (confirming = { action: "block", user, invoker }) };
      case "mute":
      case "unmute":
      case "notification-settings":
        // Only on a conversation's row, where `conversationChannelId` is set.
        return notificationMenuHandler(action, { kind: "conversation", channelId: conversationChannelId ?? "" }, person);
    }
  }
</script>

<ContextMenu
  bind:this={menu}
  label={target ? strings.contextMenu.actionsFor(displayName(target.user)) : strings.members.profile}
  {entries}
  {fallbackFocus}
/>

{#if confirming}
  {@const pending = confirming}
  <RelationshipConfirmDialog
    {client}
    action={pending.action}
    user={pending.user}
    onclose={() => (confirming = null)}
    ondone={(sentence) => (announcement = sentence)}
    fallbackFocus={() => (pending.invoker.isConnected ? pending.invoker.focus() : fallbackFocus?.())}
  />
{/if}

<p class="sr-only" aria-live="polite">{announcement}</p>

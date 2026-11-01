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
  import Icon from "./Icon.svelte";
  import RangeField from "./RangeField.svelte";
  import RelationshipConfirmDialog from "./RelationshipConfirmDialog.svelte";
  import { MAXIMUM_VOLUME_PERCENT, MINIMUM_VOLUME_PERCENT } from "../lib/voice-settings";

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
    isInVoice: boolean;
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
    options: { canOpenConversation?: boolean; conversationChannelId?: Snowflake; isInVoice?: boolean } = {},
  ): boolean {
    target = {
      user,
      invoker,
      canOpenConversation: options.canOpenConversation ?? true,
      conversationChannelId: options.conversationChannelId ?? null,
      isInVoice: options.isInVoice ?? false,
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

{#snippet voiceControls()}
  <!-- Someone in a voice channel, not you: Discord's user volume and local mute, kept in this browser. -->
  {#if target?.isInVoice && target.user.id !== client.user?.id}
    {@const userId = target.user.id}
    {@const setting = client.voice.personSetting(userId)}
    <div class="px-(--mw-component-context-menu-item-padding-inline) pt-2xs">
      <RangeField
        label={strings.voice.userVolume}
        value={setting.volumePercent}
        min={MINIMUM_VOLUME_PERCENT}
        max={MAXIMUM_VOLUME_PERCENT}
        valueText={strings.voiceSettings.percent}
        inputAttributes={{ "data-menu-control": "" }}
        oninput={(value) => client.voice.setPersonVolume(userId, value)}
      />
    </div>
    <!-- A checkbox item stays open, so the change can be heard and undone at once. -->
    <button
      type="button"
      role="menuitemcheckbox"
      aria-checked={setting.isLocallyMuted}
      class="flex min-h-(--mw-component-context-menu-item-height) w-full items-center justify-between gap-xs rounded-(--mw-component-context-menu-item-radius) px-(--mw-component-context-menu-item-padding-inline) py-3xs text-start type-menu-item text-(--mw-component-context-menu-item-text) hover:bg-(--mw-component-context-menu-item-background-focus) hover:text-(--mw-component-context-menu-item-text-focus) focus-visible:bg-(--mw-component-context-menu-item-background-focus) focus-visible:text-(--mw-component-context-menu-item-text-focus) focus-visible:focus-ring-inset pointer-coarse:min-h-(--mw-size-hit-web-min)"
      onclick={() => client.voice.togglePersonMuted(userId)}
    >
      {strings.voice.muteForMe}
      <!-- A box, ticked when on: a checkbox reads as one, not as another action. -->
      <span class="grid size-(--mw-component-context-menu-item-icon-size) shrink-0 place-items-center rounded-xs border-2 border-current">
        {#if setting.isLocallyMuted}<Icon name="check" class="size-full" />{/if}
      </span>
    </button>
    <hr
      class="mx-(--mw-component-context-menu-separator-margin-inline) my-(--mw-component-context-menu-separator-margin-block) h-(--mw-component-context-menu-separator-height) shrink-0 border-0 bg-(--mw-component-context-menu-separator)"
    />
  {/if}
{/snippet}

<ContextMenu
  bind:this={menu}
  label={target ? strings.contextMenu.actionsFor(displayName(target.user)) : strings.members.profile}
  {entries}
  header={voiceControls}
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

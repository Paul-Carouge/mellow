<script lang="ts">
  import { untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";

  import { strings } from "../i18n";
  import { type ChannelSection, channelSections } from "../lib/channels";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import {
    type Channel,
    type ChannelPlacement,
    type DirectMessageChannel,
    type Guild,
    Permission,
    type PublicUser,
    type Snowflake,
    type VoiceState,
    hasPermission,
    iconUrl,
  } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { channelDragSource } from "./channel-drag";
  import {
    type ChannelDrop,
    type ListedBox,
    categoryDropAt,
    categoryStepIndex,
    channelDropAt,
    channelStepDestination,
    isPendingOrderSettled,
    placementsOf,
    sectionsFromPlacements,
    withCategoryMoved,
    withChannelMoved,
  } from "./channel-order";
  import { canManageChannel, visibleChannelSections } from "./channel-settings";
  import { contextMenuTrigger } from "./context-menu-trigger";
  import {
    type ChannelMenuFacts,
    type MenuItemHandler,
    type ServerMenuAction,
    categoryMenuActions,
    channelMenuActions,
    copyIdentifier,
    groupMenuActions,
    menuEntries,
    serverMenuActions,
    serverMenuFactsOf,
  } from "./context-menus";
  import { nameWithUnreadState } from "./format";
  import { isTargetMuted, notificationMenuHandler } from "./notification-preferences.svelte";
  import GroupAvatar from "./GroupAvatar.svelte";
  import {
    conversationDisplayName,
    isGroupConversation,
    otherParticipants,
    participantCount,
  } from "./group-conversations";
  import ContextMenu, { type ContextMenuOutcome } from "./ContextMenu.svelte";
  import GroupConfirmDialog from "./GroupConfirmDialog.svelte";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import MentionBadge from "./MentionBadge.svelte";
  import type { MenuOrigin } from "./menu-placement";
  import DeleteChannelDialog from "./DeleteChannelDialog.svelte";
  import MoveToCategoryDialog from "./MoveToCategoryDialog.svelte";
  import RenameCategoryDialog from "./RenameCategoryDialog.svelte";
  import PersonContextMenu from "./PersonContextMenu.svelte";
  import RenameGroupDialog from "./RenameGroupDialog.svelte";
  import { lengthTokenInPixels } from "./tokens";
  import { prepareChimeSound } from "./chime-sound";
  import { displayName } from "./format";

  export type ServerDialog =
    | "invite-people"
    | "server-settings"
    | "create-channel"
    | "create-category"
    | "leave-server";

  // The channel list of the navigator card (DESIGN.md §5.3, §5.6, ADR-0022 §1): server header
  // with its menu, then channels by category, or the conversations at Home.
  interface Props {
    client: MellowClient;
    /** Kept by the shell, so collapsing survives the drawer closing. */
    collapsedCategoryIds: SvelteSet<Snowflake>;
    /** The "+" beside "Direct messages" at Home: the "New message" picker (ADR-0026). */
    onnewmessage: () => void;
    /** A dialog about the open server (the header's menu). */
    onopendialog: (dialog: ServerDialog, guildId: Snowflake) => void;
    /** The "+" beside a category: create a channel inside it. */
    oncreatechannelin: (category: Channel) => void;
    /** The gear of a channel row: its settings (Discord's "Edit channel"). */
    oneditchannel: (channel: Channel) => void;
    /** After a channel or conversation was chosen (the narrow layout closes its drawer). */
    onnavigate: () => void;
  }

  let { client, collapsedCategoryIds, onnewmessage, onopendialog, oncreatechannelin, oneditchannel, onnavigate }: Props =
    $props();

  const guild = $derived(client.selectedGuild);
  const user = $derived(client.user);
  const canCreateChannels = $derived(guild ? hasPermission(guild.my_permissions, Permission.MANAGE_CHANNELS) : false);
  // Channel overwrites included: a read-only channel shows a lock (DESIGN.md §5.3).
  function canSendIn(channelId: Snowflake): boolean {
    const permissions = client.myChannelPermissions(channelId);
    return permissions === null || hasPermission(permissions, Permission.SEND_MESSAGES);
  }
  // Without `CONNECT` a voice channel shows a lock, as a read-only text channel does; the instance decides.
  function canConnectTo(channelId: Snowflake): boolean {
    const permissions = client.myChannelPermissions(channelId);
    return permissions === null || hasPermission(permissions, Permission.CONNECT);
  }
  function isInVoiceChannel(channelId: Snowflake): boolean {
    return client.voice.phase !== "idle" && client.voice.channelId === channelId;
  }
  /** A click joins (and asks for the microphone then): voice channels never open a chat. */
  function joinVoiceChannel(channelId: Snowflake) {
    void prepareChimeSound();
    void client.voice.join(channelId);
  }
  /** The member list's person, or a placeholder while it loads. */
  function voiceParticipant(guildId: Snowflake, state: VoiceState): PublicUser {
    return (
      client.membersByGuild.get(guildId)?.find((member) => member.user.id === state.user_id)?.user ??
      (state.user_id === user?.id && user
        ? user
        : { id: state.user_id, username: state.user_id, display_name: strings.voice.someone })
    );
  }
  function canEditChannel(channel: Channel): boolean {
    return visibleChannelSections(channel, client.myChannelPermissions(channel.id)).length > 0;
  }

  // --- Order: a pending move, the drag, the menus' moves (DESIGN.md §5.3) ------------------------

  /**
   * How long an agreed order waits for the gateway's `GUILD_CREATE` before the instance's order
   * shows again: a fallback for an echo that never comes (a dropped connection resyncs anyway).
   */
  const PENDING_ORDER_ECHO_TIMEOUT_MILLISECONDS = 5_000;

  interface PendingOrder {
    readonly guildId: Snowflake;
    readonly placements: readonly ChannelPlacement[];
    /** The guild's channels when the instance agreed: the next snapshot replaces them. */
    readonly answeredChannels: readonly Channel[] | null;
  }

  /** The order sent and not yet echoed, shown meanwhile so a drop does not snap back. */
  let pendingOrder = $state.raw<PendingOrder | null>(null);
  let isReordering = $state(false);
  let reorderError = $state("");
  const instanceSections = $derived(guild ? channelSections(guild) : []);
  const sections = $derived.by(() => {
    const pending = pendingOrder;
    if (!guild || pending === null || pending.guildId !== guild.id) {
      return instanceSections;
    }
    return sectionsFromPlacements(guild.channels, pending.placements) ?? instanceSections;
  });
  const canReorderNow = $derived(canCreateChannels && !isReordering);

  $effect(() => {
    const pending = pendingOrder;
    if (pending === null || guild?.id !== pending.guildId) {
      return;
    }
    if (isPendingOrderSettled(pending.placements, pending.answeredChannels, placementsOf(instanceSections), guild.channels)) {
      pendingOrder = null;
    }
  });

  /** Sends the whole new order; resolves to the error to show, or null. */
  async function applyOrder(guildId: Snowflake, next: readonly ChannelSection[], failureSummary: string): Promise<string | null> {
    const placements = placementsOf(next);
    const order: PendingOrder = { guildId, placements, answeredChannels: null };
    pendingOrder = order;
    isReordering = true;
    reorderError = "";
    try {
      await client.reorderChannels(guildId, placements);
      if (pendingOrder === order) {
        const answered: PendingOrder = {
          ...order,
          answeredChannels: client.guilds.find((candidate) => candidate.id === guildId)?.channels ?? null,
        };
        pendingOrder = answered;
        setTimeout(() => {
          if (pendingOrder === answered) {
            pendingOrder = null;
          }
        }, PENDING_ORDER_ECHO_TIMEOUT_MILLISECONDS);
      }
      return null;
    } catch (error) {
      if (pendingOrder === order) {
        pendingOrder = null;
      }
      return describeError(error, failureSummary);
    } finally {
      isReordering = false;
    }
  }

  async function menuOutcome(next: ChannelSection[] | null, failureSummary: string, done: string): Promise<ContextMenuOutcome> {
    if (!guild || next === null) {
      return { announcement: "" };
    }
    const problem = await applyOrder(guild.id, next, failureSummary);
    return problem === null ? { announcement: done } : { error: problem };
  }

  function moveChannelStep(channel: Channel, step: -1 | 1): Promise<ContextMenuOutcome> {
    const destination = channelStepDestination(sections, channel.id, step);
    return menuOutcome(
      destination && withChannelMoved(sections, channel.id, destination),
      strings.channelOrder.channelMoveFailed,
      step === -1 ? strings.channelOrder.channelMovedUp(channel.name) : strings.channelOrder.channelMovedDown(channel.name),
    );
  }

  function moveCategoryStep(category: Channel, step: -1 | 1): Promise<ContextMenuOutcome> {
    const index = categoryStepIndex(sections, category.id, step);
    return menuOutcome(
      index === null ? null : withCategoryMoved(sections, category.id, index),
      strings.channelOrder.categoryMoveFailed,
      step === -1 ? strings.channelOrder.categoryMovedUp(category.name) : strings.channelOrder.categoryMovedDown(category.name),
    );
  }

  function channelMovedSentence(channel: Channel, categoryId: Snowflake | null): string {
    const category = sections.find((section) => section.category?.id === categoryId)?.category;
    return category ? strings.channelOrder.channelMovedTo(channel.name, category.name) : strings.channelOrder.channelMovedOut(channel.name);
  }

  /** "Move to category…": to the end of the one chosen. */
  async function moveChannelTo(channel: Channel, categoryId: Snowflake | null): Promise<string | null> {
    // Opened, so the moved row is there to take focus back.
    if (categoryId !== null) {
      collapsedCategoryIds.delete(categoryId);
    }
    const section = sections.find((candidate) => (candidate.category?.id ?? null) === categoryId);
    const next = withChannelMoved(sections, channel.id, {
      categoryId,
      index: section?.channels.length ?? 0,
      leavesHiddenCategory: true,
    });
    if (!guild || next === null) {
      return null;
    }
    const sentence = channelMovedSentence(channel, categoryId);
    const problem = await applyOrder(guild.id, next, strings.channelOrder.channelMoveFailed);
    if (problem === null) {
      announcement = sentence;
    }
    return problem;
  }

  let movingChannel = $state<Channel | null>(null);
  /** Plain: read once the dialog and its block are gone (the `{@const}` trap). */
  let movedChannelId: Snowflake | null = null;
  /** The moved row, rebuilt in its new section, takes focus back. */
  function focusMovedChannel() {
    if (movedChannelId !== null) {
      focusListed(movedChannelId);
    }
  }
  /** Delete a channel or a category, or rename a category, from its menu. */
  let channelDialog = $state<{ kind: "delete" | "rename"; channel: Channel } | null>(null);

  function focusListed(id: Snowflake) {
    list?.querySelector<HTMLElement>(`[data-order-id="${id}"] button`)?.focus();
  }

  // The drag: what is carried, where it would land, and the list scrolling under it.
  interface Carried {
    readonly kind: "channel" | "category";
    readonly id: Snowflake;
    readonly name: string;
    readonly mayChangeCategory: boolean;
    readonly isVoice: boolean;
    /** Where the pointer grabbed it, from its top. */
    readonly grabOffset: number;
    readonly left: number;
    readonly width: number;
    readonly height: number;
  }

  let list = $state<HTMLElement | null>(null);
  let carried = $state.raw<Carried | null>(null);
  /** The pointer's height in the list's scrolled content. */
  let carriedY = $state(0);
  let channelDrop = $state.raw<ChannelDrop | null>(null);
  let categoryDrop = $state.raw<{ readonly index: number; readonly y: number } | null>(null);
  const dropLineY = $derived(
    carried?.kind === "channel"
      ? channelDrop?.indicator.kind === "line"
        ? channelDrop.indicator.y
        : null
      : (categoryDrop?.y ?? null),
  );
  const dropCategoryId = $derived(channelDrop?.indicator.kind === "category" ? channelDrop.indicator.id : null);
  let pointerClientY = 0;
  let autoScrollSpeed = 0;
  let autoScrollFrame: number | null = null;

  function contentY(clientY: number): number {
    return list ? clientY - list.getBoundingClientRect().top + list.scrollTop : 0;
  }

  function listedBoxes(): ListedBox[] {
    if (!list) {
      return [];
    }
    const top = list.getBoundingClientRect().top - list.scrollTop;
    return [...list.querySelectorAll<HTMLElement>("[data-order-id]")].map((element) => {
      const bounds = element.getBoundingClientRect();
      return {
        kind: element.dataset.orderKind === "category" ? "category" : "channel",
        id: element.dataset.orderId ?? "",
        top: bounds.top - top,
        bottom: bounds.bottom - top,
      };
    });
  }

  function updateDrop() {
    if (!carried) {
      return;
    }
    carriedY = contentY(pointerClientY);
    if (carried.kind === "channel") {
      channelDrop = channelDropAt(sections, listedBoxes(), carried.id, carriedY, carried.mayChangeCategory);
    } else {
      categoryDrop = categoryDropAt(sections, listedBoxes(), carried.id, carriedY);
    }
  }

  function scrollUnderCarriedRow() {
    autoScrollFrame = null;
    if (!carried || !list || autoScrollSpeed === 0) {
      return;
    }
    const scrolledFrom = list.scrollTop;
    list.scrollTop += autoScrollSpeed;
    // At either end of the list: nothing left to scroll until the pointer moves again.
    if (list.scrollTop === scrolledFrom) {
      return;
    }
    updateDrop();
    autoScrollFrame = requestAnimationFrame(scrollUnderCarriedRow);
  }

  /**
   * Takes only the id, read untracked by the attachment: anything else it read (the channel
   * object, permissions) would recreate the attachment, and so cancel a drag in progress, on any
   * unrelated gateway update. The rest is read when the drag starts.
   */
  function dragHandlers(kind: Carried["kind"], id: Snowflake) {
    return {
      canStart: () => canReorderNow,
      onstart: (_x: number, y: number, source: HTMLElement) => {
        const item = sections
          .flatMap((section) => (section.category ? [section.category, ...section.channels] : section.channels))
          .find((candidate) => candidate.id === id);
        if (!item) {
          return;
        }
        const bounds = source.getBoundingClientRect();
        const listLeft = list?.getBoundingClientRect().left ?? 0;
        rowMenu?.close();
        reorderError = "";
        carried = {
          kind,
          id: item.id,
          name: item.name,
          mayChangeCategory: kind === "category" || canManageChannel(client.myChannelPermissions(id)),
          isVoice: item.kind === "voice",
          grabOffset: y - bounds.top,
          left: bounds.left - listLeft,
          width: bounds.width,
          height: bounds.height,
        };
      },
      onmove: (_x: number, y: number) => {
        pointerClientY = y;
        updateDrop();
        if (!list) {
          return;
        }
        const bounds = list.getBoundingClientRect();
        const edge = lengthTokenInPixels("component-channel-drag-autoscroll-edge");
        const step = lengthTokenInPixels("component-channel-drag-autoscroll-step");
        const nearness = (distance: number) => Math.min(1, Math.max(0, 1 - distance / edge));
        autoScrollSpeed = step * (nearness(bounds.bottom - y) - nearness(y - bounds.top));
        if (autoScrollSpeed !== 0 && autoScrollFrame === null) {
          autoScrollFrame = requestAnimationFrame(scrollUnderCarriedRow);
        }
      },
      onend: (isDropped: boolean) => void drop(isDropped),
    };
  }

  async function drop(isDropped: boolean) {
    const item = carried;
    const target = channelDrop;
    const categoryTarget = categoryDrop;
    carried = null;
    channelDrop = null;
    categoryDrop = null;
    autoScrollSpeed = 0;
    if (autoScrollFrame !== null) {
      cancelAnimationFrame(autoScrollFrame);
      autoScrollFrame = null;
    }
    if (!isDropped || !item || !guild) {
      return;
    }
    const guildId = guild.id;
    let next: ChannelSection[] | null = null;
    let sentence = "";
    let failure: string = strings.channelOrder.channelMoveFailed;
    if (item.kind === "channel" && target) {
      const channel = sections.flatMap((section) => section.channels).find((candidate) => candidate.id === item.id);
      next = withChannelMoved(sections, item.id, target.destination);
      sentence = channel ? channelMovedSentence(channel, target.destination.categoryId) : "";
    } else if (item.kind === "category" && categoryTarget) {
      next = withCategoryMoved(sections, item.id, categoryTarget.index);
      sentence = strings.channelOrder.categoryMoved(item.name);
      failure = strings.channelOrder.categoryMoveFailed;
    }
    if (next === null) {
      return;
    }
    const problem = await applyOrder(guildId, next, failure);
    if (problem === null) {
      announcement = sentence;
    } else {
      reorderError = problem;
    }
  }

  /** Icons that failed to load show the # glyph instead. */
  const failedIconIds = new SvelteSet<Snowflake>();

  let headerButton = $state<HTMLButtonElement | null>(null);
  let menu = $state<ContextMenu | null>(null);
  let isMenuOpen = $state(false);
  const serverMenuEntries = $derived(
    guild
      ? menuEntries(
          serverMenuActions(serverMenuFactsOf(guild, user?.id, isTargetMuted({ kind: "server", guildId: guild.id })), "header"),
          (action) => serverMenuHandler(action, guild),
        )
      : [],
  );
  const hasServerMenu = $derived(serverMenuEntries.length > 0);

  /** The header's dialogs and the strip's share their names with the menu's actions. */
  function serverMenuHandler(action: ServerMenuAction, server: Guild): MenuItemHandler {
    switch (action) {
      case "copy-server-id":
        return { onselectAndWait: () => copyIdentifier(server.id, strings.contextMenu.serverIdCopied) };
      case "mute":
      case "unmute":
      case "notification-settings":
        return notificationMenuHandler(action, { kind: "server", guildId: server.id }, server.name);
      default:
        return { onselect: () => onopendialog(action, server.id) };
    }
  }

  // The rows' menus: a channel's, a group's, or the person's for a one-to-one conversation.
  let rowMenu = $state<ContextMenu | null>(null);
  let personMenu = $state<PersonContextMenu | null>(null);
  let rowMenuTarget = $state<
    | { kind: "channel"; channel: Channel; invoker: HTMLElement }
    | { kind: "category"; category: Channel; invoker: HTMLElement }
    | { kind: "group"; group: DirectMessageChannel; invoker: HTMLElement }
    | null
  >(null);
  let groupDialog = $state<{ kind: "rename" | "leave"; group: DirectMessageChannel; invoker: HTMLElement } | null>(null);
  let announcement = $state("");
  // A channel or group that disappears (deleted, hidden, left) while its menu is open closes it.
  $effect(() => {
    const target = rowMenuTarget;
    const isGone =
      target?.kind === "channel" || target?.kind === "category"
        ? client.myChannelPermissions(target.kind === "channel" ? target.channel.id : target.category.id) === null
        : target?.kind === "group" && !client.conversations.some((conversation) => conversation.id === target.group.id);
    if (isGone) {
      rowMenu?.close();
    }
  });
  const rowMenuEntries = $derived.by(() => {
    const target = rowMenuTarget;
    if (target?.kind === "channel") {
      const { channel } = target;
      const notificationTarget = { kind: "channel", guildId: channel.guild_id, channelId: channel.id } as const;
      const voiceAction: ChannelMenuFacts["voice"] = isInVoiceChannel(channel.id) ? "leave" : "join";
      const facts = {
        voice: channel.kind === "voice" ? voiceAction : undefined,
        canEditChannel: canEditChannel(channel),
        isMuted: isTargetMuted(notificationTarget),
        canMoveUp: canReorderNow && channelStepDestination(sections, channel.id, -1) !== null,
        canMoveDown: canReorderNow && channelStepDestination(sections, channel.id, 1) !== null,
        canMoveToCategory:
          canReorderNow &&
          canManageChannel(client.myChannelPermissions(channel.id)) &&
          sections.some((section) => section.category !== null),
        canDelete: canManageChannel(client.myChannelPermissions(channel.id)),
      };
      return menuEntries(channelMenuActions(facts), (action) => {
        switch (action) {
          case "join-voice":
            return { onselect: () => joinVoiceChannel(channel.id) };
          case "leave-voice":
            return { onselect: () => client.voice.leave() };
          case "edit-channel":
            return { onselect: () => oneditchannel(channel) };
          case "move-up":
          case "move-down":
            return { onselectAndWait: () => moveChannelStep(channel, action === "move-up" ? -1 : 1) };
          case "move-to-category":
            return {
              onselect: () => {
                movingChannel = channel;
                movedChannelId = channel.id;
              },
            };
          case "copy-channel-id":
            return { onselectAndWait: () => copyIdentifier(channel.id, strings.contextMenu.channelIdCopied) };
          case "delete-channel":
            return { onselect: () => (channelDialog = { kind: "delete", channel }) };
          case "mute":
          case "unmute":
          case "notification-settings":
            return notificationMenuHandler(action, notificationTarget, `#${channel.name}`);
        }
      });
    }
    if (target?.kind === "category") {
      const { category } = target;
      const facts = {
        canManage: canManageChannel(client.myChannelPermissions(category.id)),
        canMoveUp: canReorderNow && categoryStepIndex(sections, category.id, -1) !== null,
        canMoveDown: canReorderNow && categoryStepIndex(sections, category.id, 1) !== null,
      };
      return menuEntries(categoryMenuActions(facts), (action) => {
        switch (action) {
          case "rename-category":
            return { onselect: () => (channelDialog = { kind: "rename", channel: category }) };
          case "move-up":
          case "move-down":
            return { onselectAndWait: () => moveCategoryStep(category, action === "move-up" ? -1 : 1) };
          case "copy-category-id":
            return { onselectAndWait: () => copyIdentifier(category.id, strings.contextMenu.categoryIdCopied) };
          case "delete-category":
            return { onselect: () => (channelDialog = { kind: "delete", channel: category }) };
        }
      });
    }
    if (target?.kind === "group") {
      const { group, invoker } = target;
      const notificationTarget = { kind: "conversation", channelId: group.id } as const;
      return menuEntries(groupMenuActions(isTargetMuted(notificationTarget)), (action) => {
        switch (action) {
          case "copy-channel-id":
            return { onselectAndWait: () => copyIdentifier(group.id, strings.contextMenu.channelIdCopied) };
          case "mute":
          case "unmute":
          case "notification-settings":
            return notificationMenuHandler(
              action,
              notificationTarget,
              conversationDisplayName(group, user?.id ?? null),
            );
          case "rename-group":
          case "leave-group":
            return {
              onselect: () =>
                (groupDialog = { kind: action === "rename-group" ? "rename" : "leave", group, invoker }),
            };
        }
      });
    }
    return [];
  });

  /** A participant row's click opens their menu under it, as its keyboard does. */
  function openParticipantMenu(person: PublicUser, row: HTMLElement) {
    const bounds = row.getBoundingClientRect();
    personMenu?.openFor(
      person,
      { kind: "anchor", bounds: { top: bounds.top, bottom: bounds.bottom, left: bounds.left, right: bounds.right }, align: "start" },
      row,
      { isInVoice: true },
    );
  }

  function openChannelMenu(channel: Channel, origin: MenuOrigin, invoker: HTMLElement): boolean {
    if (carried) {
      return false;
    }
    rowMenuTarget = { kind: "channel", channel, invoker };
    rowMenu?.open(origin, invoker);
    return true;
  }

  function openCategoryMenu(category: Channel, origin: MenuOrigin, invoker: HTMLElement): boolean {
    if (carried) {
      return false;
    }
    rowMenuTarget = { kind: "category", category, invoker };
    rowMenu?.open(origin, invoker);
    return true;
  }

  function openConversationMenu(
    conversation: DirectMessageChannel,
    recipient: PublicUser | undefined,
    origin: MenuOrigin,
    invoker: HTMLElement,
  ): boolean {
    if (!isGroupConversation(conversation)) {
      return recipient
        ? (personMenu?.openFor(recipient, origin, invoker, {
            canOpenConversation: false,
            conversationChannelId: conversation.id,
          }) ?? false)
        : false;
    }
    rowMenuTarget = { kind: "group", group: conversation, invoker };
    rowMenu?.open(origin, invoker);
    return true;
  }

  function openMenu() {
    if (!headerButton) {
      return;
    }
    // Inset by the menu margin on both sides, and that far below the header.
    const bounds = headerButton.getBoundingClientRect();
    const margin = lengthTokenInPixels("component-sidebar-header-menu-margin");
    menu?.open(
      { kind: "anchor", bounds: { top: bounds.top, bottom: bounds.bottom, left: bounds.left + margin, right: bounds.right - margin }, align: "stretch" },
      headerButton,
    );
  }

  function selectChannel(channelId: Snowflake) {
    client.selectChannel(channelId);
    onnavigate();
  }

  /** `null` opens the Friends space. */
  function selectConversation(channelId: Snowflake | null) {
    client.selectGuild(null, channelId);
    onnavigate();
  }

  function toggleCategory(categoryId: Snowflake) {
    if (collapsedCategoryIds.has(categoryId)) {
      collapsedCategoryIds.delete(categoryId);
    } else {
      collapsedCategoryIds.add(categoryId);
    }
  }

  /** A muted row is greyed and never shows unread (DESIGN.md §5.3); its bell-slash says so too. */
  const rowClasses = (isSelected: boolean, hasUnread: boolean, isMuted = false) => [
    "flex w-full items-center gap-(--mw-component-channel-row-icon-gap) rounded-(--mw-component-channel-row-radius) px-(--mw-component-channel-row-padding-inline) text-start transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) focus-visible:focus-ring-inset",
    isSelected
      ? "bg-(--mw-component-channel-row-background-selected) text-(--mw-component-channel-row-text-selected)"
      : [
          isMuted
            ? "text-(--mw-component-channel-row-text-muted-channel)"
            : hasUnread
              ? "text-(--mw-component-channel-row-text-unread)"
              : "text-(--mw-component-channel-row-text-read)",
          "hover:bg-(--mw-component-channel-row-background-hover) hover:text-(--mw-component-channel-row-text-hover)",
        ],
  ];
</script>

{#snippet mutedMark()}
  <Icon name="bell-slash" class="size-(--mw-component-channel-row-muted-icon-size) shrink-0 text-(--mw-component-channel-row-muted-icon)" />
{/snippet}

{#snippet unreadPill()}
  <span
    aria-hidden="true"
    class="absolute top-1/2 -left-(--mw-component-channel-row-unread-pill-offset) h-(--mw-component-channel-row-unread-pill-height) w-(--mw-component-channel-row-unread-pill-width) -translate-y-1/2 rounded-full bg-(--mw-component-channel-row-unread-pill-color)"
  ></span>
{/snippet}

<div class="flex min-h-0 min-w-0 flex-col">
  <header class="mt-xs me-xs h-(--mw-component-sidebar-header-height) shrink-0">
    {#if guild && hasServerMenu}
      <button
        bind:this={headerButton}
        type="button"
        aria-haspopup="menu"
        aria-expanded={isMenuOpen}
        aria-controls="server-menu"
        class={[
          "flex size-full items-center gap-xs rounded-sm px-xs text-start text-(--mw-component-sidebar-header-text) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-sidebar-header-background-hover)",
          isMenuOpen && "bg-(--mw-component-sidebar-header-background-hover)",
        ]}
        onclick={() => (isMenuOpen ? menu?.close() : openMenu())}
      >
        <h2 class="min-w-0 flex-1 truncate type-heading-md">{guild.name}</h2>
        <Icon
          name={isMenuOpen ? "x" : "caret-down"}
          class="size-(--mw-component-sidebar-header-chevron-size) text-text-secondary"
        />
      </button>
    {:else if guild}
      <h2
        class="flex size-full items-center truncate px-xs type-heading-md text-(--mw-component-sidebar-header-text)"
      >
        {guild.name}
      </h2>
    {:else}
      <!-- Home: Discord's "+" beside "Direct messages" starts a conversation or a group (ADR-0026). -->
      <div class="flex size-full items-center gap-2xs ps-xs">
        <h2 class="min-w-0 flex-1 truncate type-heading-md text-(--mw-component-sidebar-header-text)">
          {strings.sidebar.directMessages}
        </h2>
        <IconButton
          icon="plus"
          label={strings.groups.newMessage}
          aria-haspopup="dialog"
          class="size-(--mw-component-category-height) rounded-xs text-(--mw-component-category-text) hover:text-(--mw-component-category-text-hover)"
          iconClass="size-(--mw-component-category-add-icon-size)"
          onclick={onnewmessage}
        />
      </div>
    {/if}
  </header>

  <ContextMenu
    bind:this={menu}
    bind:isOpen={isMenuOpen}
    id="server-menu"
    label={strings.sidebar.serverMenu}
    entries={serverMenuEntries}
    gapToken="component-sidebar-header-menu-margin"
  />

  <ContextMenu
    bind:this={rowMenu}
    label={rowMenuTarget?.kind === "channel"
      ? strings.contextMenu.actionsFor(
          rowMenuTarget.channel.kind === "voice" ? rowMenuTarget.channel.name : `#${rowMenuTarget.channel.name}`,
        )
      : rowMenuTarget?.kind === "category"
        ? strings.contextMenu.actionsFor(rowMenuTarget.category.name)
        : rowMenuTarget?.kind === "group"
        ? strings.contextMenu.actionsFor(conversationDisplayName(rowMenuTarget.group, user?.id ?? null))
        : strings.sidebar.directMessages}
    entries={rowMenuEntries}
    onclose={() => (rowMenuTarget = null)}
  />
  <PersonContextMenu {client} bind:this={personMenu} onopenedconversation={onnavigate} />
  <p class="sr-only" aria-live="polite">{announcement}</p>
  <!-- A refused move: the old order is back, and this says why until closed or the next move. -->
  <div role="alert" class="contents">
    {#if reorderError}
      <p class="mt-xs me-(--mw-component-channel-row-margin-inline) flex items-start gap-xs ps-(--mw-component-channel-row-padding-inline) type-body-sm text-text-danger">
        <Icon name="warning-circle" class="size-(--mw-component-channel-row-trailing-icon-size) shrink-0" />
        <span class="min-w-0 flex-1">{reorderError}</span>
        <IconButton
          icon="x"
          label={strings.common.close}
          class="size-(--mw-component-channel-row-settings-button-size) text-(--mw-component-channel-row-settings-icon) hover:text-(--mw-component-channel-row-settings-icon-hover)"
          iconClass="size-(--mw-component-channel-row-settings-icon-size)"
          onclick={() => (reorderError = "")}
        />
      </p>
    {/if}
  </div>

  <nav
    bind:this={list}
    aria-label={guild ? strings.sidebar.channelsIn(guild.name) : strings.sidebar.directMessages}
    class={[
      "relative -ms-(--mw-component-channel-row-unread-pill-offset) min-h-0 flex-1 overflow-y-auto ps-(--mw-component-channel-row-unread-pill-offset) pb-xs [scrollbar-width:thin]",
      carried && "select-none",
    ]}
  >
    <!-- The negative margin lets the unread pills sit in the strip's margin without being clipped. -->
    {#if guild}
      {@render guildChannels(guild)}
    {:else}
      {@const incomingRequests = client.incomingRequestCount()}
      <!-- Friends sits above the conversations, as on Discord (ADR-0025); its badge counts the
           requests waiting for an answer. -->
      <div class="mt-xs me-(--mw-component-channel-row-margin-inline)">
        <button
          type="button"
          aria-current={client.isFriendsSpaceOpen ? "page" : undefined}
          aria-label={strings.sidebar.friendsWithRequests(incomingRequests)}
          class={[rowClasses(client.isFriendsSpaceOpen, false), "h-(--mw-component-member-row-height) gap-sm pointer-coarse:h-(--mw-size-hit-web-min)"]}
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
                class={[rowClasses(isSelected, hasUnread, isConversationMuted), "h-(--mw-component-member-row-height) gap-sm pointer-coarse:h-(--mw-size-hit-web-min)"]}
                onclick={() => selectConversation(conversation.id)}
                {@attach contextMenuTrigger((origin, invoker) => openConversationMenu(conversation, recipient, origin, invoker))}
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
    {/if}
  </nav>
</div>

{#snippet guildChannels(guild: Guild)}
  {#if sections.every((section) => section.channels.length === 0)}
    <div class="mt-xs me-(--mw-component-channel-row-margin-inline) flex flex-col items-start gap-xs px-(--mw-component-channel-row-padding-inline)">
      {#if canCreateChannels}
        <p class="type-body text-text-muted">{strings.sidebar.noChannelsYet}</p>
        <Button
          variant="primary"
          onclick={() => onopendialog("create-channel", guild.id)}>{strings.sidebar.createAChannel}</Button
        >
      {:else}
        <p class="type-body text-text-muted">{strings.sidebar.noVisibleChannelsYet}</p>
      {/if}
    </div>
  {/if}
  {#each sections as section (section.category?.id ?? "uncategorised")}
    {@const category = section.category}
    {@const isCollapsed = category !== null && collapsedCategoryIds.has(category.id)}
    {@const isCarriedCategory = category !== null && carried?.kind === "category" && carried.id === category.id}
    {#if category}
      {@const isDropTarget = dropCategoryId === category.id}
      <!-- Right-click, long press or Shift+F10: the category's menu; a manager drags it from here. -->
      <h3
        data-order-kind="category"
        data-order-id={category.id}
        class={[
          "mt-(--mw-component-category-margin-top) me-(--mw-component-channel-row-margin-inline) flex items-center gap-2xs rounded-(--mw-component-channel-row-radius)",
          isDropTarget && "bg-(--mw-component-channel-drag-target-background)",
          isCarriedCategory && "cursor-grabbing opacity-(--mw-component-channel-drag-source-opacity)",
        ]}
        {@attach contextMenuTrigger((origin, invoker) => openCategoryMenu(category, origin, invoker))}
        {@attach canCreateChannels && untrack(() => channelDragSource(dragHandlers("category", category.id)))}
      >
        <button
          type="button"
          aria-expanded={!isCollapsed}
          class={[
            "flex h-(--mw-component-category-height) min-w-0 flex-1 items-center gap-2xs rounded-xs type-category transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:text-(--mw-component-category-text-hover) focus-visible:focus-ring-inset pointer-coarse:h-(--mw-size-hit-web-min)",
            isDropTarget ? "text-(--mw-component-channel-drag-target-text)" : "text-(--mw-component-category-text)",
          ]}
          onclick={() => toggleCategory(category.id)}
          onkeydown={(event) => {
            if ((event.key === "ArrowLeft" && !isCollapsed) || (event.key === "ArrowRight" && isCollapsed)) {
              event.preventDefault();
              toggleCategory(category.id);
            }
          }}
        >
          <Icon name={isCollapsed ? "caret-right" : "caret-down"} class="size-(--mw-component-category-chevron-size)" />
          <span class="truncate">{category.name}</span>
        </button>
        {#if canCreateChannels}
          <IconButton
            icon="plus"
            label={strings.sidebar.createChannelIn(category.name)}
            class="size-(--mw-component-category-height) rounded-xs text-(--mw-component-category-text) hover:text-(--mw-component-category-text-hover)"
            iconClass="size-(--mw-component-category-add-icon-size)"
            onclick={() => oncreatechannelin(category)}
          />
        {/if}
      </h3>
    {/if}
    <ul class={[isCarriedCategory && "opacity-(--mw-component-channel-drag-source-opacity)"]}>
      {#each section.channels as channel (channel.id)}
        {#if channel.kind === "voice"}
          {@render voiceChannelRow(guild, channel, isCollapsed)}
        {:else}
        {@const isSelected = channel.id === client.selectedChannelId}
        {@const isChannelMuted = isTargetMuted({ kind: "channel", guildId: guild.id, channelId: channel.id })}
        {@const hasUnread = !isSelected && !isChannelMuted && client.hasUnread(channel.id)}
        {@const mentionCount = isSelected ? 0 : client.mentionCount(channel.id)}
        {@const canSendMessages = canSendIn(channel.id)}
        {@const canEdit = canEditChannel(channel)}
        {@const icon = channel.icon && !failedIconIds.has(channel.icon.id) ? channel.icon : null}
        <!-- Collapsed categories still show the selected and unread channels, as on Discord. -->
        {#if !isCollapsed || isSelected || hasUnread}
          <!-- The gear sits inside the row's menu area: right-click on either opens the channel's menu. -->
          <li
            data-order-kind="channel"
            data-order-id={channel.id}
            class={[
              "group/row relative mt-(--mw-component-channel-row-gap) me-(--mw-component-channel-row-margin-inline)",
              carried?.kind === "channel" &&
                carried.id === channel.id &&
                "cursor-grabbing opacity-(--mw-component-channel-drag-source-opacity)",
            ]}
            {@attach contextMenuTrigger((origin, invoker) => openChannelMenu(channel, origin, invoker))}
            {@attach canCreateChannels && untrack(() => channelDragSource(dragHandlers("channel", channel.id)))}
          >
            {#if hasUnread}{@render unreadPill()}{/if}
            <button
              type="button"
              aria-current={isSelected ? "page" : undefined}
              aria-label={nameWithUnreadState(
                [
                  isChannelMuted ? strings.notifications.withMuted(channel.name) : channel.name,
                  !canSendMessages && strings.common.readOnly,
                ]
                  .filter(Boolean)
                  .join(", "),
                hasUnread,
                mentionCount,
              )}
              class={[
                rowClasses(isSelected, hasUnread, isChannelMuted),
                "h-(--mw-component-channel-row-height) pointer-coarse:h-(--mw-size-hit-web-min)",
                // Room for the gear, so a long name never runs under it; a badge already keeps it.
                canEdit &&
                  mentionCount === 0 &&
                  "pe-[calc(var(--mw-component-channel-row-settings-button-size)+var(--mw-component-channel-row-padding-inline))] pointer-coarse:pe-(--mw-size-hit-web-min)",
              ]}
              onclick={() => selectChannel(channel.id)}
            >
              {#if icon}
                {@const iconId = icon.id}
                <!-- Channel icons are always still (DESIGN.md §6.54): a rounded square, since circles are people. -->
                <img
                  src={iconUrl(icon, false)}
                  alt=""
                  draggable="false"
                  class="size-(--mw-component-channel-row-icon-size) shrink-0 rounded-(--mw-component-channel-row-image-radius) object-cover"
                  onerror={() => failedIconIds.add(iconId)}
                />
              {:else}
                <Icon name={canSendMessages ? "hash" : "lock"} class="size-(--mw-component-channel-row-icon-size)" />
              {/if}
              <span class="min-w-0 flex-1 truncate type-channel-name">
                {channel.name}
              </span>
              {#if icon && !canSendMessages}
                <!-- The icon took the lock's place: it moves after the name. -->
                <Icon name="lock" class="size-(--mw-component-channel-row-trailing-icon-size) shrink-0" />
              {/if}
              {#if isChannelMuted}{@render mutedMark()}{/if}
              {#if mentionCount > 0}
                <!-- The gear takes the badge's place while it shows. -->
                <MentionBadge
                  {mentionCount}
                  class={canEdit ? "group-focus-within/row:invisible group-hover/row:invisible" : ""}
                />
              {/if}
            </button>
            {#if canEdit}
              <!-- Shown on hover, on focus within the row and on the selected row (touch has no hover). -->
              <IconButton
                icon="gear-six"
                label={strings.sidebar.editChannel(channel.name)}
                aria-haspopup="dialog"
                class={[
                  "absolute inset-y-0 end-2xs my-auto size-(--mw-component-channel-row-settings-button-size) text-(--mw-component-channel-row-settings-icon) hover:text-(--mw-component-channel-row-settings-icon-hover) pointer-coarse:end-0",
                  isSelected ? "visible" : "invisible group-focus-within/row:visible group-hover/row:visible",
                ].join(" ")}
                iconClass="size-(--mw-component-channel-row-settings-icon-size)"
                onclick={() => oneditchannel(channel)}
              />
            {/if}
          </li>
        {/if}
        {/if}
      {/each}
    </ul>
  {/each}
  {#if carried}
    {#if dropLineY !== null}
      <!-- Where the carried row lands: apricot, "you, now" (DESIGN.md §5.3). -->
      <div
        aria-hidden="true"
        class="pointer-events-none absolute start-(--mw-component-channel-row-unread-pill-offset) end-(--mw-component-channel-row-margin-inline) z-20 flex -translate-y-1/2 items-center"
        style:top="{dropLineY}px"
      >
        <span
          class="size-(--mw-component-channel-drag-indicator-dot-size) shrink-0 rounded-full border-(length:--mw-component-channel-drag-indicator-height) border-(--mw-component-channel-drag-indicator) bg-(--mw-component-navigator-background)"
        ></span>
        <span class="h-(--mw-component-channel-drag-indicator-height) flex-1 rounded-full bg-(--mw-component-channel-drag-indicator)"></span>
      </div>
    {/if}
    <!-- The carried row, lifted into the floating layer under the pointer. -->
    <div
      aria-hidden="true"
      class="channel-drag-ghost pointer-events-none absolute z-10 flex items-center gap-(--mw-component-channel-row-icon-gap) rounded-(--mw-component-channel-drag-ghost-radius) bg-(--mw-component-channel-drag-ghost-background) px-(--mw-component-channel-row-padding-inline) text-(--mw-component-channel-drag-ghost-text) shadow-high"
      style:top="{carriedY - carried.grabOffset}px"
      style:left="{carried.left}px"
      style:width="{carried.width}px"
      style:height="{carried.height}px"
    >
      {#if carried.kind === "channel"}
        <Icon name={carried.isVoice ? "speaker-high" : "hash"} class="size-(--mw-component-channel-row-icon-size) shrink-0" />
        <span class="min-w-0 truncate type-channel-name">{carried.name}</span>
      {:else}
        <Icon name="caret-down" class="size-(--mw-component-category-chevron-size) shrink-0" />
        <span class="min-w-0 truncate type-category">{carried.name}</span>
      {/if}
    </div>
  {/if}
{/snippet}

{#snippet voiceChannelRow(guild: Guild, channel: Channel, isCollapsed: boolean)}
  {@const participants = client.voiceParticipants(channel.id)}
  {@const isHere = isInVoiceChannel(channel.id)}
  {@const canConnect = canConnectTo(channel.id)}
  {@const canEdit = canEditChannel(channel)}
  {@const limit = channel.user_limit ?? null}
  <!-- Collapsed categories still show voice channels with people in them, as on Discord. -->
  {#if !isCollapsed || participants.length > 0 || isHere}
    <li
      data-order-kind="channel"
      data-order-id={channel.id}
      class={[
        "mt-(--mw-component-channel-row-gap) me-(--mw-component-channel-row-margin-inline)",
        carried?.kind === "channel" && carried.id === channel.id && "cursor-grabbing opacity-(--mw-component-channel-drag-source-opacity)",
      ]}
      {@attach canCreateChannels && untrack(() => channelDragSource(dragHandlers("channel", channel.id)))}
    >
      <!-- Right-click, long press or Shift+F10: the channel's menu, with Join or Leave first. -->
      <div class="group/row relative" {@attach contextMenuTrigger((origin, invoker) => openChannelMenu(channel, origin, invoker))}>
        <button
          type="button"
          aria-label={strings.voice.rowLabel(channel.name, participants.length, limit, isHere, canConnect)}
          class={[
            // Where you are reads bright, without the selected wash: that stays for the open text channel.
            rowClasses(false, isHere),
            "h-(--mw-component-channel-row-height) pointer-coarse:h-(--mw-size-hit-web-min)",
            canEdit &&
              "pe-[calc(var(--mw-component-channel-row-settings-button-size)+var(--mw-component-channel-row-padding-inline))] pointer-coarse:pe-(--mw-size-hit-web-min)",
          ]}
          onclick={() => joinVoiceChannel(channel.id)}
        >
          <Icon name={canConnect ? "speaker-high" : "lock"} class="size-(--mw-component-channel-row-icon-size) shrink-0" />
          <span class="min-w-0 flex-1 truncate type-channel-name">{channel.name}</span>
          {#if limit !== null}
            <!-- The gear takes its place while it shows. -->
            <span
              aria-hidden="true"
              class={[
                "shrink-0 type-body-sm tabular-nums text-(--mw-component-voice-participant-limit-text)",
                canEdit && "group-focus-within/row:invisible group-hover/row:invisible",
              ]}>{participants.length}/{limit}</span
            >
          {/if}
        </button>
        {#if canEdit}
          <IconButton
            icon="gear-six"
            label={strings.sidebar.editChannel(channel.name)}
            aria-haspopup="dialog"
            class="absolute inset-y-0 end-2xs my-auto size-(--mw-component-channel-row-settings-button-size) text-(--mw-component-channel-row-settings-icon) hover:text-(--mw-component-channel-row-settings-icon-hover) invisible group-focus-within/row:visible group-hover/row:visible pointer-coarse:end-0 pointer-coarse:visible"
            iconClass="size-(--mw-component-channel-row-settings-icon-size)"
            onclick={() => oneditchannel(channel)}
          />
        {/if}
      </div>
      {#if participants.length > 0}
        <ul aria-label={strings.voice.participantsIn(channel.name)}>
          {#each participants as state (state.user_id)}
            {@const person = voiceParticipant(guild.id, state)}
            {@const isSpeaking = client.speakingUserIds.has(state.user_id) || (state.user_id === user?.id && client.voice.isSelfSpeaking)}
            {@const isMicrophoneOff = state.self_mute || state.suppressed}
            {@const states = [
              state.self_deaf && strings.voice.soundOff,
              state.self_mute && !state.self_deaf && strings.voice.microphoneOff,
              state.suppressed && strings.voice.cannotSpeak,
              state.user_id !== user?.id && client.voice.personSetting(state.user_id).isLocallyMuted && strings.voice.mutedForYou,
            ].filter((label): label is string => typeof label === "string")}
            {@const isMutedForYou = state.user_id !== user?.id && client.voice.personSetting(state.user_id).isLocallyMuted}
            <li class="me-(--mw-component-channel-row-margin-inline)">
              <!-- A click, right-click, long press or Shift+F10: their menu, with volume and "Mute for me" (stage 16.3). -->
              <button
                type="button"
                aria-haspopup="menu"
                class="flex h-(--mw-component-voice-participant-row-height) w-full min-w-0 items-center gap-(--mw-component-voice-participant-gap) rounded-(--mw-component-channel-row-radius) ps-(--mw-component-voice-participant-indent) pe-(--mw-component-channel-row-padding-inline) text-start hover:bg-(--mw-component-voice-participant-background-hover) focus-visible:focus-ring-inset pointer-coarse:h-(--mw-size-hit-web-min)"
                onclick={(event) => openParticipantMenu(person, event.currentTarget)}
                {@attach contextMenuTrigger((origin, invoker) => personMenu?.openFor(person, origin, invoker, { isInVoice: true }) ?? false)}
              >
              <!-- Speaking is shown, never announced (it would drown everything else out). -->
              <span class={["voice-avatar shrink-0 rounded-full", isSpeaking && "is-speaking"]}>
                <Avatar user={person} sizeToken="--mw-component-voice-participant-avatar-size" />
              </span>
              <span
                class={[
                  "min-w-0 flex-1 truncate type-body-sm",
                  isSpeaking ? "text-(--mw-component-voice-participant-name-text-speaking)" : "text-(--mw-component-voice-participant-name-text)",
                ]}
              >
                {displayName(person)}<span class="sr-only">{states.length > 0 ? `, ${states.join(", ")}` : ""}</span>
              </span>
              {#if isMicrophoneOff && !state.self_deaf}
                <Icon name="microphone-slash" class="size-(--mw-component-voice-participant-state-icon-size) shrink-0 text-(--mw-component-voice-participant-state-icon)" />
              {/if}
              {#if state.self_deaf}
                <Icon name="speaker-slash" class="size-(--mw-component-voice-participant-state-icon-size) shrink-0 text-(--mw-component-voice-participant-state-icon)" />
              {/if}
              {#if isMutedForYou}
                <Icon name="speaker-x" class="size-(--mw-component-voice-participant-state-icon-size) shrink-0 text-(--mw-component-voice-participant-local-mute-icon)" />
              {/if}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </li>
  {/if}
{/snippet}

{#if channelDialog?.kind === "delete"}
  <DeleteChannelDialog
    {client}
    channel={channelDialog.channel}
    onclose={() => (channelDialog = null)}
    ondone={(sentence) => (announcement = sentence)}
    fallbackFocus={() => list?.querySelector<HTMLElement>("button")?.focus()}
  />
{:else if channelDialog?.kind === "rename"}
  <RenameCategoryDialog
    {client}
    category={channelDialog.channel}
    onclose={() => (channelDialog = null)}
    ondone={(sentence) => (announcement = sentence)}
  />
{/if}

{#if movingChannel}
  {@const channel = movingChannel}
  <MoveToCategoryDialog
    {channel}
    {sections}
    onmove={(categoryId) => moveChannelTo(channel, categoryId)}
    onclose={() => (movingChannel = null)}
    fallbackFocus={focusMovedChannel}
  />
{/if}

{#if groupDialog?.kind === "rename"}
  <RenameGroupDialog
    {client}
    group={groupDialog.group}
    onclose={() => (groupDialog = null)}
    ondone={(sentence) => (announcement = sentence)}
  />
{:else if groupDialog?.kind === "leave"}
  {@const invoker = groupDialog.invoker}
  <GroupConfirmDialog
    {client}
    group={groupDialog.group}
    person={null}
    onclose={() => (groupDialog = null)}
    ondone={(sentence) => (announcement = sentence)}
    fallbackFocus={() => invoker.isConnected && invoker.focus()}
  />
{/if}

<style>
  /* Dusk's lamp on whoever is talking: an apricot ring cut from the navigator, fading in and out
     over motion.duration.fast (instant under reduced motion, where the token is 0). */
  .voice-avatar {
    display: inline-flex;
    box-shadow:
      0 0 0 var(--mw-component-voice-participant-speaking-ring-gap) transparent,
      0 0 0 calc(var(--mw-component-voice-participant-speaking-ring-gap) + var(--mw-component-voice-participant-speaking-ring-width)) transparent;
    transition: box-shadow var(--mw-motion-duration-fast) var(--mw-motion-easing-standard);
  }
  .voice-avatar.is-speaking {
    box-shadow:
      0 0 0 var(--mw-component-voice-participant-speaking-ring-gap) var(--mw-component-navigator-background),
      0 0 0 calc(var(--mw-component-voice-participant-speaking-ring-gap) + var(--mw-component-voice-participant-speaking-ring-width)) var(--mw-component-voice-participant-speaking-ring);
  }
  /* The one motion of a drag: the row lifts (DESIGN.md §5.3); reduced motion zeroes the duration. */
  .channel-drag-ghost {
    transform: scale(var(--mw-component-channel-drag-lift-scale));
    animation: channel-drag-lift var(--mw-motion-duration-fast) var(--mw-motion-easing-settle) both;
  }
  @keyframes channel-drag-lift {
    from {
      transform: scale(1);
      box-shadow: none;
    }
  }
</style>

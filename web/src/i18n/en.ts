// The English catalogue: the source of truth for every string the web client shows or announces
// (ADR-0022 §7). `fr.ts` is typed against it, so a key missing there fails `npm run check`.

import type { ApiError } from "../lib/api";
import { MAXIMUM_COUNTED_UNREAD_MENTIONS } from "../lib/protocol";
import { pluralChooser } from "./plural";

const plural = pluralChooser("en");
const count = (value: number) => value.toLocaleString("en");

/** "Ana", "Ana and Bob", "Ana, Bob and Chloé", then "Ana, Bob and 2 others"; "Just you" for nobody. */
function participantNamesInEnglish(names: readonly string[]): string {
  const [first, second, third] = names;
  if (first === undefined) {
    return "Just you";
  }
  if (second === undefined) {
    return first;
  }
  if (names.length === 2) {
    return `${first} and ${second}`;
  }
  if (names.length === 3 && third !== undefined) {
    return `${first}, ${second} and ${third}`;
  }
  const others = names.length - 2;
  return `${first}, ${second} and ${count(others)} ${plural(others, { one: "other", other: "others" })}`;
}

function memberCountInEnglish(members: number): string {
  return `${count(members)} ${plural(members, { one: "member", other: "members" })}`;
}

export const en = {
  common: {
    cancel: "Cancel",
    close: "Close",
    tryAgain: "Try again",
    save: "Save",
    saving: "Saving…",
    edit: "Edit",
    delete: "Delete",
    online: "Online",
    offline: "Offline",
    /** A name followed by its unread state, for screen readers. */
    withUnread: (name: string) => `${name}, unread`,
    unreadSuffix: ", unread",
    /** A name followed by its red badge's count, for screen readers: "general, 3 mentions". */
    withMentions: (name: string, mentions: number) =>
      mentions >= MAXIMUM_COUNTED_UNREAD_MENTIONS
        ? `${name}, ${count(MAXIMUM_COUNTED_UNREAD_MENTIONS - 1)}+ mentions`
        : `${name}, ${plural(mentions, { one: `${count(mentions)} mention`, other: `${count(mentions)} mentions` })}`,
    readOnly: "read-only",
    someone: "Someone",
  },

  /** Status names, keyed by the protocol's values (DESIGN.md §2.3). */
  presence: {
    online: "Online",
    idle: "Idle",
    do_not_disturb: "Do not disturb",
    offline: "Offline",
    invisible: "Invisible",
  },

  errors: {
    /** "Couldn't sign in: the login or the password is wrong." */
    withReason: (summary: string, reason: string) => `${summary}: ${reason}`,
    unknownReason: "something went wrong. Try again.",
    reasons: {
      NETWORK: "the instance could not be reached. Try again.",
      INVALID_CREDENTIALS: "the login or the password is wrong.",
      USERNAME_TAKEN: "this username is taken.",
      EMAIL_TAKEN: "this email already has an account.",
      REGISTRATION_CLOSED: "this instance does not accept new accounts.",
      VALIDATION_FAILED: "the instance refused what was entered.",
      RATE_LIMITED: "too many attempts. Wait a moment and try again.",
      SERVER_BUSY: "the instance is busy. Try again in a moment.",
      MISSING_PERMISSIONS: "you don't have the permission to do that here.",
      NOT_FOUND: "it no longer exists, or you can't see it.",
      INVALID_INVITE: "this invite is unknown, expired or used up.",
      BANNED: "you are banned from this server.",
      OWNER_CANNOT_LEAVE: "the owner cannot leave their own server.",
      LIMIT_REACHED: "a limit of this instance is reached.",
      FILE_TOO_LARGE: "the file is larger than this instance accepts.",
      CONFLICT: "someone else changed it at the same time. Try again.",
    } satisfies Partial<Record<ApiError["code"], string>>,
  },

  app: {
    unreachableTitle: "This instance can't be reached right now.",
    unreachableBody: "Check your connection, then try again.",
  },

  connection: {
    connecting: "Connecting…",
    reconnecting: "Connection lost. Reconnecting…",
  },

  time: {
    todayAt: (time: string) => `Today at ${time}`,
    yesterdayAt: (time: string) => `Yesterday at ${time}`,
  },

  fileSize: {
    // Below 1024, so never grouped: "1023 bytes".
    bytes: (bytes: number) => plural(bytes, { one: `${bytes} byte`, other: `${bytes} bytes` }),
    inUnit: (value: string, unit: "KB" | "MB" | "GB") => `${value} ${unit}`,
  },

  typing: {
    several: "Several people are typing…",
    isTyping: " is typing…",
    and: " and ",
    areTyping: " are typing…",
    listSeparator: ", ",
    andOthersAreTyping: (others: number) =>
      ` and ${count(others)} ${plural(others, { one: "other", other: "others" })} are typing…`,
  },

  signIn: {
    signInTitle: (instance: string) => `Sign in to ${instance}`,
    registerTitle: (instance: string) => `Create an account on ${instance}`,
    signInSubtitle: "Your account is hosted on this instance.",
    registerSubtitle: "Your account will live on this instance.",
    email: "Email",
    username: "Username",
    usernameHelper: "2 to 32 characters: lowercase letters, digits, dots and underscores.",
    password: "Password",
    passwordLengthHelper: (minimum: number) => `At least ${count(minimum)} characters.`,
    login: "Username or email",
    signIn: "Sign in",
    signingIn: "Signing in…",
    createAccount: "Create account",
    creatingAccount: "Creating account…",
    hasAccount: "Already have an account?",
    hasNoAccount: "No account yet?",
    createAnAccount: "Create an account",
    signInFailed: "Couldn't sign in",
    registerFailed: "Couldn't create your account",
    showPassword: "Show password",
  },

  rail: {
    servers: "Servers",
    directMessages: "Direct messages",
    addServer: "Add a server",
  },

  shell: {
    serversAndChannels: "Servers and channels",
    membersOf: (server: string) => `Members of ${server}`,
  },

  sidebar: {
    directMessages: "Direct messages",
    serverMenu: "Server menu",
    invitePeople: "Invite people",
    createChannel: "Create channel",
    createCategory: "Create category",
    serverSettings: "Server settings",
    leaveServer: "Leave server",
    channelsIn: (server: string) => `Channels in ${server}`,
    noConversations: "No conversations yet. Message a friend, or someone from a server's member list.",
    friends: "Friends",
    /** The Friends entry's name, with the count its red badge shows. */
    friendsWithRequests: (incoming: number) =>
      incoming > 0
        ? `Friends, ${count(incoming)} ${plural(incoming, { one: "friend request", other: "friend requests" })}`
        : "Friends",
    noChannelsYet: "No channels yet. Create the first one to get the conversation going.",
    createAChannel: "Create a channel",
    noVisibleChannelsYet: "No channels you can see yet. Ask a moderator for access.",
    createChannelIn: (category: string) => `Create a channel in ${category}`,
    editChannel: (channel: string) => `Edit channel ${channel}`,
  },

  /** Moving channels and categories (DESIGN.md §5.3): what is read out, and what went wrong. */
  channelOrder: {
    channelMovedUp: (channel: string) => `Moved #${channel} up.`,
    channelMovedDown: (channel: string) => `Moved #${channel} down.`,
    channelMovedTo: (channel: string, category: string) => `Moved #${channel} to ${category}.`,
    channelMovedOut: (channel: string) => `Moved #${channel} out of any category.`,
    categoryMovedUp: (category: string) => `Moved ${category} up.`,
    categoryMovedDown: (category: string) => `Moved ${category} down.`,
    categoryMoved: (category: string) => `Moved ${category}.`,
    channelMoveFailed: "Couldn't move the channel",
    categoryMoveFailed: "Couldn't move the category",
    moveTitle: (channel: string) => `Move #${channel}`,
    moveSubtitle: "It goes to the end of the category you choose.",
    destination: "Category",
    noCategory: "No category",
    currentPlace: "Here now",
    move: "Move channel",
    moving: "Moving…",
  },

  /** Deleting a channel or a category, and renaming a category (DESIGN.md §5.3, §8). */
  channelRemoval: {
    deleteChannel: "Delete channel",
    deleteCategory: "Delete category",
    renameCategory: "Rename category",
    channelTitle: (channel: string) => `Delete #${channel}?`,
    channelBody: "All messages in this channel will be deleted for everyone. This can't be undone.",
    confirmLabel: (channel: string) => `Type ${channel} to confirm`,
    categoryTitle: (category: string) => `Delete the category ${category}?`,
    categoryBody: "Its channels stay, without a category. This can't be undone.",
    deleting: "Deleting…",
    channelDeleteFailed: "Couldn't delete the channel",
    categoryDeleteFailed: "Couldn't delete the category",
    channelDeleted: (channel: string) => `Deleted #${channel}.`,
    categoryDeleted: (category: string) => `Deleted the category ${category}.`,
    renaming: "Renaming…",
    renameFailed: "Couldn't rename the category",
    categoryRenamed: (category: string) => `Category renamed to ${category}.`,
  },

  userCard: {
    muteMicrophone: "Mute microphone",
    unmuteMicrophone: "Unmute microphone",
    deafen: "Deafen",
    undeafen: "Undeafen",
    userSettings: "User settings",
    /** The avatar and name button that opens the status menu. */
    statusButtonLabel: (person: string, status: string, customStatus: string | null) =>
      `${person}, ${status}${customStatus ? `, ${customStatus}` : ""}. Set your status`,
    statusMenu: "Your status",
    doNotDisturbHint: "You won't get notifications",
    invisibleHint: "You'll look offline",
    setCustomStatus: "Set a custom status",
    editCustomStatus: "Edit custom status",
    copyUserId: "Copy user ID",
    userIdCopied: "User ID copied",
    copyUserIdFailed: "Couldn't copy your user ID. Allow clipboard access and try again.",
    statusFailed: "Couldn't change your status",
  },

  customStatus: {
    field: "Custom status",
    placeholder: "What are you up to?",
    charactersLeft: (left: number) =>
      left >= 0
        ? `${count(left)} ${plural(left, { one: "character", other: "characters" })} left`
        : `${count(-left)} ${plural(-left, { one: "character", other: "characters" })} too many`,
    save: "Save status",
    clear: "Clear status",
    failed: "Couldn't save your custom status",
    refused: "Couldn't save your custom status: remove line breaks and invisible characters, then try again.",
  },

  chat: {
    chat: "Chat",
    openNavigation: "Open servers and channels",
    goToLabel: "Go to a channel, server or person",
    goTo: "Go to…",
    memberList: "Member list",
    channelSettings: "Channel settings",
    markAsRead: "Mark as read",
    newMessagesSince: (newCount: number, isPartial: boolean, since: string) =>
      `${count(newCount)}${isPartial ? "+" : ""} new ${plural(isPartial ? 2 : newCount, { one: "message", other: "messages" })} since ${since}`,
    composerInChannel: (channel: string) => `Message #${channel}`,
    composerToPerson: (person: string) => `Message @${person}`,
    welcomeToChannel: (channel: string) => `Welcome to #${channel}`,
    channelStart: (channel: string) => `This is the start of the #${channel} channel.`,
    conversationStart: (person: string) => `This is the beginning of your direct message history with ${person}.`,
    composerToGroup: (group: string) => `Message ${group}`,
    groupStart: (people: string) => `This is the beginning of your group conversation with ${people}.`,
    replyingTo: "Replying to",
    replyHint: (person: string) => `Replying to ${person}. Escape cancels the reply.`,
    cancelReply: "Cancel reply",
    attachFiles: "Attach files",
    sendMessage: "Send message",
    cannotSend: "You do not have permission to send messages in this channel.",
    /** The locked composer of a conversation with someone you blocked (ADR-0025). */
    youBlocked: (person: string) => `You blocked ${person}. Unblock them to send messages.`,
    dropToAttach: (target: string) => `Drop to attach to ${target}`,
    dropHint: "You can add a message before sending.",
    tooManyAttachments: (maximum: number) => `You can attach up to ${count(maximum)} files to one message.`,
    uploadsRunning: "Your files are still uploading. Send again once they're done.",
    uploaded: (fileName: string) => `${fileName} uploaded.`,
    fileTooLarge: (fileName: string, size: string, instance: string) =>
      `${fileName} is ${size}, more than ${instance} accepts.`,
    uploadFailed: (fileName: string) => `Couldn't upload ${fileName}`,
    editFailed: "Couldn't edit the message",
    deleteFailed: "Couldn't delete the message",
    copyFailed: "Couldn't copy the text. Select it and copy it by hand.",
    loadingWorkspace: "Loading your servers and conversations…",
    noVisibleChannelsTitle: "No channels you can see",
    noVisibleChannelsBody: (server: string) => `Channels you can read in ${server} will show here.`,
  },

  messages: {
    loading: "Loading messages",
    startOf: (channel: string) => `Start of ${channel}`,
    loadFailed: (channel: string) => `Couldn't load messages in ${channel}.`,
    loadOlderFailed: "Couldn't load older messages.",
    messagesIn: (channel: string) => `Messages in ${channel}`,
    newSince: (time: string) => `New messages since ${time}`,
    newPill: "New",
    viewingOlder: "You're viewing older messages",
    jumpToPresent: "Jump to present",
    originalUnavailable: "Original message was deleted or is not loaded",
    /** A reply to someone you blocked: their words stay folded (ADR-0025). */
    blockedReply: "Blocked message",
    blockedRun: (messages: number) =>
      `${count(messages)} blocked ${plural(messages, { one: "message", other: "messages" })}`,
    showBlocked: "Show",
    hideBlocked: "Hide",
    /** A message read out, or its hidden author prefix: "Ana: hello". */
    authorSaid: (author: string, text: string) => `${author}: ${text}`,
    attachmentCount: (attachments: number) =>
      plural(attachments, { one: `${count(attachments)} attachment`, other: `${count(attachments)} attachments` }),
    edited: "(edited)",
    editedAt: (timestamp: string) => `Edited ${timestamp}`,
    mentionsYou: "Mentions you.",
    spoilerHidden: "Spoiler, select to reveal",
    /** Read out before a revealed spoiler's text. */
    spoilerRevealedPrefix: "Spoiler: ",
    editMessage: "Edit message",
    escTo: "Esc to",
    cancelLowercase: "cancel",
    enterTo: "· Enter to",
    saveLowercase: "save",
    cancelEditing: "Cancel editing",
    saveChanges: "Save changes",
    sending: "Sending…",
    notSent: "Not sent.",
    retry: "Retry",
    retryLabel: "Retry sending this message",
    deleteUnsentLabel: "Delete this unsent message",
    actions: "Message actions",
    reply: "Reply",
    copyText: "Copy text",
    more: "More",
    download: (fileName: string) => `Download ${fileName}`,
  },

  reactions: {
    reactions: "Reactions",
    addReaction: "Add reaction",
    reactWith: (emoji: string) => `React with ${emoji}`,
    /** A chip's name; `aria-pressed` tells whether the user is among them. */
    chip: (emoji: string, reactions: number) =>
      `${emoji}, ${count(reactions)} ${plural(reactions, { one: "reaction", other: "reactions" })}`,
    /** "You, Ana and 3 others reacted with 👍": `names` are other people, up to three. */
    reactedWith: (includesYou: boolean, names: readonly string[], others: number, emoji: string) => {
      const people = [...(includesYou ? ["You"] : []), ...names];
      if (others > 0) {
        people.push(`${count(others)} ${plural(others, { one: "other", other: "others" })}`);
      }
      const list = people.length > 1 ? `${people.slice(0, -1).join(", ")} and ${people.at(-1)}` : (people[0] ?? "");
      // A no-break space keeps the emoji with its verb when the tooltip wraps.
      return `${list} reacted with\u00a0${emoji}`;
    },
    loadingWho: "Loading who reacted…",
    whoFailed: "Couldn't load who reacted.",
    failed: "Couldn't react",
    removeShort: "Remove",
    remove: (person: string) => `Remove ${person}'s reaction`,
    removeFailed: "Couldn't remove the reaction",
    removeAll: "Remove all reactions",
    removeAllTitle: "Remove all reactions?",
    removeAllSubtitle: "Every reaction on this message will be removed for everyone.",
    removeAllSubmit: "Remove all",
    removeAllFailed: "Couldn't remove the reactions",
    noReactions: "No reactions left.",
  },

  emojiPicker: {
    label: "Emoji picker",
    search: "Search emoji",
    placeholder: "Find the perfect emoji",
    recent: "Recently used",
    results: "Search results",
    noResults: (query: string) => `No emoji for “${query}”.`,
    help: "Arrows to move, Enter to choose, Esc to close.",
    categories: {
      people: "People",
      nature: "Nature",
      food: "Food & drink",
      activities: "Activities",
      travel: "Travel & places",
      objects: "Objects",
      symbols: "Symbols",
      flags: "Flags",
    },
  },

  pins: {
    pinnedMessages: "Pinned messages",
    pin: "Pin message",
    unpin: "Unpin message",
    pinned: "Pinned",
    pinTitle: "Pin this message?",
    pinSubtitle: (channel: string) => `It will be listed in the pinned messages of ${channel}.`,
    pinSubmit: "Pin",
    unpinTitle: "Unpin this message?",
    unpinSubtitle: "It will no longer be listed in the pinned messages.",
    unpinSubmit: "Unpin",
    pinFailed: "Couldn't pin the message",
    unpinFailed: "Couldn't unpin the message",
    loading: "Loading pinned messages…",
    loadFailed: "Couldn't load the pinned messages.",
    emptyTitle: "No pinned messages yet",
    emptyBody: "Pin a message from its More menu to keep it here for everyone.",
    jump: "Jump",
    jumpTo: (author: string) => `Jump to ${author}'s message`,
  },

  attachmentTray: {
    filesToSend: "Files to send",
    uploading: "Uploading…",
    remove: (fileName: string) => `Remove ${fileName}`,
  },

  deleteMessage: {
    title: "Delete this message?",
    subtitle: "It will be removed for everyone.",
    submit: "Delete message",
    tip: "Tip: hold Shift while deleting to skip this next time.",
  },

  addServer: {
    joinTitle: "Join a server",
    createTitle: "Create a server",
    joinSubtitle: "Enter an invite to join a server someone already runs.",
    createSubtitle: "Give it a picture, a name and a start. You can change all of it later.",
    join: "Join",
    joining: "Joining…",
    create: "Create",
    creating: "Creating…",
    inviteCode: "Invite code",
    inviteCodeHelper: "Paste the invite code someone gave you.",
    serverName: "Server name",
    hasNoInvite: "Don't have an invite?",
    hasInvite: "Have an invite already?",
    joinFailed: "Couldn't join the server",
    createFailed: "Couldn't create the server",
    iconOptional: "Optional: the initials show until you add one.",
    iconFailed: (server: string) => `${server} was created, but its icon couldn't be saved`,
    iconLater: "Add it from Server settings.",
    startWith: "Start with",
    preview: "Preview",
    previewPlaceholder: "Your server",
    /** Starter templates (DESIGN.md §5.2): names in the interface's language, kept as typed. */
    templates: {
      friends: {
        name: "Friends",
        categories: [{ name: "Text channels", channels: [{ name: "general" }, { name: "photos" }, { name: "plans" }] }],
      },
      community: {
        name: "Community",
        categories: [
          {
            name: "Information",
            channels: [
              { name: "welcome", topic: "Start here: who we are and how things work." },
              { name: "announcements", topic: "News from the people who run the server." },
            ],
          },
          { name: "Community", channels: [{ name: "general" }, { name: "introductions" }, { name: "help" }] },
        ],
      },
      studyGroup: {
        name: "Study group",
        categories: [
          { name: "Information", channels: [{ name: "announcements" }] },
          { name: "Study", channels: [{ name: "general" }, { name: "questions" }, { name: "resources" }, { name: "schedule" }] },
        ],
      },
      gaming: {
        name: "Gaming",
        categories: [{ name: "Text channels", channels: [{ name: "general" }, { name: "clips" }, { name: "looking for group" }] }],
      },
      blank: {
        name: "Blank",
        categories: [{ name: "Text channels", channels: [{ name: "general" }] }],
      },
    },
  },

  createChannel: {
    title: "Create channel",
    titleCategory: "Create category",
    inCategory: (category: string) => `in ${category}`,
    channelType: "Channel type",
    textChannel: "Text channel",
    textChannelDescription: "Messages, pictures and links.",
    category: "Category",
    categoryDescription: "A heading that groups channels in the list.",
    submit: "Create channel",
    submitCategory: "Create category",
    creating: "Creating…",
    channelName: "Channel name",
    categoryName: "Category name",
    topic: "Topic (optional)",
    topicHelper: "Shown at the top of the channel.",
    failed: "Couldn't create the channel",
    failedCategory: "Couldn't create the category",
  },

  invitePeople: {
    title: (server: string) => `Invite friends to ${server}`,
    failed: "Couldn't create an invite",
    copyFailed: "Couldn't copy the code. Select it and copy it by hand.",
    codeLabel: "Send this invite code to a friend",
    creating: "Creating an invite…",
    copy: "Copy",
    copied: "Copied",
    neverExpires: "This invite never expires.",
    codeCopied: "Invite code copied",
  },

  leaveServer: {
    title: (server: string) => `Leave '${server}'`,
    subtitle: (server: string) =>
      `Are you sure you want to leave ${server}? You won't be able to rejoin unless you are re-invited.`,
    submit: "Leave server",
    leaving: "Leaving…",
    failed: "Couldn't leave the server",
  },

  members: {
    members: "Members",
    profile: "Profile",
    profileOf: (person: string) => `Profile of ${person}`,
    backToMembers: "Back to members",
    sendMessage: "Send message",
    opening: "Opening…",
    openConversationFailed: "Couldn't open the conversation",
    addFriend: "Add friend",
    acceptRequest: "Accept friend request",
    requestSent: "Friend request sent",
    friendRequestFailed: "Couldn't send the friend request",
    acceptFailed: "Couldn't accept the friend request",
    loadFailed: "Couldn't load the members.",
    loading: "Loading members…",
    groupLabel: (group: string, members: number) =>
      `${group}, ${count(members)} ${plural(members, { one: "member", other: "members" })}`,
    /** "Ana, idle, custom status: Reviewing PRs" (DESIGN.md §5.7). */
    memberLabel: (person: string, status: string, customStatus: string | null) =>
      `${person}, ${status}${customStatus ? `, custom status: ${customStatus}` : ""}`,
  },

  /** Right-click, long press and Shift+F10 menus (DESIGN.md §6.33, docs/design/web-context-menus.md). */
  contextMenu: {
    /** A menu's name: "Actions for Ana", "Actions for #general", "Actions for Rust Lyon". */
    actionsFor: (name: string) => `Actions for ${name}`,
    message: "Message",
    ignoreRequest: "Ignore request",
    cancelRequest: "Cancel request",
    deleteMessage: "Delete message",
    editChannel: "Edit channel",
    moveUp: "Move up",
    moveDown: "Move down",
    moveToCategory: "Move to category…",
    copyChannelId: "Copy channel ID",
    copyCategoryId: "Copy category ID",
    copyServerId: "Copy server ID",
    userIdCopied: "User ID copied.",
    channelIdCopied: "Channel ID copied.",
    categoryIdCopied: "Category ID copied.",
    serverIdCopied: "Server ID copied.",
    copyIdFailed: "Couldn't copy the ID. Allow clipboard access and try again.",
  },

  /** The Friends space at Home (DESIGN.md §5.6.1, ADR-0025). */
  friends: {
    title: "Friends",
    tabs: {
      online: "Online",
      all: "All",
      pending: "Pending",
      blocked: "Blocked",
      add: "Add friend",
    },
    /** The Pending tab's name, with the count its red badge shows. */
    pendingWithCount: (incoming: number) =>
      `Pending, ${count(incoming)} ${plural(incoming, { one: "request", other: "requests" })} received`,
    groups: {
      online: "Online",
      all: "All friends",
      incoming: "Received",
      outgoing: "Sent",
      blocked: "Blocked",
    },
    groupLabel: (group: string, people: number) =>
      `${group}, ${count(people)} ${plural(people, { one: "person", other: "people" })}`,
    incomingRequest: "Incoming friend request",
    outgoingRequest: "Outgoing friend request",
    blockedDetail: "Blocked",
    message: (person: string) => `Message ${person}`,
    more: (person: string) => `More actions for ${person}`,
    menu: (person: string) => `Actions for ${person}`,
    accept: (person: string) => `Accept ${person}'s request`,
    ignore: (person: string) => `Ignore ${person}'s request`,
    cancel: (person: string) => `Cancel the request to ${person}`,
    unblock: "Unblock",
    unblockLabel: (person: string) => `Unblock ${person}`,
    removeFriend: "Remove friend",
    block: "Block",
    accepted: (person: string) => `You're now friends with ${person}.`,
    ignored: (person: string) => `Request from ${person} ignored.`,
    cancelled: (person: string) => `Request to ${person} cancelled.`,
    removed: (person: string) => `${person} was removed from your friends.`,
    blocked: (person: string) => `${person} was blocked.`,
    unblocked: (person: string) => `${person} was unblocked.`,
    acceptFailed: (person: string) => `Couldn't accept ${person}'s request`,
    ignoreFailed: (person: string) => `Couldn't ignore ${person}'s request`,
    cancelFailed: (person: string) => `Couldn't cancel the request to ${person}`,
    removeFailed: (person: string) => `Couldn't remove ${person} from your friends`,
    blockFailed: (person: string) => `Couldn't block ${person}`,
    unblockFailed: (person: string) => `Couldn't unblock ${person}`,
    emptyOnline: "None of your friends are around right now.",
    noFriendsYet: "No friends yet. Add someone by their username and they'll show up here.",
    emptyPending: "No pending requests. Requests you send or receive wait here.",
    emptyBlocked: "You haven't blocked anyone.",
    addFriend: "Add friend",
    addServer: "Add a server",
    addTitle: "Add friend",
    addSubtitle: (instance: string) => `You can add friends with their username on ${instance}.`,
    username: "Username",
    usernamePlaceholder: "Their username",
    send: "Send friend request",
    sending: "Sending…",
    sent: (person: string) => `Friend request sent to ${person}.`,
    alreadySent: (person: string) => `You already asked ${person}. The request is waiting for their answer.`,
    alreadyFriends: (person: string) => `You're already friends with ${person}.`,
    acceptedTheirs: (person: string) => `${person} had asked you too: you're now friends.`,
    /** The same words for a missing account, yourself and a block (ADR-0025). */
    notFound: "Hm, that didn't work. Check the username and try again.",
    rateLimited: "You've sent a lot of requests. Wait a moment, then try again.",
    /** The sender's own cap on friends and requests sent (ADR-0025). */
    limitReached:
      "You have as many friends and sent requests as an account can. Cancel a request or remove a friend to add someone new.",
    sendFailed: "Couldn't send the friend request",
    removeTitle: (person: string) => `Remove ${person} from your friends?`,
    removeBody: (person: string) =>
      `${person} won't be in your friends anymore. To be friends again, one of you sends a new request.`,
    removing: "Removing…",
    blockTitle: (person: string) => `Block ${person}?`,
    blockBody: (person: string) =>
      `${person} won't be told. You won't be able to message each other or send friend requests, their messages in servers will be hidden, and they'll leave your friends.`,
    blockSubmit: (person: string) => `Block ${person}`,
    blocking: "Blocking…",
  },

  /** Group conversations and the "New message" picker (DESIGN.md §5.6, ADR-0026). */
  groups: {
    /** Everyone in a conversation but you, as its name reads when it has none of its own. */
    participantNames: (names: readonly string[]) => participantNamesInEnglish(names),
    memberCount: (members: number) => memberCountInEnglish(members),
    /** A group row's accessible name: "Trip to Lyon, group, 4 members". */
    rowLabel: (group: string, members: number) => `${group}, group, ${memberCountInEnglish(members)}`,
    group: "Group",
    newMessage: "New message",
    pickerSubtitle: "Pick one friend to message them, or several to start a group.",
    addTitle: (group: string) => `Add friends to ${group}`,
    addSubtitle: "They'll see the whole conversation, older messages included.",
    search: "Search your friends",
    searchPlaceholder: "A friend's name",
    roomLeft: (room: number) =>
      room > 0
        ? `You can add ${count(room)} more ${plural(room, { one: "friend", other: "friends" })}.`
        : "That's as many people as a group can hold.",
    picked: "Friends you picked",
    unpick: (person: string) => `Remove ${person}`,
    groupName: "Group name (optional)",
    groupNameHelper: "Leave it empty to show everyone's names.",
    message: "Message",
    createGroup: "Create group",
    creating: "Creating…",
    addToGroup: "Add to group",
    adding: "Adding…",
    noFriends: "You don't have friends to message yet. Add a friend by their username first.",
    addFriend: "Add friend",
    everyoneIsIn: "All your friends are already in this group.",
    noMatch: (query: string) => `No friends match “${query}”.`,
    createFailed: "Couldn't create the group",
    /** NOT_FOUND when creating: not a friend any more, or a block either way, never told apart (ADR-0026). */
    cannotJoin: "one of these people can't join it. Check your friends list and try again.",
    addFailed: (person: string) => `Couldn't add ${person} to the group`,
    addNotFound: "they can't join this group. Check your friends list and try again.",
    addFull: "the group is full.",
    added: (names: readonly string[]) => `${participantNamesInEnglish(names)} joined the group.`,
    members: "Members",
    owner: "Owner",
    /** "Ana, online, owner, custom status: Reviewing PRs". */
    memberLabel: (person: string, status: string, isOwner: boolean, customStatus: string | null) =>
      `${person}, ${status}${isOwner ? ", owner" : ""}${customStatus ? `, custom status: ${customStatus}` : ""}`,
    addFriends: "Add friends",
    addFriendsToGroup: "Add friends to the group",
    rename: "Rename group",
    leave: "Leave group",
    remove: (person: string) => `Remove ${person} from the group`,
    nameLabel: "Group name",
    saveName: "Save name",
    renamed: (group: string) => `Group renamed to ${group}.`,
    nameCleared: "The group shows everyone's names again.",
    renameFailed: "Couldn't rename the group",
    leaveTitle: (group: string) => `Leave ${group}?`,
    leaveBody: "You won't see this group or its messages anymore. Someone in it can add you back later.",
    leaveBodyAsOwner: (nextOwner: string) =>
      `You won't see this group or its messages anymore, and ${nextOwner} will own it. Someone in it can add you back later.`,
    leaveBodyAsLast: "You're the last one here: the group and its messages will be deleted for good.",
    leaving: "Leaving…",
    leaveFailed: "Couldn't leave the group",
    removeTitle: (person: string) => `Remove ${person} from the group?`,
    removeBody: (person: string) =>
      `${person} won't see this group or its messages anymore. Someone in it can add them back later.`,
    removeSubmit: (person: string) => `Remove ${person}`,
    removing: "Removing…",
    removed: (person: string) => `${person} was removed from the group.`,
    removeFailed: (person: string) => `Couldn't remove ${person} from the group`,
    /** Back on Friends after the open group closed (DESIGN.md §5.6). */
    left: (group: string) => `You left the group “${group}”.`,
    removedYou: (group: string) => `You're no longer in the group “${group}”.`,
  },

  quickSwitcher: {
    label: "Go to",
    placeholder: "Where would you like to go?",
    results: "Results",
    channels: "Channels",
    people: "People",
    servers: "Servers",
    directMessage: "Direct message",
    server: "Server",
    noResults: (query: string) => `No results for “${query}”. Check the spelling or search another server.`,
    resultCount: (results: number) =>
      `${count(results)} ${plural(results, { one: "result", other: "results" })}`,
    help: "Arrows to choose, Enter to open, Esc to close. Prefixes: @ people, # channels, * servers.",
    openConversationFailed: (person: string) => `Couldn't open the conversation with ${person}`,
  },

  serverSettings: {
    serverSettings: "Server settings",
    moderation: "Moderation",
    sections: {
      overview: "Overview",
      roles: "Roles",
      invites: "Invites",
      members: "Members",
      bans: "Bans",
      "moderation-log": "Moderation log",
    },
    deleteServer: "Delete server",
    backToServerSettings: "Back to server settings",
    loading: "Loading…",

    unsaved: {
      careful: "Careful — you have unsaved changes.",
      saveOrReset: "Save or reset your changes before leaving.",
      reset: "Reset",
      saveChanges: "Save changes",
      saving: "Saving…",
      saved: "Changes saved",
    },

    overview: {
      serverName: "Server name",
      serverNameHelper: "Up to 100 characters. Everyone in the server sees it.",
      nameRequired: "Give the server a name.",
      saveFailed: "Couldn't save the server name",
    },

    deleteServerDialog: {
      title: (server: string) => `Delete '${server}'`,
      body: (server: string) =>
        `This deletes ${server} for everyone, with its channels, messages, roles and invites. It can't be undone.`,
      confirmLabel: "Enter the server name to confirm",
      submit: "Delete server",
      deleting: "Deleting…",
      failed: "Couldn't delete the server",
    },

    roles: {
      createRole: "Create role",
      creating: "Creating…",
      newRoleName: "new role",
      empty: "Roles let you group people and give them permissions.",
      roleList: "Roles, highest first",
      memberCount: (members: number) =>
        plural(members, { one: `${count(members)} member`, other: `${count(members)} members` }),
      locked: "You can only edit roles below your highest role.",
      lockedRole: (role: string) => `${role}, locked`,
      display: "Display",
      permissions: "Permissions",
      roleName: "Role name",
      everyoneCannotBeRenamed: "@everyone can't be renamed.",
      everyoneNote: "Everyone in the server has @everyone. Its permissions apply to all members.",
      color: "Role colour",
      noColor: "No colour",
      colorOption: (hex: string) => `Colour ${hex}`,
      colorPreview: "Names with this role",
      moveUp: "Move up",
      moveDown: "Move down",
      moveHint: "Ctrl+Shift+Up and Ctrl+Shift+Down move the selected role too.",
      deleteRole: "Delete role",
      deleteTitle: (role: string) => `Delete the role '${role}'?`,
      deleteBody: (role: string) => `Members with ${role} lose it, with the permissions it gives. This can't be undone.`,
      deleting: "Deleting…",
      nameRequired: "Give the role a name.",
      sensitive: "Sensitive",
      permissionLocked: "You can only grant or remove permissions you have.",
      createFailed: "Couldn't create the role",
      saveFailed: "Couldn't save the role",
      deleteFailed: "Couldn't delete the role",
      moveFailed: "Couldn't move the role",
      groups: {
        general: "General",
        membership: "Membership",
        textChannels: "Text channels",
        moderation: "Moderation",
        advanced: "Advanced",
      },
    },

    permissions: {
      viewChannels: { label: "View channels", description: "See text channels and read what is posted there." },
      manageChannels: { label: "Manage channels", description: "Create, edit and delete channels." },
      manageRoles: {
        label: "Manage roles",
        description: "Create roles, and edit or delete the roles below their highest role.",
      },
      manageServer: {
        label: "Manage server",
        description: "Rename the server and manage its invites.",
      },
      viewModerationLog: { label: "View moderation log", description: "See who changed what on this server." },
      createInvite: { label: "Create invite", description: "Invite new people to this server." },
      sendMessages: { label: "Send messages", description: "Write in text channels." },
      readMessageHistory: {
        label: "Read message history",
        description: "Read messages sent before they opened the channel.",
      },
      attachFiles: { label: "Attach files", description: "Add files and pictures to their messages." },
      addReactions: { label: "Add reactions", description: "React to messages with a new emoji." },
      mentionEveryone: { label: "Mention @everyone", description: "Notify everyone in a channel at once." },
      manageMessages: { label: "Manage messages", description: "Delete messages sent by other members." },
      kickMembers: { label: "Kick members", description: "Remove people; they can come back with a new invite." },
      banMembers: { label: "Ban members", description: "Remove people and stop them from rejoining." },
      administrator: {
        label: "Administrator",
        description: "Every permission in every channel, whatever the channel settings. Grant it with care.",
      },
    },

    members: {
      search: "Search members",
      searchHelper: "By display name or username.",
      noMatch: (query: string) => `No members match “${query}”. Check the spelling.`,
      loadFailed: "Couldn't load the members.",
      owner: "Owner",
      memberRoles: (person: string) => `Roles of ${person}`,
      manage: "Manage",
      manageLabel: (person: string) => `Manage ${person}`,
      addRole: "Add role…",
      addRoleTo: (person: string) => `Add a role to ${person}`,
      removeRole: (role: string, person: string) => `Remove ${role} from ${person}`,
      kick: "Kick",
      ban: "Ban",
      kickLabel: (person: string) => `Kick ${person}`,
      banLabel: (person: string) => `Ban ${person}`,
      kickTitle: (person: string, server: string) => `Kick ${person} from ${server}?`,
      kickBody: (person: string) => `${person} will be removed from the server. They can rejoin with a new invite.`,
      kicking: "Kicking…",
      banTitle: (person: string, server: string) => `Ban ${person} from ${server}?`,
      banBody: (person: string) => `${person} will be removed and won't be able to rejoin, even with an invite.`,
      banReason: "Reason (optional, visible to moderators)",
      banning: "Banning…",
      kickFailed: (person: string) => `Couldn't kick ${person}`,
      banFailed: (person: string) => `Couldn't ban ${person}`,
      roleFailed: "Couldn't change the roles",
      kicked: (person: string) => `${person} was kicked`,
      banned: (person: string) => `${person} was banned`,
    },

    bans: {
      empty: (server: string) => `No one is banned from ${server}.`,
      loadFailed: "Couldn't load the bans.",
      noReason: "No reason given.",
      bannedOn: (date: string) => `Banned ${date}`,
      revoke: "Revoke ban",
      revokeLabel: (person: string) => `Revoke ban for ${person}`,
      revoking: "Revoking…",
      revokeFailed: (person: string) => `Couldn't revoke the ban of ${person}`,
      revoked: (person: string) => `${person} can join again`,
    },

    invites: {
      empty: "No active invites.",
      loadFailed: "Couldn't load the invites.",
      createInvite: "Create invite",
      creating: "Creating…",
      createFailed: "Couldn't create an invite",
      created: (code: string) => `Invite ${code} created`,
      createdBy: (person: string) => `Created by ${person}`,
      uses: (uses: number, maximum: number | null) =>
        maximum === null
          ? plural(uses, { one: `${count(uses)} use`, other: `${count(uses)} uses` })
          : `${count(uses)} of ${count(maximum)} uses`,
      expires: (date: string) => `Expires ${date}`,
      neverExpires: "Never expires",
      revoke: "Revoke",
      revokeLabel: (code: string) => `Revoke invite ${code}`,
      revokeTitle: "Revoke this invite?",
      revokeBody: "The link will stop working. People who already joined stay in the server.",
      revokeSubmit: "Revoke invite",
      revoking: "Revoking…",
      revokeFailed: "Couldn't revoke the invite",
    },

    moderationLog: {
      empty: "Nothing recorded yet. Changes to roles, channels, invites and members will show here.",
      loadFailed: "Couldn't load the moderation log.",
      loadOlder: "Load older entries",
      loadingOlder: "Loading older entries…",
      loadOlderFailed: "Couldn't load older entries.",
      reason: (reason: string) => `Reason: ${reason}`,
      deletedAccount: "A deleted account",
      formerMember: "a former member",
      deletedRole: "a deleted role",
      deletedChannel: "a deleted channel",
      channelCreate: (actor: string, channel: string) => `${actor} created #${channel}`,
      channelUpdate: (actor: string, channel: string) => `${actor} edited #${channel}`,
      channelDelete: (actor: string) => `${actor} deleted a channel`,
      overwriteSet: (actor: string, target: string, channel: string) =>
        `${actor} changed what ${target} can do in #${channel}`,
      overwriteDelete: (actor: string, target: string, channel: string) =>
        `${actor} removed the permissions of ${target} in #${channel}`,
      roleCreate: (actor: string, role: string) => `${actor} created the role ${role}`,
      roleUpdate: (actor: string, role: string) => `${actor} edited the role ${role}`,
      roleDelete: (actor: string) => `${actor} deleted a role`,
      rolesReorder: (actor: string) => `${actor} reordered the roles`,
      channelsReorder: (actor: string) => `${actor} reordered the channels`,
      memberRoleAdd: (actor: string, role: string, member: string) => `${actor} gave ${role} to ${member}`,
      memberRoleRemove: (actor: string, role: string, member: string) => `${actor} took ${role} from ${member}`,
      memberKick: (actor: string, member: string) => `${actor} kicked ${member}`,
      memberBanAdd: (actor: string, member: string) => `${actor} banned ${member}`,
      memberBanRemove: (actor: string, member: string) => `${actor} revoked the ban of ${member}`,
      messageDelete: (actor: string, author: string, channel: string) =>
        `${actor} deleted a message by ${author} in #${channel}`,
      messagePin: (actor: string, channel: string) => `${actor} pinned a message in #${channel}`,
      messageUnpin: (actor: string, channel: string) => `${actor} unpinned a message in #${channel}`,
      messageReactionRemove: (actor: string, channel: string) =>
        `${actor} removed reactions from a message in #${channel}`,
      guildRename: (actor: string, before: string, after: string) =>
        `${actor} renamed the server from ${before} to ${after}`,
      guildUpdate: (actor: string) => `${actor} edited the server`,
      inviteCreate: (actor: string, code: string) => `${actor} created the invite ${code}`,
      inviteDelete: (actor: string, code: string) => `${actor} revoked the invite ${code}`,
      unknown: (actor: string) => `${actor} made a change this version of Mellow can't show yet`,
      nameChange: (before: string, after: string) => `Name: ${before} to ${after}`,
      topicChange: (topic: string) => `New topic: ${topic}`,
      topicCleared: "Topic removed",
      colorChange: "Colour changed",
      permissionsAdded: (key: "permissions" | "allow" | "deny", names: string) =>
        key === "deny" ? `Now denied: ${names}` : key === "allow" ? `Now allowed: ${names}` : `Granted: ${names}`,
      permissionsRemoved: (key: "permissions" | "allow" | "deny", names: string) =>
        key === "deny" ? `No longer denied: ${names}` : key === "allow" ? `No longer allowed: ${names}` : `Removed: ${names}`,
    },
  },

  channelSettings: {
    channelSettings: "Channel settings",
    /** The settings dialog's name for screen readers: "Channel settings, general". */
    channelSettingsOf: (channel: string) => `Channel settings, ${channel}`,
    sections: {
      overview: "Overview",
      permissions: "Permissions",
    },
    backToChannelSettings: "Back to channel settings",

    overview: {
      channelName: "Channel name",
      channelNameHelper: "Up to 100 characters.",
      topic: "Topic",
      topicHelper: "Shown at the top of the channel. Up to 1,024 characters.",
      nameRequired: "Give the channel a name.",
      saveFailed: "Couldn't save the channel",
    },

    permissions: {
      intro: (channel: string) =>
        `Choose who can see and use #${channel}. Inherit keeps what their roles allow on the whole server.`,
      targets: "Roles and members",
      add: "Add a role or member",
      addTitle: (channel: string) => `Add to #${channel}`,
      addSubtitle: "Its permissions here start on Inherit; change them once it is in the list.",
      search: "Search roles and members",
      roles: "Roles",
      members: "Members",
      noMatch: (query: string) => `No roles or members match “${query}”. Check the spelling.`,
      nothingToAdd: "Every role and member you can manage is already in the list.",
      loadingMembers: "Loading members…",
      addFailed: "Couldn't add it to the channel",
      remove: "Remove",
      removeLabel: (name: string) => `Remove ${name} from this channel's permissions`,
      removing: "Removing…",
      removeFailed: "Couldn't remove it from the channel",
      saveFailed: "Couldn't save the permissions",
      lockedTarget: "You can only change roles below your highest role.",
      lockedTargetRow: (name: string) => `${name}, locked`,
      lockedPermissions: "You can only change permissions you have in this channel.",
      everyoneNote: "@everyone applies to everyone in the server. Roles and members in the list override it.",
      states: { deny: "Deny", inherit: "Inherit", allow: "Allow" },
      /** Channel words where the role editor's server-wide words would mislead. */
      channelWords: {
        viewChannels: { label: "View channel", description: "See this channel and read what is posted in it." },
        manageChannels: { label: "Manage channel", description: "Rename this channel and change its topic and icon." },
        manageRoles: { label: "Manage permissions", description: "Change who can do what in this channel." },
      },
    },
  },

  iconEditor: {
    formats: (maximumSize: string) =>
      `PNG, JPEG, GIF or WebP, up to ${maximumSize}. You choose the square once the picture is open.`,
    server: {
      title: "Server icon",
      emptyText: "The server's initials show until you add an icon.",
      note: "Shown in the server list. Animated GIFs play while the pointer is on them.",
    },
    channel: {
      title: "Channel icon",
      emptyText: "The # shows until you add an icon.",
      note: "Shown small and still, in place of the # beside the channel's name.",
    },
    upload: "Upload icon",
    uploading: "Uploading icon…",
    remove: "Remove icon",
    removing: "Removing icon…",
    saved: "Icon saved.",
    removed: "Icon removed.",
    changeFailed: "Couldn't change the icon",
    removeFailed: "Couldn't remove the icon",
    crop: {
      title: "Choose the square",
      subtitle: "Drag the picture and zoom until the square holds what you want to show. The rest is cut away.",
      stage: "Picture to crop",
      stageHint: "Arrow keys move the picture, Shift moves it further, + and − zoom.",
      zoom: "Zoom",
      centre: "Centre it",
      samples: "At its real size",
      sampleAtRest: "In the list",
      sampleSelected: "Selected",
      opening: "Opening the picture…",
      save: "Save icon",
      saving: "Uploading…",
    },
  },

  settings: {
    userSettings: "User settings",
    appSettings: "App settings",
    myAccount: "My account",
    profile: "Profile",
    about: "About",
    closeSettings: "Close settings",
    escapeKey: "Esc",
    signOut: "Sign out",
    signOutFailed: "Couldn't sign out",
    backToUserSettings: "Back to user settings",
    webClient: "Mellow web client",
    sourceCode: "Source code",
    licence: "Mellow is free software under the AGPL-3.0 licence. You can read, change and share its code.",
    sourceCodeOnGitHub: "Source code on GitHub",
  },

  account: {
    displayName: "Display name",
    username: "Username",
    email: "Email",
    password: "Password",
    editDisplayName: "Edit display name",
    editUsername: "Edit username",
    changeDisplayName: "Change display name",
    changeUsername: "Change username",
    changePassword: "Change password",
    changingPassword: "Changing password…",
    displayNameHelper: "Up to 32 characters. This is the name people see in chat.",
    currentPassword: "Current password",
    newPassword: "New password",
    confirmNewPassword: "Confirm new password",
    passwordsDiffer: "Doesn't match the new password.",
    wrongCurrentPassword: "the current password is wrong.",
    displayNameFailed: "Couldn't save the display name",
    usernameFailed: "Couldn't change the username",
    passwordFailed: "Couldn't change the password",
    displayNameSaved: "Display name saved.",
    usernameSaved: "Username saved.",
    passwordChanged: "Password changed. Your other devices are signed out.",
  },

  profile: {
    formats: (maximumSize: string) =>
      `Pictures can be PNG, JPEG, GIF or WebP, up to ${maximumSize}. Animated GIFs stay animated.`,
    tooLarge: (maximumSize: string) => `the picture is larger than ${maximumSize}. Choose a smaller one.`,
    unreadable: "the picture couldn't be read, or it's over 8192 pixels a side or 600 frames. Choose another one.",
    notAPicture: "choose a PNG, JPEG, GIF or WebP picture.",
    preview: "Preview",
    avatar: {
      title: "Avatar",
      emptyText: "Your initial shows until you add a picture.",
      saved: "Avatar saved.",
      removed: "Avatar removed.",
      change: "Change avatar",
      uploading: "Uploading avatar…",
      remove: "Remove avatar",
      removing: "Removing avatar…",
      changeFailed: "Couldn't change the avatar",
      removeFailed: "Couldn't remove the avatar",
    },
    banner: {
      title: "Profile banner",
      emptyText: "A plain strip shows until you add a picture.",
      saved: "Banner saved.",
      removed: "Banner removed.",
      change: "Change banner",
      uploading: "Uploading banner…",
      remove: "Remove banner",
      removing: "Removing banner…",
      changeFailed: "Couldn't change the banner",
      removeFailed: "Couldn't remove the banner",
    },
  },
};

export type Catalogue = typeof en;

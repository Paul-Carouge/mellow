// The English catalogue: the source of truth for every string the web client shows or announces
// (ADR-0022 §7). `fr.ts` is typed against it, so a key missing there fails `npm run check`.

import type { ApiError } from "../lib/api";
import { pluralChooser } from "./plural";

const plural = pluralChooser("en");
const count = (value: number) => value.toLocaleString("en");

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
    readOnly: "read-only",
    someone: "Someone",
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
    leaveServer: "Leave server",
    channelsIn: (server: string) => `Channels in ${server}`,
    noConversations:
      "No conversations yet. Open someone's profile in a server's member list and choose Send message.",
    justYou: "Just you",
    noChannelsYet: "No channels yet. Create the first one to get the conversation going.",
    createAChannel: "Create a channel",
    noVisibleChannelsYet: "No channels you can see yet. Ask a moderator for access.",
    createChannelIn: (category: string) => `Create a channel in ${category}`,
  },

  userCard: {
    muteMicrophone: "Mute microphone",
    unmuteMicrophone: "Unmute microphone",
    deafen: "Deafen",
    undeafen: "Undeafen",
    userSettings: "User settings",
  },

  chat: {
    chat: "Chat",
    openNavigation: "Open servers and channels",
    goToLabel: "Go to a channel, server or person",
    goTo: "Go to…",
    memberList: "Member list",
    markAsRead: "Mark as read",
    newMessagesSince: (newCount: number, isPartial: boolean, since: string) =>
      `${count(newCount)}${isPartial ? "+" : ""} new ${plural(isPartial ? 2 : newCount, { one: "message", other: "messages" })} since ${since}`,
    composerInChannel: (channel: string) => `Message #${channel}`,
    composerToPerson: (person: string) => `Message @${person}`,
    welcomeToChannel: (channel: string) => `Welcome to #${channel}`,
    channelStart: (channel: string) => `This is the start of the #${channel} channel.`,
    conversationStart: (person: string) => `This is the beginning of your direct message history with ${person}.`,
    replyingTo: "Replying to",
    replyHint: (person: string) => `Replying to ${person}. Escape cancels the reply.`,
    cancelReply: "Cancel reply",
    attachFiles: "Attach files",
    sendMessage: "Send message",
    cannotSend: "You do not have permission to send messages in this channel.",
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
    noServersTitle: "No servers yet",
    noServersBody: "Create a server for your group, or join one with an invite code.",
    addServer: "Add a server",
    noConversationsTitle: "No conversations yet",
    noConversationsBody:
      "Start one with someone from your servers: open their profile in the member list and choose Send message.",
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
    download: (fileName: string) => `Download ${fileName}`,
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
    createSubtitle: "Give your server a name. You can add channels and invite people next.",
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
  },

  createChannel: {
    title: "Create channel",
    inCategory: (category: string) => `in ${category}`,
    submit: "Create channel",
    creating: "Creating…",
    channelName: "Channel name",
    placeholder: "new-channel",
    helper: "Names are lowercase, with dashes instead of spaces.",
    failed: "Couldn't create the channel",
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
    loadFailed: "Couldn't load the members.",
    loading: "Loading members…",
    groupLabel: (group: string, members: number) =>
      `${group}, ${count(members)} ${plural(members, { one: "member", other: "members" })}`,
    memberLabel: (person: string, isOnline: boolean) => `${person}, ${isOnline ? "online" : "offline"}`,
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

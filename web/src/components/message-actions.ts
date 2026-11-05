// What the chat pane lets a message row do with reactions, pins, polls, checklists and progress
// bars, passed down as one object.

import type { Message, PublicUser, Snowflake } from "../lib/protocol";

export interface ReactionAndPinActions {
  /** Joining an existing reaction, and seeing who reacted: the channel's history is readable. */
  canJoinReactions: boolean;
  /** A new emoji: `ADD_REACTIONS` too. */
  canAddReactions: boolean;
  /** Other people's reactions: `MANAGE_MESSAGES` in a server channel. */
  canRemoveReactions: boolean;
  /** `MANAGE_MESSAGES` in a server channel; any participant in a conversation. */
  canManagePins: boolean;
  /** The message whose emoji picker is open keeps its hover bar shown, so focus can go back. */
  heldMessageId: Snowflake | null;
  toggleReaction: (message: Message, emoji: string) => void;
  openPicker: (message: Message, anchor: HTMLElement) => void;
  loadReactors: (message: Message, emoji: string) => Promise<PublicUser[]>;
  askToPin: (message: Message, isPinned: boolean) => void;
  viewReactions: (message: Message) => void;
  askToRemoveAllReactions: (message: Message) => void;
  /** Reading a server channel's history, or writing in a conversation (ADR-0043). */
  canVoteInPolls: boolean;
  choosePollAnswer: (message: Message, answerId: number) => void;
  removeOwnPollVotes: (message: Message) => void;
  viewPollVoters: (message: Message, answerId: number) => void;
  askToEndPoll: (message: Message) => void;
  /** Writing here (ADR-0047). */
  canTickChecklists: boolean;
  /** Its author while they may write here, or a moderator of a server channel. */
  canEditTracker: (message: Message) => boolean;
  nameOfUser: (userId: Snowflake) => string;
  toggleChecklistItem: (message: Message, itemId: number) => void;
  /** Each resolves `true` once the instance took the change; a refusal is shown by the pane. */
  addChecklistItem: (message: Message, text: string) => Promise<boolean>;
  renameChecklistItem: (message: Message, itemId: number, text: string) => Promise<boolean>;
  removeChecklistItem: (message: Message, itemId: number) => Promise<boolean>;
  updateProgressBar: (message: Message) => void;
}

/** The keys of a message's hover bar after its reactions, in order (DESIGN.md §5.4). */
export type HoverBarAction = "edit" | "reply" | "forward" | "copy-text" | "delete" | "more";

export interface HoverBarFacts {
  readonly isOwn: boolean;
  /** A forward copies someone's words: it cannot be edited (ADR-0039). */
  readonly isForward: boolean;
  /** A poll is neither edited, forwarded nor copied as text (ADR-0043). */
  readonly isPoll: boolean;
  readonly hasText: boolean;
  readonly canSendMessages: boolean;
  /** Delete others' messages: `MANAGE_MESSAGES`. */
  readonly canManageMessages: boolean;
  /** Shift is held over the row with a mouse: Delete joins the bar, and a click on it skips the confirmation. */
  readonly isShiftHeld: boolean;
  /** The message menu has items, so More has something to open. */
  readonly hasMenu: boolean;
}

/**
 * Edit takes Reply's place on a message you can edit, as on Discord; Delete stays in More unless
 * Shift is held, so the everyday bar stays short.
 */
export function hoverBarActions(facts: HoverBarFacts): HoverBarAction[] {
  const canEdit = facts.isOwn && facts.canSendMessages && !facts.isForward && !facts.isPoll;
  const actions: (HoverBarAction | false)[] = [
    canEdit ? "edit" : facts.canSendMessages && "reply",
    !facts.isPoll && "forward",
    !facts.isPoll && facts.hasText && "copy-text",
    facts.isShiftHeld && (facts.isOwn || facts.canManageMessages) && "delete",
    facts.hasMenu && "more",
  ];
  return actions.filter((action): action is HoverBarAction => action !== false);
}

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

// What the chat pane lets a message row do with reactions and pins, passed down as one object.

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
}

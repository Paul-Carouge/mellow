// The two ways a person's profile shows (DESIGN.md §5.7.1 to §5.7.3): the hover card, after the
// pointer or keyboard focus rests on a name, and the profile modal, on a click. Both are rendered
// once by the shell (the card also inside each drawer, see `ProfileHoverCard`), outside the rows
// that open them: a row's own handlers, keys and menus never reach them, and a row that leaves
// (a member gone, a message deleted) does not take an open profile with it.

import type { Attachment } from "svelte/attachments";

import type { PublicUser, Snowflake, UserProfile } from "../lib/protocol";
import { HoverCardController } from "./hover-card-controller";
import type { HoverCardPlacement } from "./hover-card-placement";
import { durationTokenInMilliseconds } from "./tokens";

export interface ProfileHoverTarget {
  readonly user: PublicUser;
  /** The name or row the card belongs to; its position follows this element. */
  readonly anchor: HTMLElement;
  readonly placement: HoverCardPlacement;
}

export interface ProfileModalRequest {
  readonly user: PublicUser;
  /** Focus goes back to it when the modal closes; null when it has gone. */
  readonly opener: HTMLElement | null;
}

const surfaces = $state<{ hoverCard: ProfileHoverTarget | null; modal: ProfileModalRequest | null }>({
  hoverCard: null,
  modal: null,
});

/**
 * Esc closes the card wherever focus is, even when the pointer opened it (WCAG 1.4.13), and only
 * the card: the side sheet, a drawer or the channel never see that key.
 */
function closeHoverCardOnEscape(event: KeyboardEvent): void {
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    hoverCardController.close();
  }
}

const hoverCardController = new HoverCardController<ProfileHoverTarget>({
  openDelayMilliseconds: () => durationTokenInMilliseconds("component-profile-hover-card-open-delay"),
  closeDelayMilliseconds: () => durationTokenInMilliseconds("component-profile-hover-card-close-delay"),
  isSameTrigger: (left, right) => left.anchor === right.anchor,
  isStillThere: (target) => target.anchor.isConnected,
  onchange: (shown) => {
    const wasShown = surfaces.hoverCard !== null;
    surfaces.hoverCard = shown;
    // Listening only while a card shows: the rest of the time Esc belongs to everyone else.
    if (shown !== null && !wasShown) {
      window.addEventListener("keydown", closeHoverCardOnEscape, { capture: true });
    } else if (shown === null && wasShown) {
      window.removeEventListener("keydown", closeHoverCardOnEscape, { capture: true });
    }
  },
});

export function shownProfileHoverCard(): ProfileHoverTarget | null {
  return surfaces.hoverCard;
}

/** The pointer on the card itself keeps it, as on its trigger. */
export const profileHoverCardPointer = {
  enter: () => hoverCardController.enterCard(),
  leave: () => hoverCardController.leaveCard(),
};

export function closeProfileHoverCard(): void {
  hoverCardController.close();
}

export function openProfileModal(user: PublicUser, opener: HTMLElement | null): void {
  hoverCardController.close();
  surfaces.modal = { user, opener };
}

export function closeProfileModal(): void {
  surfaces.modal = null;
}

export function profileModalRequest(): ProfileModalRequest | null {
  return surfaces.modal;
}

/** Focus given back by the modal as it closes: its row must not open the hover card again. */
let isReturningFocus = false;

/**
 * Gives focus back to what opened the modal. A name that cannot take focus (an author, a mention
 * chip) hands it to the nearest focusable element around it, the message, as menus do.
 */
export function returnFocusWithoutHoverCard(opener: HTMLElement, options: FocusOptions): void {
  isReturningFocus = true;
  try {
    opener.focus(options);
    if (document.activeElement !== opener) {
      opener.parentElement?.closest<HTMLElement>("[tabindex]")?.focus(options);
    }
  } finally {
    isReturningFocus = false;
  }
}

/** A mouse or a pen can rest on a name; a finger cannot, so touch never opens a card. */
function canHover(event: PointerEvent): boolean {
  return event.pointerType === "mouse" || event.pointerType === "pen";
}

export interface ProfileTriggerOptions {
  placement: HoverCardPlacement;
  /** For names that are not buttons (an author, a mention chip): a click opens the modal. */
  opensProfileOnClick?: boolean;
}

// shortcut: each author name, author avatar and mention chip gets its own six listeners (seven with the click) and an
// attachment effect, so a message list costs O(rows) of them. One delegated listener per list would
// need a way back from an element to its `PublicUser` across the message list, search results and
// pins (an id-to-user registry); do it if a profile of long lists shows the attachments.
/**
 * Makes `element` a profile trigger: the hover card after the pointer or keyboard focus rests on
 * it, closed at once by `Esc` (before a drawer or the side sheet sees the key) or a press.
 */
export function profileTrigger(user: PublicUser, options: ProfileTriggerOptions): Attachment<HTMLElement> {
  return (element) => {
    const target: ProfileHoverTarget = { user, anchor: element, placement: options.placement };

    const onPointerEnter = (event: PointerEvent) => {
      if (canHover(event)) {
        hoverCardController.enterTrigger(target);
      }
    };
    const onPointerLeave = (event: PointerEvent) => {
      if (canHover(event)) {
        hoverCardController.leaveTrigger();
      }
    };
    // Only keyboard focus: a click focuses the row too, and opens the modal instead.
    const onFocusIn = (event: FocusEvent) => {
      if (!isReturningFocus && event.target instanceof Element && event.target.matches(":focus-visible")) {
        hoverCardController.enterTrigger(target);
      }
    };
    const onFocusOut = () => hoverCardController.leaveTrigger();
    const onPress = () => hoverCardController.close();
    const onClick = (event: MouseEvent) => {
      event.stopPropagation();
      openProfileModal(user, element);
    };

    element.addEventListener("pointerenter", onPointerEnter);
    element.addEventListener("pointerleave", onPointerLeave);
    element.addEventListener("focusin", onFocusIn);
    element.addEventListener("focusout", onFocusOut);
    element.addEventListener("pointerdown", onPress);
    element.addEventListener("contextmenu", onPress);
    if (options.opensProfileOnClick) {
      element.addEventListener("click", onClick);
    }
    return () => {
      element.removeEventListener("pointerenter", onPointerEnter);
      element.removeEventListener("pointerleave", onPointerLeave);
      element.removeEventListener("focusin", onFocusIn);
      element.removeEventListener("focusout", onFocusOut);
      element.removeEventListener("pointerdown", onPress);
      element.removeEventListener("contextmenu", onPress);
      element.removeEventListener("click", onClick);
      // A row leaving drops the wait for its card and takes a shown card along, once the update
      // that removed it is over (state cannot change while Svelte tears a block down). An
      // attachment run again on the same row (a new user object) keeps both.
      queueMicrotask(() => {
        if (!element.isConnected && hoverCardController.dropTrigger(target)) {
          hoverCardController.close();
        }
      });
    };
  };
}

// About me and links, shared by the hover card and the modal: hovering a name then opening the
// profile reads them once. Kept a minute, since no gateway event says they changed (ADR-0050), and
// only for the account that read them: signing in as someone else starts afresh.
const RECENT_PROFILE_MILLISECONDS = 60_000;
const MAXIMUM_RECENT_PROFILES = 50;
const recentProfiles = new Map<Snowflake, { readAt: number; profile: Promise<UserProfile> }>();
let recentProfilesReaderId: Snowflake | null = null;

/**
 * `load` unless `readerId` read the same profile less than a minute ago; the reader's own profile
 * is always read again (they may just have edited it).
 */
export function readRecentUserProfile(
  load: (userId: Snowflake) => Promise<UserProfile>,
  userId: Snowflake,
  readerId: Snowflake | null,
): Promise<UserProfile> {
  if (readerId !== recentProfilesReaderId) {
    recentProfiles.clear();
    recentProfilesReaderId = readerId;
  }
  const now = Date.now();
  const recent = recentProfiles.get(userId);
  if (userId !== readerId && recent && now - recent.readAt < RECENT_PROFILE_MILLISECONDS) {
    return recent.profile;
  }
  const profile = load(userId);
  recentProfiles.delete(userId);
  recentProfiles.set(userId, { readAt: now, profile });
  // A failure is not remembered: the next look asks again.
  profile.catch(() => {
    if (recentProfiles.get(userId)?.profile === profile) {
      recentProfiles.delete(userId);
    }
  });
  if (recentProfiles.size > MAXIMUM_RECENT_PROFILES) {
    const oldest = recentProfiles.keys().next().value;
    if (oldest !== undefined) {
      recentProfiles.delete(oldest);
    }
  }
  return profile;
}

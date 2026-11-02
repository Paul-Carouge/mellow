// When animated pictures (avatars, server icons, custom emoji, banners) and Melli play
// (docs/design/web-animation-setting.md, DESIGN.md §2.8): the reader's choice for this browser,
// checked on the way in, and the one decision every picture asks. Pure: storage is passed in.

/** "on-hover": avatars and server icons play while pointed at or focused, as on Discord; "always": everything plays. */
export type PictureAnimationChoice = "on-hover" | "always";
export const PICTURE_ANIMATION_CHOICES: readonly PictureAnimationChoice[] = ["on-hover", "always"];
export const DEFAULT_PICTURE_ANIMATION_CHOICE: PictureAnimationChoice = "on-hover";

const PICTURE_ANIMATION_STORAGE_KEY = "mellow.picture-animation";

/**
 * Whether a picture plays. `isPlayingWithoutChoice` is what the picture does by itself: true for
 * those that always play (emoji, banners, the profile card, Melli), hover or focus for avatars and
 * server icons. "Always animate" is the reader's own decision, so it wins over the system's reduced
 * motion; otherwise reduced motion keeps every picture still, hovered or not.
 */
export function shouldPictureAnimate(
  choice: PictureAnimationChoice,
  isReducedMotionPreferred: boolean,
  isPlayingWithoutChoice: boolean,
): boolean {
  return choice === "always" || (!isReducedMotionPreferred && isPlayingWithoutChoice);
}

/** The choice this browser remembers; the default when there is none or it is not one we know. */
export function readPictureAnimationChoice(storage: Pick<Storage, "getItem"> | null): PictureAnimationChoice {
  let stored: string | null = null;
  try {
    stored = storage?.getItem(PICTURE_ANIMATION_STORAGE_KEY) ?? null;
  } catch (error) {
    console.warn("unreadable picture animation choice", error);
  }
  return PICTURE_ANIMATION_CHOICES.find((choice) => choice === stored) ?? DEFAULT_PICTURE_ANIMATION_CHOICE;
}

/** Stores the choice when the browser lets us; a full or blocked storage keeps it for this tab only. */
export function writePictureAnimationChoice(
  storage: Pick<Storage, "setItem"> | null,
  choice: PictureAnimationChoice,
): void {
  try {
    storage?.setItem(PICTURE_ANIMATION_STORAGE_KEY, choice);
  } catch (error) {
    console.warn("couldn't store the picture animation choice", error);
  }
}

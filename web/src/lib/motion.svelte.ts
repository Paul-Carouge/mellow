import { MediaQuery } from "svelte/reactivity";

import {
  type PictureAnimationChoice,
  readPictureAnimationChoice,
  shouldPictureAnimate,
  writePictureAnimationChoice,
} from "./picture-animation";
import { browserStorage } from "./reactions";

// UI motion (sheets, menus, scrolling) follows the system's reduced motion; animated pictures and
// Melli follow the reader's choice too (DESIGN.md §2.8), read here once for every component.
export const prefersReducedMotion = new MediaQuery("prefers-reduced-motion: reduce");

const pictureAnimation = $state<{ choice: PictureAnimationChoice }>({
  choice: readPictureAnimationChoice(browserStorage()),
});

export function pictureAnimationChoice(): PictureAnimationChoice {
  return pictureAnimation.choice;
}

/** Applies the choice at once in this tab and stores it for the next visits. */
export function choosePictureAnimation(choice: PictureAnimationChoice): void {
  pictureAnimation.choice = choice;
  writePictureAnimationChoice(browserStorage(), choice);
}

/** Whether an animated picture or Melli plays now; see `shouldPictureAnimate`. */
export function isPictureAnimationWanted(isPlayingWithoutChoice: boolean): boolean {
  return shouldPictureAnimate(pictureAnimation.choice, prefersReducedMotion.current, isPlayingWithoutChoice);
}

// The GIF picker's rules (docs/design/web-gif-picker.md), apart from its markup: the search's
// pause, the guard against late answers, arrow keys over justified rows, the favourites' order and
// a tile's shape.

import type { KlipyGif, KlipyItem } from "../lib/klipy";

/** Typing pauses this long before Klipy is asked: one request per word, not per letter. */
export const GIF_SEARCH_DEBOUNCE_MILLISECONDS = 300;

/**
 * Only the latest request may change the picker: each `start` makes earlier ones stale, so an
 * answer for "ca" arriving after the one for "cat" is dropped.
 */
export function createLatestRequestGuard(): { start: () => number; isLatest: (request: number) => boolean } {
  let latest = 0;
  return {
    start: () => {
      latest += 1;
      return latest;
    },
    isLatest: (request) => request === latest,
  };
}

/** Runs `action` once calls have paused for `delayMilliseconds`; `flush` runs it now, `cancel` drops it. */
export function debounce(
  action: () => void,
  delayMilliseconds: number,
): { call: () => void; flush: () => void; cancel: () => void } {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const cancel = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  };
  return {
    call: () => {
      cancel();
      timer = setTimeout(() => {
        timer = null;
        action();
      }, delayMilliseconds);
    },
    flush: () => {
      cancel();
      action();
    },
    cancel,
  };
}

export interface TileBounds {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

/**
 * Where an arrow key goes from tile `current`, among tiles laid out in justified rows in Klipy's
 * order: Left and Right step through that order, Up and Down reach the row above or below, the
 * tile whose middle is nearest; Home and End the first and last. `"search"` is Up from the first
 * row; null when the key does nothing.
 */
export function nextGifTile(bounds: readonly TileBounds[], current: number, key: string): number | "search" | null {
  const last = bounds.length - 1;
  const here = bounds[current];
  if (!here) {
    return null;
  }
  switch (key) {
    case "ArrowLeft":
      return current > 0 ? current - 1 : null;
    case "ArrowRight":
      return current < last ? current + 1 : null;
    case "Home":
      return 0;
    case "End":
      return last;
    case "ArrowUp":
    case "ArrowDown":
      break;
    default:
      return null;
  }
  const isUp = key === "ArrowUp";
  const middle = here.left + here.width / 2;
  // Rows are told apart by their top; a tile is in another row when it starts half a row away.
  const rowTops = bounds
    .map((tile) => tile.top)
    .filter((top) => (isUp ? top < here.top - here.height / 2 : top > here.top + here.height / 2));
  if (rowTops.length === 0) {
    return isUp ? "search" : null;
  }
  const targetTop = isUp ? Math.max(...rowTops) : Math.min(...rowTops);
  let best: number | null = null;
  let bestDistance = Number.POSITIVE_INFINITY;
  bounds.forEach((tile, index) => {
    if (Math.abs(tile.top - targetTop) > here.height / 2) {
      return;
    }
    const distance = Math.abs(tile.left + tile.width / 2 - middle);
    if (distance < bestDistance) {
      best = index;
      bestDistance = distance;
    }
  });
  return best;
}

/**
 * Favourites come back from Klipy in its own order; the picker shows them newest starred first,
 * as the instance lists them. A slug Klipy no longer knows is left out.
 */
export function gifsInSlugOrder(items: readonly KlipyItem[], slugs: readonly string[]): KlipyGif[] {
  const bySlug = new Map(items.flatMap((item) => (item.kind === "gif" ? [[item.slug, item] as const] : [])));
  return slugs.flatMap((slug) => {
    const gif = bySlug.get(slug);
    return gif ? [gif] : [];
  });
}

/** The starred slugs after a star or an unstar: a new star goes first, as the instance lists them. */
export function withFavouriteSet(slugs: readonly string[], slug: string, isStarred: boolean): string[] {
  const others = slugs.filter((candidate) => candidate !== slug);
  return isStarred ? [slug, ...others] : others;
}

/** Wider or narrower than this, a tile is cropped rather than made a sliver or a whole row. */
const MINIMUM_TILE_ASPECT_RATIO = 0.5;
const MAXIMUM_TILE_ASPECT_RATIO = 2.5;
/** Klipy's ads have no size of their own here: they take a square. */
const AD_ASPECT_RATIO = 1;

/** Width over height of a tile in the justified rows. */
export function tileAspectRatio(item: KlipyItem): number {
  const width = item.kind === "gif" ? item.tile.width : item.width;
  const height = item.kind === "gif" ? item.tile.height : item.height;
  const ratio = width && height ? width / height : AD_ASPECT_RATIO;
  return Math.min(Math.max(ratio, MINIMUM_TILE_ASPECT_RATIO), MAXIMUM_TILE_ASPECT_RATIO);
}

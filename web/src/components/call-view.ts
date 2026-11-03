// What the call view shows (ADR-0035, docs/design/web-cameras.md): which people get a tile and in
// which order, how large each tile's picture is asked for, the grid's shape, and who the speaker
// view puts large.

import type { Snowflake, VideoTileSize } from "../lib/protocol";
import type { WatchedCamera } from "../lib/voice-session.svelte";

export type CallLayout = "grid" | "speaker";

/** What a person sends: a camera, a shared screen (ADR-0037: never both), or nothing. */
export type CallVideo = "camera" | "screen" | null;

export interface CallPerson {
  readonly userId: Snowflake;
  readonly video: CallVideo;
}

/** Nine tiles at most, as many as the video receive slots (ADR-0035: a 3 × 3 grid). */
export const MAXIMUM_CALL_TILES = 9;

/** The 16:9 of cameras and of most screens. */
const TILE_ASPECT_RATIO = 16 / 9;

/**
 * People with a picture first, then the others, each in join order; the first `maximum` get a
 * tile and the rest show as avatars under them. In the speaker view the featured person comes first.
 */
export function arrangeCallTiles(
  people: readonly CallPerson[],
  featuredUserId: Snowflake | null,
  maximum = MAXIMUM_CALL_TILES,
): { tiles: CallPerson[]; overflow: CallPerson[] } {
  const featured = people.filter((person) => person.userId === featuredUserId);
  const rest = people.filter((person) => person.userId !== featuredUserId);
  const ordered = [...featured, ...rest.filter((person) => person.video !== null), ...rest.filter((person) => person.video === null)];
  return { tiles: ordered.slice(0, maximum), overflow: ordered.slice(maximum) };
}

/** ADR-0035: a grid of 1 is large, 2 to 4 medium, 5 to 9 small; the speaker view's large tile is large, the strip small. */
export function tileSizeOf(layout: CallLayout, tileCount: number, isFeatured: boolean): VideoTileSize {
  if (layout === "speaker") {
    return isFeatured ? "large" : "small";
  }
  return tileCount <= 1 ? "large" : tileCount <= 4 ? "medium" : "small";
}

/** The pictures to ask the instance for: everyone with a camera or a screen on a tile, you aside (your own picture is local). */
export function watchedCamerasOf(
  tiles: readonly CallPerson[],
  layout: CallLayout,
  ownUserId: Snowflake | null,
): WatchedCamera[] {
  return tiles.flatMap((person, index) =>
    person.video === null || person.userId === ownUserId
      ? []
      : [{ userId: person.userId, size: tileSizeOf(layout, tiles.length, layout === "speaker" && index === 0) }],
  );
}

/** The column count that gives `count` 16:9 tiles the largest size in `width` × `height`, and that size. */
export function gridShapeFor(count: number, width: number, height: number, gap: number): { columns: number; tileWidth: number } {
  let best = { columns: 1, tileWidth: 0 };
  for (let columns = 1; columns <= Math.max(1, count); columns += 1) {
    const rows = Math.ceil(count / columns);
    const widthPerTile = (width - gap * (columns - 1)) / columns;
    const heightPerTile = (height - gap * (rows - 1)) / rows;
    const tileWidth = Math.floor(Math.max(0, Math.min(widthPerTile, heightPerTile * TILE_ASPECT_RATIO)));
    if (tileWidth > best.tileWidth) {
      best = { columns, tileWidth };
    }
  }
  return best;
}

/**
 * Who the speaker view puts large when nobody is picked: whoever is talking (someone else than you,
 * as on Discord), staying on the same person while they still talk, and on the last one through
 * silences. `null` when that person left.
 */
export function nextSpeaker(
  current: Snowflake | null,
  speakingUserIds: ReadonlySet<Snowflake>,
  ownUserId: Snowflake | null,
  presentUserIds: readonly Snowflake[],
): Snowflake | null {
  if (current !== null && speakingUserIds.has(current) && presentUserIds.includes(current)) {
    return current;
  }
  const speaker = presentUserIds.find((userId) => userId !== ownUserId && speakingUserIds.has(userId));
  if (speaker !== undefined) {
    return speaker;
  }
  return current !== null && presentUserIds.includes(current) ? current : null;
}

/** The person the speaker view puts large: the one picked, else the last speaker, else the first picture, else the first person. */
export function featuredPersonOf(
  people: readonly CallPerson[],
  pickedUserId: Snowflake | null,
  speakerUserId: Snowflake | null,
  ownUserId: Snowflake | null,
): Snowflake | null {
  const isPresent = (userId: Snowflake | null) => userId !== null && people.some((person) => person.userId === userId);
  if (isPresent(pickedUserId)) {
    return pickedUserId;
  }
  if (isPresent(speakerUserId)) {
    return speakerUserId;
  }
  const others = people.filter((person) => person.userId !== ownUserId);
  return (others.find((person) => person.video !== null) ?? others[0] ?? people[0])?.userId ?? null;
}

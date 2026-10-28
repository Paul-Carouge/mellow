// Moving channels and categories in the sidebar (DESIGN.md §5.3, docs/design/web-channels-and-
// server-creation.md): pure rules over the sidebar's sections, so the drag, the menus and the
// pending order share one model. The instance decides (ADR-0007); these rules only build the
// whole new order it wants (`PUT /guilds/{id}/channels/order`) and hide moves it would refuse.

import type { ChannelSection } from "../lib/channels";
import type { Channel, ChannelPlacement, Snowflake } from "../lib/protocol";

/** Where a channel goes: a category (`null`: none) and its index there, as the list shows it. */
export interface ChannelDestination {
  readonly categoryId: Snowflake | null;
  readonly index: number;
}

/**
 * Every listed channel once, in the sidebar's order: each category, then its channels. An
 * uncategorised channel sends its own `parent_id`, which is a hidden category's for a channel the
 * reader sees in a category they do not (the instance keeps it there).
 */
export function placementsOf(sections: readonly ChannelSection[]): ChannelPlacement[] {
  return sections.flatMap((section) => [
    ...(section.category ? [{ id: section.category.id, parent_id: null }] : []),
    ...section.channels.map((channel) => ({ id: channel.id, parent_id: section.category?.id ?? channel.parent_id })),
  ]);
}

export function isSameOrder(left: readonly ChannelPlacement[], right: readonly ChannelPlacement[]): boolean {
  return (
    left.length === right.length &&
    left.every((placement, index) => placement.id === right[index]?.id && placement.parent_id === right[index]?.parent_id)
  );
}

/**
 * The sections a pending order shows, with the channels as they are now (a rename meanwhile
 * shows). `null` once the channels and the order no longer list the same ids (one was created or
 * deleted meanwhile): the instance's order shows instead.
 */
export function sectionsFromPlacements(
  channels: readonly Channel[],
  placements: readonly ChannelPlacement[],
): ChannelSection[] | null {
  const byId = new Map(channels.map((channel) => [channel.id, channel]));
  if (byId.size !== placements.length || placements.some((placement) => !byId.has(placement.id))) {
    return null;
  }
  const uncategorised: ChannelSection = { category: null, channels: [] };
  const categorySections = new Map<Snowflake, ChannelSection>();
  for (const placement of placements) {
    const channel = byId.get(placement.id);
    if (channel?.kind === "category") {
      categorySections.set(channel.id, { category: channel, channels: [] });
    }
  }
  for (const placement of placements) {
    const channel = byId.get(placement.id);
    if (channel === undefined || channel.kind === "category") {
      continue;
    }
    const section = (placement.parent_id !== null && categorySections.get(placement.parent_id)) || uncategorised;
    section.channels.push({ ...channel, parent_id: placement.parent_id });
  }
  return [...(uncategorised.channels.length > 0 ? [uncategorised] : []), ...categorySections.values()];
}

function sectionOfChannel(sections: readonly ChannelSection[], channelId: Snowflake): ChannelSection | undefined {
  return sections.find((section) => section.channels.some((channel) => channel.id === channelId));
}

/** The category the channel is listed under; `undefined` when it is not listed. */
export function listedCategoryOf(sections: readonly ChannelSection[], channelId: Snowflake): Snowflake | null | undefined {
  const section = sectionOfChannel(sections, channelId);
  return section === undefined ? undefined : (section.category?.id ?? null);
}

/**
 * The sections with the channel moved to `destination`, or `null` when nothing would change (or
 * either end is gone). The index counts the rows as listed, the moved one included, so a drop
 * below a row of the same section lands where the line was drawn.
 */
export function withChannelMoved(
  sections: readonly ChannelSection[],
  channelId: Snowflake,
  destination: ChannelDestination,
): ChannelSection[] | null {
  const source = sectionOfChannel(sections, channelId);
  const channel = source?.channels.find((candidate) => candidate.id === channelId);
  const sourceCategoryId = source?.category?.id ?? null;
  if (!source || !channel) {
    return null;
  }
  if (destination.categoryId !== null && !sections.some((section) => section.category?.id === destination.categoryId)) {
    return null;
  }
  const isSameSection = sourceCategoryId === destination.categoryId;
  const sourceIndex = source.channels.indexOf(channel);
  const index = isSameSection && sourceIndex < destination.index ? destination.index - 1 : destination.index;
  // Out of a category: none. Within the uncategorised rows: its own parent (a hidden category's).
  const parentId = destination.categoryId ?? (isSameSection ? channel.parent_id : null);
  const moved = { ...channel, parent_id: parentId };
  const withoutChannel = sections.map((section) => ({
    category: section.category,
    channels: section.channels.filter((candidate) => candidate.id !== channelId),
  }));
  const hasUncategorised = withoutChannel.some((section) => section.category === null);
  const next =
    destination.categoryId === null && !hasUncategorised
      ? [{ category: null, channels: [] as Channel[] }, ...withoutChannel]
      : withoutChannel;
  const target = next.find((section) => (section.category?.id ?? null) === destination.categoryId);
  if (!target) {
    return null;
  }
  target.channels.splice(Math.max(0, Math.min(index, target.channels.length)), 0, moved);
  const result = next.filter((section) => section.category !== null || section.channels.length > 0);
  return isSameOrder(placementsOf(result), placementsOf(sections)) ? null : result;
}

/** The categories in their listed order. */
function categoriesOf(sections: readonly ChannelSection[]): Channel[] {
  return sections.flatMap((section) => (section.category ? [section.category] : []));
}

/**
 * The sections with the category (and its channels) moved to `index` among the categories, the
 * moved one included as for channels; `null` when nothing would change.
 */
export function withCategoryMoved(
  sections: readonly ChannelSection[],
  categoryId: Snowflake,
  index: number,
): ChannelSection[] | null {
  const categorySections = sections.filter((section) => section.category !== null);
  const sourceIndex = categorySections.findIndex((section) => section.category?.id === categoryId);
  const moving = categorySections[sourceIndex];
  if (moving === undefined) {
    return null;
  }
  const targetIndex = sourceIndex < index ? index - 1 : index;
  const reordered = categorySections.filter((section) => section !== moving);
  reordered.splice(Math.max(0, Math.min(targetIndex, reordered.length)), 0, moving);
  const result = [...sections.filter((section) => section.category === null), ...reordered];
  return isSameOrder(placementsOf(result), placementsOf(sections)) ? null : result;
}

/** "Move up" and "Move down" of a channel: within its section, `null` at its edge. */
export function channelStepDestination(
  sections: readonly ChannelSection[],
  channelId: Snowflake,
  step: -1 | 1,
): ChannelDestination | null {
  const section = sectionOfChannel(sections, channelId);
  const index = section?.channels.findIndex((channel) => channel.id === channelId) ?? -1;
  if (!section || index < 0) {
    return null;
  }
  const target = index + step;
  if (target < 0 || target >= section.channels.length) {
    return null;
  }
  // As listed, the moved row included: one down is past the next row.
  return { categoryId: section.category?.id ?? null, index: step === 1 ? target + 1 : target };
}

/** "Move up" and "Move down" of a category, as an index for `withCategoryMoved`; `null` at an edge. */
export function categoryStepIndex(sections: readonly ChannelSection[], categoryId: Snowflake, step: -1 | 1): number | null {
  const categories = categoriesOf(sections);
  const index = categories.findIndex((category) => category.id === categoryId);
  const target = index + step;
  if (index < 0 || target < 0 || target >= categories.length) {
    return null;
  }
  return step === 1 ? target + 1 : target;
}

// --- Dropping ----------------------------------------------------------------------------------

/** A row or category header as laid out, top to bottom, in one vertical coordinate system. */
export interface ListedBox {
  readonly kind: "category" | "channel";
  readonly id: Snowflake;
  readonly top: number;
  readonly bottom: number;
}

export type ChannelDrop =
  | { readonly destination: ChannelDestination; readonly indicator: { readonly kind: "line"; readonly y: number } }
  | { readonly destination: ChannelDestination; readonly indicator: { readonly kind: "category"; readonly id: Snowflake } };

/**
 * Where a carried channel would land with the pointer at `y`: on a category header, into that
 * category at its top (the only way into a collapsed one); otherwise at the nearest gap between
 * rows. Without `mayChangeCategory` only its own section's gaps count. `null` when the drop would
 * change nothing.
 */
export function channelDropAt(
  sections: readonly ChannelSection[],
  boxes: readonly ListedBox[],
  channelId: Snowflake,
  y: number,
  mayChangeCategory: boolean,
): ChannelDrop | null {
  const ownCategoryId = listedCategoryOf(sections, channelId);
  if (ownCategoryId === undefined) {
    return null;
  }
  const isAllowed = (categoryId: Snowflake | null) => mayChangeCategory || categoryId === ownCategoryId;
  const resultFor = (drop: ChannelDrop) =>
    withChannelMoved(sections, channelId, drop.destination) === null ? null : drop;

  const header = boxes.find((box) => box.kind === "category" && y >= box.top && y <= box.bottom);
  if (header && isAllowed(header.id)) {
    return resultFor({ destination: { categoryId: header.id, index: 0 }, indicator: { kind: "category", id: header.id } });
  }

  const gaps: { y: number; destination: ChannelDestination }[] = [];
  const firstBox = boxes[0];
  if (!sections.some((section) => section.category === null) && firstBox) {
    gaps.push({ y: firstBox.top, destination: { categoryId: null, index: 0 } });
  }
  for (const box of boxes) {
    if (box.kind !== "channel") {
      continue;
    }
    const section = sectionOfChannel(sections, box.id);
    const index = section?.channels.findIndex((channel) => channel.id === box.id) ?? -1;
    if (!section || index < 0) {
      continue;
    }
    const categoryId = section.category?.id ?? null;
    gaps.push({ y: box.top, destination: { categoryId, index } }, { y: box.bottom, destination: { categoryId, index: index + 1 } });
  }
  // An expanded category with no channel yet: just under its header.
  for (const box of boxes) {
    const section = sections.find((candidate) => candidate.category?.id === box.id);
    if (box.kind === "category" && section && section.channels.length === 0) {
      gaps.push({ y: box.bottom, destination: { categoryId: box.id, index: 0 } });
    }
  }
  const nearest = gaps
    .filter((gap) => isAllowed(gap.destination.categoryId))
    .reduce<(typeof gaps)[number] | null>(
      (best, gap) => (best === null || Math.abs(gap.y - y) < Math.abs(best.y - y) ? gap : best),
      null,
    );
  return nearest ? resultFor({ destination: nearest.destination, indicator: { kind: "line", y: nearest.y } }) : null;
}

/**
 * Where a carried category would land: before the nearest category header, or after the last
 * row. `null` when that changes nothing.
 */
export function categoryDropAt(
  sections: readonly ChannelSection[],
  boxes: readonly ListedBox[],
  categoryId: Snowflake,
  y: number,
): { readonly index: number; readonly y: number } | null {
  const headers = boxes.filter((box) => box.kind === "category");
  const lastBottom = boxes.reduce((bottom, box) => Math.max(bottom, box.bottom), Number.NEGATIVE_INFINITY);
  const gaps = [
    ...headers.map((header) => ({
      y: header.top,
      index: categoriesOf(sections).findIndex((category) => category.id === header.id),
    })),
    ...(headers.length > 0 ? [{ y: lastBottom, index: categoriesOf(sections).length }] : []),
  ].filter((gap) => gap.index >= 0);
  const nearest = gaps.reduce<(typeof gaps)[number] | null>(
    (best, gap) => (best === null || Math.abs(gap.y - y) < Math.abs(best.y - y) ? gap : best),
    null,
  );
  return nearest && withCategoryMoved(sections, categoryId, nearest.index) !== null ? nearest : null;
}

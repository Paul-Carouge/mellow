// The channel order of the open server as a manager changes it (DESIGN.md §5.3): the menus' and
// the drag's moves go through here, and the order sent but not yet echoed by the gateway shows
// meanwhile, so a drop does not snap back.

import { strings } from "../i18n";
import { type ChannelSection, channelSections } from "../lib/channels";
import { type MellowClient, describeError } from "../lib/client.svelte";
import type { Channel, ChannelPlacement, Snowflake } from "../lib/protocol";
import {
  type ChannelDestination,
  categoryStepIndex,
  channelStepDestination,
  isPendingOrderSettled,
  placementsOf,
  sectionsFromPlacements,
  withCategoryMoved,
  withChannelMoved,
} from "./channel-order";
import type { ContextMenuOutcome } from "./ContextMenu.svelte";

/**
 * How long an agreed order waits for the gateway's `GUILD_CREATE` before the instance's order
 * shows again: a fallback for an echo that never comes (a dropped connection resyncs anyway).
 */
export const PENDING_ORDER_ECHO_TIMEOUT_MILLISECONDS = 5_000;

export type ChannelReorderClient = Pick<MellowClient, "selectedGuild" | "guilds" | "reorderChannels">;

interface PendingOrder {
  readonly guildId: Snowflake;
  readonly placements: readonly ChannelPlacement[];
  /** The guild's channels when the instance agreed: the next snapshot replaces them. */
  readonly answeredChannels: readonly Channel[] | null;
}

/**
 * Create it while a component initialises: it watches for the echo for as long as that lives. The
 * client is read through a getter, so a new one is followed.
 */
export class ChannelReorder {
  readonly #client: () => ChannelReorderClient;
  readonly #announce: (sentence: string) => void;
  /** The order sent and not yet echoed. */
  #pendingOrder = $state.raw<PendingOrder | null>(null);
  readonly #instanceSections = $derived.by(() => {
    const guild = this.#client().selectedGuild;
    return guild ? channelSections(guild) : [];
  });

  /** A request is in flight: no other move starts until it is answered. */
  isReordering = $state(false);
  /** Why a drop was refused: the old order is back, and this says why until cleared or the next move. */
  error = $state("");
  /** The open server's channels by category, the pending order applied. */
  readonly sections = $derived.by((): ChannelSection[] => {
    const guild = this.#client().selectedGuild;
    const pending = this.#pendingOrder;
    if (!guild || pending === null || pending.guildId !== guild.id || this.#isSettled(pending, guild.channels)) {
      return this.#instanceSections;
    }
    return sectionsFromPlacements(guild.channels, pending.placements) ?? this.#instanceSections;
  });

  /** `announce` speaks a move the drag or "Move to category…" made; the menus announce their own. */
  constructor(client: () => ChannelReorderClient, announce: (sentence: string) => void) {
    this.#client = client;
    this.#announce = announce;
    // A settled order already stops showing; forgetting it keeps a later instance order from
    // bringing it back.
    $effect(() => {
      const pending = this.#pendingOrder;
      const guild = this.#client().selectedGuild;
      if (pending !== null && guild?.id === pending.guildId && this.#isSettled(pending, guild.channels)) {
        this.#pendingOrder = null;
      }
    });
  }

  #isSettled(pending: PendingOrder, channelsNow: readonly Channel[]): boolean {
    return isPendingOrderSettled(pending.placements, pending.answeredChannels, placementsOf(this.#instanceSections), channelsNow);
  }

  /** "Move up" and "Move down" of a channel's menu. */
  moveChannelStep(channel: Channel, step: -1 | 1): Promise<ContextMenuOutcome> {
    const destination = channelStepDestination(this.sections, channel.id, step);
    return this.#menuOutcome(
      destination && withChannelMoved(this.sections, channel.id, destination),
      strings.channelOrder.channelMoveFailed,
      step === -1 ? strings.channelOrder.channelMovedUp(channel.name) : strings.channelOrder.channelMovedDown(channel.name),
    );
  }

  /** "Move up" and "Move down" of a category's menu. */
  moveCategoryStep(category: Channel, step: -1 | 1): Promise<ContextMenuOutcome> {
    const index = categoryStepIndex(this.sections, category.id, step);
    return this.#menuOutcome(
      index === null ? null : withCategoryMoved(this.sections, category.id, index),
      strings.channelOrder.categoryMoveFailed,
      step === -1 ? strings.channelOrder.categoryMovedUp(category.name) : strings.channelOrder.categoryMovedDown(category.name),
    );
  }

  /** "Move to category…": to the end of the one chosen; resolves to the error to show, or null. */
  async moveChannelTo(channel: Channel, categoryId: Snowflake | null): Promise<string | null> {
    const guild = this.#client().selectedGuild;
    const section = this.sections.find((candidate) => (candidate.category?.id ?? null) === categoryId);
    const next = withChannelMoved(this.sections, channel.id, {
      categoryId,
      index: section?.channels.length ?? 0,
      leavesHiddenCategory: true,
    });
    if (!guild || next === null) {
      return null;
    }
    const sentence = this.#channelMovedSentence(channel, categoryId);
    const problem = await this.#apply(guild.id, next, strings.channelOrder.channelMoveFailed);
    if (problem === null) {
      this.#announce(sentence);
    }
    return problem;
  }

  /** A dragged channel dropped; a refusal shows in `error`. */
  async dropChannel(channelId: Snowflake, destination: ChannelDestination): Promise<void> {
    const channel = this.sections.flatMap((section) => section.channels).find((candidate) => candidate.id === channelId);
    await this.#drop(
      withChannelMoved(this.sections, channelId, destination),
      channel ? this.#channelMovedSentence(channel, destination.categoryId) : "",
      strings.channelOrder.channelMoveFailed,
    );
  }

  /** A dragged category dropped at `index` among the categories; a refusal shows in `error`. */
  async dropCategory(category: Pick<Channel, "id" | "name">, index: number): Promise<void> {
    await this.#drop(
      withCategoryMoved(this.sections, category.id, index),
      strings.channelOrder.categoryMoved(category.name),
      strings.channelOrder.categoryMoveFailed,
    );
  }

  async #drop(next: ChannelSection[] | null, sentence: string, failureSummary: string): Promise<void> {
    const guild = this.#client().selectedGuild;
    if (!guild || next === null) {
      return;
    }
    const problem = await this.#apply(guild.id, next, failureSummary);
    if (problem === null) {
      this.#announce(sentence);
    } else {
      this.error = problem;
    }
  }

  async #menuOutcome(next: ChannelSection[] | null, failureSummary: string, done: string): Promise<ContextMenuOutcome> {
    const guild = this.#client().selectedGuild;
    if (!guild || next === null) {
      return { announcement: "" };
    }
    const problem = await this.#apply(guild.id, next, failureSummary);
    return problem === null ? { announcement: done } : { error: problem };
  }

  #channelMovedSentence(channel: Channel, categoryId: Snowflake | null): string {
    const category = this.sections.find((section) => section.category?.id === categoryId)?.category;
    return category ? strings.channelOrder.channelMovedTo(channel.name, category.name) : strings.channelOrder.channelMovedOut(channel.name);
  }

  /** Sends the whole new order; resolves to the error to show, or null. */
  async #apply(guildId: Snowflake, next: readonly ChannelSection[], failureSummary: string): Promise<string | null> {
    const placements = placementsOf(next);
    const order: PendingOrder = { guildId, placements, answeredChannels: null };
    this.#pendingOrder = order;
    this.isReordering = true;
    this.error = "";
    try {
      await this.#client().reorderChannels(guildId, placements);
      if (this.#pendingOrder === order) {
        const answered: PendingOrder = {
          ...order,
          answeredChannels: this.#client().guilds.find((candidate) => candidate.id === guildId)?.channels ?? null,
        };
        this.#pendingOrder = answered;
        setTimeout(() => {
          if (this.#pendingOrder === answered) {
            this.#pendingOrder = null;
          }
        }, PENDING_ORDER_ECHO_TIMEOUT_MILLISECONDS);
      }
      return null;
    } catch (error) {
      if (this.#pendingOrder === order) {
        this.#pendingOrder = null;
      }
      return describeError(error, failureSummary);
    } finally {
      this.isReordering = false;
    }
  }
}

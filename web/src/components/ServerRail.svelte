<script lang="ts">
  import type { Snippet } from "svelte";

  import { strings } from "../i18n";
  import { SvelteSet } from "svelte/reactivity";

  import type { MellowClient } from "../lib/client.svelte";
  import { prefersReducedMotion } from "../lib/motion.svelte";
  import { type Snowflake, iconUrl } from "../lib/protocol";
  import { serverInitials } from "./format";
  import Icon from "./Icon.svelte";
  import Melli from "./Melli.svelte";
  import { homeMelliState } from "./melli";

  // The server strip of the navigator card (DESIGN.md §5.2, ADR-0022 §1): Home, a separator, the
  // servers, then "Add a server". Icons morph from circle to squircle on hover, focus and
  // selection; the pill at the card's edge says unread (short), hover (medium) or selected (tall).
  interface Props {
    client: MellowClient;
    onaddserver: () => void;
    /** After a server or Home was chosen (the narrow layout closes its drawer). */
    onnavigate: () => void;
  }

  let { client, onaddserver, onnavigate }: Props = $props();

  // The strip scrolls, which would clip a tooltip drawn inside it: it is fixed to the viewport.
  let tooltip = $state<{ text: string; top: number; left: number } | null>(null);
  /** The server whose animated icon plays: hovered or focused, as avatars do, never under reduced motion. */
  let animatedGuildId = $state<Snowflake | null>(null);
  /** Icons that failed to load show the initials instead. */
  const failedIconIds = new SvelteSet<Snowflake>();

  function showTooltip(event: Event, text: string) {
    // Beside the item's slot, whose trailing edge is the strip's.
    const button = event.currentTarget as HTMLElement;
    const bounds = (button.parentElement ?? button).getBoundingClientRect();
    tooltip = { text, top: bounds.top + bounds.height / 2, left: bounds.right };
  }

  function select(guildId: Snowflake | null) {
    client.selectGuild(guildId, null);
    onnavigate();
  }

  const isHomeSelected = $derived(client.selectedGuildId === null);
  const hasUnreadConversation = $derived(client.conversations.some((conversation) => client.hasUnread(conversation.id)));
</script>

{#snippet railItem(label: string, isSelected: boolean, hasUnread: boolean, onclick: (event: MouseEvent) => void, content: Snippet, kind: "home" | "server" | "action", guildId: Snowflake | null = null)}
  <li class="group relative flex w-full justify-center">
    {#if kind !== "action"}
      <!-- At the card's inner edge, half its width showing (ADR-0022 §1). -->
      <span
        aria-hidden="true"
        class={[
          "rail-pill absolute top-1/2 left-0 w-[calc(var(--mw-component-rail-pill-width)/2)] -translate-y-1/2 rounded-e-full",
          isSelected
            ? "h-(--mw-component-rail-pill-height-selected) bg-(--mw-component-rail-pill-color-selected)"
            : [
                "bg-(--mw-component-rail-pill-color)",
                hasUnread ? "h-(--mw-component-rail-pill-height-unread)" : "h-0",
                "group-hover:h-(--mw-component-rail-pill-height-hover) group-has-[:focus-visible]:h-(--mw-component-rail-pill-height-hover)",
              ],
        ]}
      ></span>
    {/if}
    <button
      type="button"
      aria-label={hasUnread ? strings.common.withUnread(label) : label}
      aria-current={isSelected ? "page" : undefined}
      class={[
        "rail-item relative grid size-(--mw-component-rail-item-size) place-items-center overflow-hidden",
        isSelected
          ? "rounded-(--mw-component-rail-icon-radius-active)"
          : "rounded-(--mw-component-rail-icon-radius-rest) hover:rounded-(--mw-component-rail-icon-radius-active) focus-visible:rounded-(--mw-component-rail-icon-radius-active)",
        kind === "home"
          ? isSelected
            ? "bg-accent-fill [--melli-mark:var(--mw-color-mascot-ink)]"
            : "hover:bg-accent-fill hover:[--melli-mark:var(--mw-color-mascot-ink)] focus-visible:bg-accent-fill focus-visible:[--melli-mark:var(--mw-color-mascot-ink)]"
          : kind === "action"
            ? "bg-(--mw-component-rail-icon-background-rest) text-(--mw-component-rail-add-icon) hover:bg-(--mw-component-rail-add-background-hover) hover:text-(--mw-component-rail-add-icon-hover) focus-visible:bg-(--mw-component-rail-add-background-hover) focus-visible:text-(--mw-component-rail-add-icon-hover)"
            : isSelected
              ? "bg-(--mw-component-rail-icon-background-active) text-(--mw-component-rail-icon-text-active)"
              : "bg-(--mw-component-rail-icon-background-rest) text-(--mw-component-rail-icon-text-rest) hover:bg-(--mw-component-rail-icon-background-active) hover:text-(--mw-component-rail-icon-text-active) focus-visible:bg-(--mw-component-rail-icon-background-active) focus-visible:text-(--mw-component-rail-icon-text-active)",
      ]}
      {onclick}
      onpointerenter={(event) => {
        animatedGuildId = guildId;
        if (event.pointerType === "mouse") {
          showTooltip(event, label);
        }
      }}
      onpointerleave={(event) => {
        tooltip = null;
        if (event.currentTarget !== document.activeElement) {
          animatedGuildId = null;
        }
      }}
      onfocus={(event) => {
        animatedGuildId = guildId;
        // Keyboard focus only: a tap or a click already shows the server it opens.
        if (event.currentTarget.matches(":focus-visible")) {
          showTooltip(event, label);
        }
      }}
      onblur={() => {
        tooltip = null;
        animatedGuildId = null;
      }}
    >
      {@render content()}
    </button>
  </li>
{/snippet}

{#snippet homeMark()}
  <!-- Melli sits on the strip at rest; hover, focus and selection fill the button apricot, which
       morphs like the other icons, and switch Melli's marks to ink (DESIGN.md §2.10). -->
  <Melli state={homeMelliState(client.status, hasUnreadConversation)} sizeToken="--mw-component-rail-item-size" />
{/snippet}

{#snippet addIcon()}
  <Icon name="plus" class="size-[calc(var(--mw-component-rail-item-size)/2)]" />
{/snippet}

<nav aria-label={strings.rail.servers} class="min-h-0 overflow-x-hidden overflow-y-auto [scrollbar-width:none]">
  <ul class="flex flex-col items-center gap-(--mw-component-rail-item-gap) pt-(--mw-component-rail-padding-top)">
    {@render railItem(strings.rail.directMessages, isHomeSelected, hasUnreadConversation, () => select(null), homeMark, "home")}
    <li aria-hidden="true" class="h-(--mw-size-rail-separator-height) w-(--mw-size-rail-separator-width) shrink-0 rounded-full bg-(--mw-component-rail-separator-color)"></li>
    {#each client.guilds as guild (guild.id)}
      {@const hasUnread = guild.channels.some((channel) => client.hasUnread(channel.id))}
      {#snippet serverMark()}
        {#if guild.icon && !failedIconIds.has(guild.icon.id)}
          {@const iconId = guild.icon.id}
          <img
            src={iconUrl(guild.icon, !prefersReducedMotion.current && animatedGuildId === guild.id)}
            alt=""
            draggable="false"
            class="absolute inset-0 size-full object-cover select-none"
            onerror={() => failedIconIds.add(iconId)}
          />
        {:else}
          <span aria-hidden="true" class="type-body font-semibold">{serverInitials(guild.name)}</span>
        {/if}
      {/snippet}
      {@render railItem(guild.name, client.selectedGuildId === guild.id, hasUnread, () => select(guild.id), serverMark, "server", guild.id)}
    {/each}
    {@render railItem(strings.rail.addServer, false, false, onaddserver, addIcon, "action")}
  </ul>
</nav>

{#if tooltip}
  <!-- Strip tooltips appear at once, to the trailing side (DESIGN.md §5.2); decorative copy of the label. -->
  <div
    aria-hidden="true"
    class="rail-tooltip pointer-events-none fixed z-50 max-w-(--mw-component-tooltip-max-width) -translate-y-1/2 rounded-(--mw-component-tooltip-radius) border border-border-divider bg-(--mw-component-tooltip-background) px-(--mw-component-tooltip-padding-inline) py-(--mw-component-tooltip-padding-block) type-tooltip break-words text-(--mw-component-tooltip-text) shadow-(--mw-component-tooltip-shadow)"
    style:top="{tooltip.top}px"
    style:left="calc({tooltip.left}px - var(--mw-space-2xs))"
  >
    {tooltip.text}
  </div>
{/if}

<style>
  /* Three curves (ADR-0022 §3): the squircle settles, colours and the pill change on `standard`. */
  .rail-item {
    transition:
      border-radius var(--mw-motion-duration-settle) var(--mw-motion-easing-settle),
      background-color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard),
      color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard);
  }
  .rail-pill {
    transition:
      height var(--mw-motion-duration-base) var(--mw-motion-easing-standard),
      background-color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard);
  }
  .rail-tooltip {
    animation: mw-nudge-from-start var(--mw-motion-duration-fast) var(--mw-motion-easing-enter) both;
  }
</style>

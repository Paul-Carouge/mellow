<script lang="ts">
  import type { Snippet } from "svelte";

  import type { MellowClient } from "../lib/client.svelte";
  import type { Snowflake } from "../lib/protocol";
  import { serverInitials } from "./format";
  import Icon from "./Icon.svelte";
  import Melli from "./Melli.svelte";
  import { homeMelliState } from "./melli";

  // Discord's server rail (DESIGN.md §5.2): Home, a separator, the servers, then "Add a server".
  // Icons morph from circle to squircle on hover, focus and selection; the pill at the leading
  // edge says unread (short), hover (medium) or selected (tall).
  interface Props {
    client: MellowClient;
    onaddserver: () => void;
    /** After a server or Home was chosen (the narrow layout closes its drawer). */
    onnavigate: () => void;
  }

  let { client, onaddserver, onnavigate }: Props = $props();

  // The rail scrolls, which would clip a tooltip drawn inside it: it is fixed to the viewport.
  let tooltip = $state<{ text: string; top: number } | null>(null);

  function showTooltip(event: Event, text: string) {
    const target = event.currentTarget as HTMLElement;
    const bounds = target.getBoundingClientRect();
    tooltip = { text, top: bounds.top + bounds.height / 2 };
  }

  function select(guildId: Snowflake | null) {
    client.selectGuild(guildId, null);
    onnavigate();
  }

  const isHomeSelected = $derived(client.selectedGuildId === null);
  const hasUnreadConversation = $derived(client.conversations.some((conversation) => client.hasUnread(conversation.id)));
</script>

{#snippet railItem(label: string, isSelected: boolean, hasUnread: boolean, onclick: (event: MouseEvent) => void, content: Snippet, kind: "home" | "server" | "action")}
  <li class="group relative flex h-(--mw-component-rail-item-size) w-full justify-center">
    <span
      aria-hidden="true"
      class={[
        "absolute top-1/2 left-0 w-(--mw-component-rail-pill-width) -translate-x-1/2 -translate-y-1/2 rounded-full bg-(--mw-component-rail-pill-color) transition-[height] duration-(--mw-component-rail-morph-duration) ease-(--mw-motion-easing-standard)",
        isSelected
          ? "h-(--mw-component-rail-pill-height-selected)"
          : [
              hasUnread ? "h-(--mw-component-rail-pill-height-unread)" : "h-0",
              "group-hover:h-(--mw-component-rail-pill-height-hover)",
            ],
      ]}
    ></span>
    <button
      type="button"
      aria-label={hasUnread ? `${label}, unread` : label}
      aria-current={isSelected ? "page" : undefined}
      class={[
        "grid size-(--mw-component-rail-item-size) place-items-center overflow-hidden transition-[border-radius,background-color,color] duration-(--mw-component-rail-morph-duration) ease-(--mw-motion-easing-standard)",
        isSelected
          ? "rounded-(--mw-component-rail-icon-radius-active)"
          : "rounded-(--mw-component-rail-icon-radius-rest) hover:rounded-(--mw-component-rail-icon-radius-active) focus-visible:rounded-(--mw-component-rail-icon-radius-active)",
        kind === "home"
          ? isSelected
            ? "bg-(--mw-component-rail-icon-background-active)"
            : "bg-(--mw-component-rail-home-background-rest) hover:bg-(--mw-component-rail-icon-background-active) focus-visible:bg-(--mw-component-rail-icon-background-active)"
          : kind === "action"
          ? "bg-(--mw-component-rail-icon-background-rest) text-(--mw-component-rail-add-icon) hover:bg-(--mw-component-rail-add-background-hover) hover:text-(--mw-component-rail-add-icon-hover) focus-visible:bg-(--mw-component-rail-add-background-hover) focus-visible:text-(--mw-component-rail-add-icon-hover)"
          : isSelected
            ? "bg-(--mw-component-rail-icon-background-active) text-(--mw-component-rail-icon-text-active)"
            : "bg-(--mw-component-rail-icon-background-rest) text-(--mw-component-rail-icon-text-rest) hover:bg-(--mw-component-rail-icon-background-active) hover:text-(--mw-component-rail-icon-text-active) focus-visible:bg-(--mw-component-rail-icon-background-active) focus-visible:text-(--mw-component-rail-icon-text-active)",
      ]}
      {onclick}
      onpointerenter={(event) => {
        if (event.pointerType === "mouse") {
          showTooltip(event, label);
        }
      }}
      onpointerleave={() => (tooltip = null)}
      onfocus={(event) => {
        // Keyboard focus only: a tap or a click already shows the server it opens.
        if (event.currentTarget.matches(":focus-visible")) {
          showTooltip(event, label);
        }
      }}
      onblur={() => (tooltip = null)}
    >
      {@render content()}
    </button>
  </li>
{/snippet}

{#snippet homeMark()}
  <!-- The button paints Melli's tile, so it morphs and turns apricot like the other icons. -->
  <Melli state={homeMelliState(client.status, hasUnreadConversation)} sizeToken="--mw-component-rail-item-size" hasTile={false}
    shouldIdleOnlyOnHover
  />
{/snippet}

{#snippet addIcon()}
  <Icon name="plus" class="size-[calc(var(--mw-component-rail-item-size)/2)]" />
{/snippet}

<nav
  aria-label="Servers"
  class="h-full w-(--mw-component-rail-width) shrink-0 overflow-x-hidden overflow-y-auto bg-(--mw-component-rail-background) [scrollbar-width:none]"
>
  <ul class="flex flex-col items-center gap-(--mw-component-rail-item-gap) pt-(--mw-component-rail-padding-top) pb-(--mw-component-rail-item-gap)">
    {@render railItem("Direct messages", isHomeSelected, hasUnreadConversation, () => select(null), homeMark, "home")}
    <li aria-hidden="true" class="h-(--mw-size-rail-separator-height) w-(--mw-size-rail-separator-width) rounded-full bg-(--mw-component-rail-separator-color)"></li>
    {#each client.guilds as guild (guild.id)}
      {@const hasUnread = guild.channels.some((channel) => client.hasUnread(channel.id))}
      {#snippet initials()}
        <span aria-hidden="true" class="type-body font-medium">{serverInitials(guild.name)}</span>
      {/snippet}
      {@render railItem(guild.name, client.selectedGuildId === guild.id, hasUnread, () => select(guild.id), initials, "server")}
    {/each}
    {@render railItem("Add a server", false, false, onaddserver, addIcon, "action")}
  </ul>
</nav>

{#if tooltip}
  <!-- Rail tooltips appear at once, to the trailing side (DESIGN.md §5.2); decorative copy of the label. -->
  <div
    aria-hidden="true"
    class="pointer-events-none fixed z-50 max-w-(--mw-component-tooltip-max-width) -translate-y-1/2 rounded-(--mw-component-tooltip-radius) bg-(--mw-component-tooltip-background) px-(--mw-component-tooltip-padding-inline) py-(--mw-component-tooltip-padding-block) type-tooltip break-words text-(--mw-component-tooltip-text) shadow-(--mw-component-tooltip-shadow)"
    style:top="{tooltip.top}px"
    style:left="calc(var(--mw-component-rail-width) - var(--mw-component-rail-item-gap))"
  >
    {tooltip.text}
  </div>
{/if}

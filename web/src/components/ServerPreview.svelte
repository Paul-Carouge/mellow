<script lang="ts">
  import { strings } from "../i18n";
  import type { InitialCategory } from "../lib/protocol";
  import { serverInitials } from "./format";
  import Icon from "./Icon.svelte";

  // The live preview of "Create a server" (DESIGN.md §5.2): the navigator card in miniature, with
  // the sidebar's own tokens and type at their real size, so what is shown is what the server will
  // look like once created: its icon selected in the strip, its name, the template's categories
  // and channels, the first one open. It repeats the form for the eye, so screen readers skip it.
  interface Props {
    name: string;
    /** A still picture and where to draw it so the chosen square fills the icon. */
    icon: { url: string; placement: { left: number; top: number; width: number } } | null;
    categories: readonly InitialCategory[];
  }

  let { name, icon, categories }: Props = $props();

  const shownName = $derived(name.trim());
</script>

<div
  aria-hidden="true"
  class="flex max-h-(--mw-component-server-preview-max-height) overflow-hidden rounded-(--mw-component-server-preview-radius) border-(length:--mw-component-server-preview-border-width) border-(--mw-component-server-preview-border) bg-(--mw-component-server-preview-background)"
>
  <!-- The strip: the new server, selected, with the apricot pill. -->
  <div class="relative flex w-(--mw-component-navigator-strip-width) shrink-0 justify-center pt-(--mw-component-rail-padding-top)">
    <span
      class="absolute start-0 top-[calc(var(--mw-component-rail-padding-top)+(var(--mw-component-rail-item-size)-var(--mw-component-rail-pill-height-selected))/2)] h-(--mw-component-rail-pill-height-selected) w-(--mw-component-rail-pill-width) -translate-x-1/2 rounded-full bg-(--mw-component-rail-pill-color-selected)"
    ></span>
    <span
      class="relative grid size-(--mw-component-rail-item-size) place-items-center overflow-hidden rounded-(--mw-component-rail-icon-radius-active) bg-(--mw-component-rail-icon-background-active) type-heading-md text-(--mw-component-rail-icon-text-active)"
    >
      {#if icon}
        <img
          src={icon.url}
          alt=""
          class="absolute max-w-none"
          style:left="{icon.placement.left}%"
          style:top="{icon.placement.top}%"
          style:width="{icon.placement.width}%"
        />
      {:else}
        {serverInitials(shownName || strings.addServer.previewPlaceholder)}
      {/if}
    </span>
  </div>

  <!-- The channel list, as the sidebar draws it. -->
  <div class="flex min-w-0 flex-1 flex-col pe-xs">
    <p
      class={[
        "flex h-(--mw-component-sidebar-header-height) shrink-0 items-center gap-xs px-xs type-heading-md",
        shownName ? "text-(--mw-component-sidebar-header-text)" : "text-(--mw-component-server-preview-placeholder-text)",
      ]}
    >
      <span class="min-w-0 flex-1 truncate">{shownName || strings.addServer.previewPlaceholder}</span>
      <Icon name="caret-down" class="size-(--mw-component-sidebar-header-chevron-size) shrink-0 text-text-secondary" />
    </p>
    {#each categories as category, categoryIndex (categoryIndex)}
      <p
        class="mt-(--mw-component-category-margin-top) flex h-(--mw-component-category-height) shrink-0 items-center gap-2xs type-category text-(--mw-component-category-text)"
      >
        <Icon name="caret-down" class="size-(--mw-component-category-chevron-size) shrink-0" />
        <span class="truncate">{category.name}</span>
      </p>
      {#each category.channels as channel, channelIndex (channelIndex)}
        {@const isOpen = categoryIndex === 0 && channelIndex === 0}
        <p
          class={[
            "mt-(--mw-component-channel-row-gap) flex h-(--mw-component-channel-row-height) shrink-0 items-center gap-(--mw-component-channel-row-icon-gap) rounded-(--mw-component-channel-row-radius) px-(--mw-component-channel-row-padding-inline)",
            isOpen
              ? "bg-(--mw-component-channel-row-background-selected) text-(--mw-component-channel-row-text-selected)"
              : "text-(--mw-component-channel-row-text-read)",
          ]}
        >
          <Icon name="hash" class="size-(--mw-component-channel-row-icon-size) shrink-0" />
          <span class="min-w-0 truncate type-channel-name">{channel.name}</span>
        </p>
      {/each}
    {/each}
  </div>
</div>

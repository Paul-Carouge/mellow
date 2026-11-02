<script module lang="ts">
  // What the navigator's channel and conversation rows share (DESIGN.md §5.3).
  import Icon from "./Icon.svelte";

  export { mutedMark, unreadPill };

  /** A muted row is greyed and never shows unread (DESIGN.md §5.3); its bell-slash says so too. */
  export const sidebarRowClasses = (isSelected: boolean, hasUnread: boolean, isMuted = false) => [
    "flex w-full items-center gap-(--mw-component-channel-row-icon-gap) rounded-(--mw-component-channel-row-radius) px-(--mw-component-channel-row-padding-inline) text-start transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) focus-visible:focus-ring-inset",
    isSelected
      ? "bg-(--mw-component-channel-row-background-selected) text-(--mw-component-channel-row-text-selected)"
      : [
          isMuted
            ? "text-(--mw-component-channel-row-text-muted-channel)"
            : hasUnread
              ? "text-(--mw-component-channel-row-text-unread)"
              : "text-(--mw-component-channel-row-text-read)",
          "hover:bg-(--mw-component-channel-row-background-hover) hover:text-(--mw-component-channel-row-text-hover)",
        ],
  ];
</script>

{#snippet mutedMark()}
  <Icon name="bell-slash" class="size-(--mw-component-channel-row-muted-icon-size) shrink-0 text-(--mw-component-channel-row-muted-icon)" />
{/snippet}

{#snippet unreadPill()}
  <span
    aria-hidden="true"
    class="absolute top-1/2 -left-(--mw-component-channel-row-unread-pill-offset) h-(--mw-component-channel-row-unread-pill-height) w-(--mw-component-channel-row-unread-pill-width) -translate-y-1/2 rounded-full bg-(--mw-component-channel-row-unread-pill-color)"
  ></span>
{/snippet}

<script lang="ts">
  import { strings } from "../i18n";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { lengthTokenInPixels } from "./tokens";

  // "Go to… Ctrl K" in the chat header (DESIGN.md 5.10), beside message search (5.12). It reads
  // as a field but is a button: the field itself lives in the switcher, so typing starts there.
  // Its own header decides its form: below `trigger-collapse-width` beside the search field (a
  // narrow chat next to a docked sheet), or below the chat's minimum width beside search's
  // magnifier (phones), it becomes a compass button, so the channel name keeps its room and two
  // magnifier buttons never sit side by side.
  interface Props {
    /** Beside the search field it takes its content's width; alone it keeps item 12's 208 px. */
    isBesideSearchField: boolean;
    onopen: () => void;
  }

  let { isBesideSearchField, onopen }: Props = $props();

  let isCollapsed = $state(false);

  function followHeaderWidth(element: HTMLElement) {
    const header = element.closest("header");
    if (!header) {
      return;
    }
    const observer = new ResizeObserver(() => {
      const collapseWidthToken = isBesideSearchField ? "component-quick-switcher-trigger-collapse-width" : "size-chat-min-width";
      isCollapsed = header.clientWidth < lengthTokenInPixels(collapseWidthToken);
    });
    observer.observe(header);
    return () => observer.disconnect();
  }
</script>

<span class="contents" {@attach followHeaderWidth}>
  {#if isCollapsed}
    <IconButton
      icon="compass"
      label={strings.chat.goToLabel}
      aria-haspopup="dialog"
      aria-keyshortcuts="Control+K"
      class="size-(--mw-component-chat-header-button-size) text-(--mw-component-chat-header-icon) hover:bg-state-hover hover:text-text-normal"
      iconClass="size-(--mw-component-chat-header-icon-size)"
      onclick={onopen}
    />
  {:else}
    <button
      type="button"
      aria-label={strings.chat.goToLabel}
      aria-keyshortcuts="Control+K"
      class={[
        "group flex h-(--mw-component-chat-header-button-size) min-w-max items-center gap-xs rounded-sm bg-surface-input ps-xs pe-2xs type-body text-text-secondary transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:text-text-heading pointer-coarse:h-(--mw-size-hit-web-min)",
        isBesideSearchField ? "shrink-0" : "flex-[0_1_var(--mw-component-quick-switcher-trigger-width)]",
      ]}
      onclick={onopen}
    >
      <Icon
        name="magnifying-glass"
        class="size-(--mw-component-quick-switcher-trigger-icon-size) text-(--mw-component-quick-switcher-trigger-icon) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) group-hover:text-(--mw-component-quick-switcher-trigger-icon-hover)"
      />
      <span class="me-auto">{strings.chat.goTo}</span>
      <kbd class="rounded-xs border border-border-divider px-2xs font-sans type-body-sm text-text-muted">Ctrl K</kbd>
    </button>
  {/if}
</span>

<script lang="ts" module>
  import type { IconName } from "./Icon.svelte";

  /** What an action that waits for the instance (or the clipboard) ended with. */
  export type ContextMenuOutcome = { readonly announcement: string } | { readonly error: string };

  interface ContextMenuItemBase {
    readonly label: string;
    readonly icon: IconName;
    /** `accent` is Discord's highlighted "Invite people"; `danger` items come last, separated. */
    readonly tone?: "normal" | "accent" | "danger";
  }

  /**
   * `onselect` closes the menu first, then acts (opening a dialog, moving focus). `onselectAndWait`
   * keeps the menu open, its items dimmed, until the action ends: it closes and the announcement
   * is read out, or the error shows at the menu's foot and the menu stays usable (DESIGN.md §6.33).
   */
  export type ContextMenuItem = ContextMenuItemBase &
    ({ readonly onselect: () => void } | { readonly onselectAndWait: () => Promise<ContextMenuOutcome> });

  export type ContextMenuEntry = ContextMenuItem | "separator";
</script>

<script lang="ts">
  import { flushSync, type Snippet } from "svelte";

  import Icon from "./Icon.svelte";
  import { menuItemsIn, moveFocusWithinMenu } from "./menu-keyboard";
  import { type MenuOrigin, placeMenu } from "./menu-placement";
  import { lengthTokenInPixels } from "./tokens";

  // The one menu of the web client (DESIGN.md §6.33): a light-dismiss popover with the menu
  // pattern (arrows wrap, Home and End, type-ahead, Esc), placed at the pointer or next to the
  // element that opened it, kept inside the window, lifting in when it opens above. Focus goes to
  // the checked item or the first one, and back to the opener when the menu closes.
  interface Props {
    label: string;
    id?: string;
    /** The items, in order; a host with its own item markup (the status menu) passes `children`. */
    entries?: readonly ContextMenuEntry[];
    children?: Snippet;
    /** Controls before the entries (a voice participant's volume and local mute). */
    header?: Snippet;
    /** Set by a host that runs its own waiting actions (the status menu); shown at the foot. */
    errorMessage?: string;
    /** Space between the menu and the element it opens from, as a token name; none by default. */
    gapToken?: string | null;
    isOpen?: boolean;
    /** Focus had nowhere to go back to (the opener left meanwhile). */
    fallbackFocus?: () => void;
    onclose?: () => void;
  }

  let {
    label,
    id,
    entries = [],
    children,
    header,
    errorMessage = "",
    gapToken = null,
    isOpen = $bindable(false),
    fallbackFocus,
    onclose,
  }: Props = $props();

  let menu = $state<HTMLElement | null>(null);
  let invoker: HTMLElement | null = null;
  let placement = $state({ top: 0, left: 0, bottom: null as number | null, width: null as number | null, isAbove: false });
  let isWaiting = $state(false);
  let waitingError = $state("");
  let announcement = $state("");
  const shownError = $derived(errorMessage || waitingError);

  /**
   * Opens at `origin`; `opener` gets focus back when the menu closes. Hosts set what the entries
   * depend on first: they are rendered here before the menu is measured. A menu opened by a
   * pointer at a point (right-click, long press) focuses itself, as desktop menus do, so no item
   * looks chosen until an arrow or a letter moves to one; from a button or the keyboard, the
   * checked item or the first one takes focus.
   */
  export function open(origin: MenuOrigin, opener: HTMLElement | null, isFromPointer = origin.kind === "point"): void {
    flushSync();
    if (!menu) {
      return;
    }
    if (menu.matches(":popover-open")) {
      menu.hidePopover();
    }
    invoker = opener;
    // The popover gives focus back to whatever held it before it opened: that must be the opener,
    // not the composer a right-click or a long press left focused.
    if (opener && !opener.contains(document.activeElement)) {
      opener.focus({ preventScroll: true });
    }
    waitingError = "";
    menu.showPopover();
    const viewport = { width: window.innerWidth, height: window.innerHeight };
    const measured = placeMenu(
      origin,
      { width: menu.offsetWidth, height: menu.offsetHeight },
      viewport,
      gapToken ? lengthTokenInPixels(gapToken) : 0,
      lengthTokenInPixels("component-context-menu-viewport-margin"),
    );
    // Above its origin, the menu keeps its bottom edge there, so an error appearing at its foot
    // grows it upwards rather than over what opened it.
    placement = {
      ...measured,
      bottom: measured.isAbove ? viewport.height - measured.top - menu.offsetHeight : null,
    };
    if (isFromPointer) {
      menu.focus();
      return;
    }
    const items = menuItemsIn(menu);
    (items.find((item) => item.getAttribute("aria-checked") === "true") ?? items[0])?.focus();
  }

  export function close(): void {
    // A waiting action may end after the menu left the page (the conversation it opened replaced
    // the list holding it): there is nothing to hide then.
    if (menu?.isConnected && menu.matches(":popover-open")) {
      menu.hidePopover();
    }
  }

  async function choose(item: ContextMenuItem) {
    if (isWaiting) {
      return;
    }
    if ("onselect" in item) {
      close();
      item.onselect();
      return;
    }
    isWaiting = true;
    waitingError = "";
    try {
      const outcome = await item.onselectAndWait();
      if ("error" in outcome) {
        waitingError = outcome.error;
      } else {
        close();
        announcement = outcome.announcement;
      }
    } finally {
      isWaiting = false;
    }
  }

  const separatorClass =
    "mx-(--mw-component-context-menu-separator-margin-inline) my-(--mw-component-context-menu-separator-margin-block) h-(--mw-component-context-menu-separator-height) shrink-0 border-0 bg-(--mw-component-context-menu-separator)";
</script>

<!-- Light dismiss and Esc come from the popover; focus goes back to the opener when it closes. -->
<div
  bind:this={menu}
  {id}
  popover="auto"
  role="menu"
  tabindex="-1"
  aria-label={label}
  aria-busy={isWaiting || undefined}
  class={[
    "fixed inset-auto m-0 min-w-(--mw-component-context-menu-min-width) outline-none flex-col rounded-(--mw-component-context-menu-radius) border-(length:--mw-component-context-menu-border-width) border-(--mw-component-context-menu-border) bg-(--mw-component-context-menu-background) p-(--mw-component-context-menu-padding) shadow-(--mw-component-context-menu-shadow) [&:popover-open]:flex",
    // A stretched menu is as wide as its opener (the server header, the user card).
    placement.width === null && "w-max max-w-(--mw-component-context-menu-max-width)",
    placement.isAbove
      ? "[&:popover-open]:animate-[mw-lift-in_var(--mw-motion-duration-fast)_var(--mw-motion-easing-enter)_both]"
      : "[&:popover-open]:animate-[mw-drop-in_var(--mw-motion-duration-fast)_var(--mw-motion-easing-enter)_both]",
  ]}
  style:top={placement.bottom === null ? `${placement.top}px` : "auto"}
  style:bottom={placement.bottom === null ? "auto" : `${placement.bottom}px`}
  style:left="{placement.left}px"
  style:width={placement.width === null ? undefined : `${placement.width}px`}
  onkeydown={(event) => moveFocusWithinMenu(event, menu)}
  oncontextmenu={(event) => event.preventDefault()}
  ontoggle={(event) => {
    isOpen = event.newState === "open";
    if (isOpen) {
      return;
    }
    waitingError = "";
    if (menu?.contains(document.activeElement) || document.activeElement === document.body) {
      if (invoker?.isConnected) {
        invoker.focus();
      } else {
        fallbackFocus?.();
      }
    }
    onclose?.();
  }}
>
  {#if children}
    {@render children()}
  {:else}
    {@render header?.()}
    {#each entries as entry, index (index)}
      {#if entry === "separator"}
        <hr class={separatorClass} />
      {:else}
        {@const tone = entry.tone ?? "normal"}
        <button
          type="button"
          role="menuitem"
          aria-disabled={isWaiting || undefined}
          class={[
            "flex min-h-(--mw-component-context-menu-item-height) w-full items-center justify-between gap-xs rounded-(--mw-component-context-menu-item-radius) px-(--mw-component-context-menu-item-padding-inline) py-3xs text-start type-menu-item focus-visible:focus-ring-inset aria-disabled:opacity-(--mw-component-button-disabled-opacity) pointer-coarse:min-h-(--mw-size-hit-web-min)",
            tone === "danger"
              ? "text-(--mw-component-context-menu-danger-text) hover:bg-(--mw-component-context-menu-danger-background-focus) hover:text-(--mw-component-context-menu-danger-text-focus) focus-visible:bg-(--mw-component-context-menu-danger-background-focus) focus-visible:text-(--mw-component-context-menu-danger-text-focus)"
              : [
                  tone === "accent"
                    ? "text-(--mw-component-context-menu-accent-text)"
                    : "text-(--mw-component-context-menu-item-text)",
                  "hover:bg-(--mw-component-context-menu-item-background-focus) hover:text-(--mw-component-context-menu-item-text-focus) focus-visible:bg-(--mw-component-context-menu-item-background-focus) focus-visible:text-(--mw-component-context-menu-item-text-focus)",
                ],
          ]}
          onclick={() => void choose(entry)}
        >
          {entry.label}
          <Icon name={entry.icon} class="size-(--mw-component-context-menu-item-icon-size) shrink-0" />
        </button>
      {/if}
    {/each}
  {/if}
  <!-- Kept in the menu, so the error is announced as soon as it appears. -->
  <div role="alert" class="contents">
    {#if shownError}
      <p class="flex gap-xs px-(--mw-component-context-menu-item-padding-inline) py-2xs type-body-sm text-text-danger">
        <Icon name="warning-circle" class="size-(--mw-component-context-menu-item-icon-size) shrink-0" />
        <span>{shownError}</span>
      </p>
    {/if}
  </div>
</div>

<!-- Outside the popover, which is hidden by the time a result is read out. -->
<p class="sr-only" aria-live="polite">{announcement}</p>

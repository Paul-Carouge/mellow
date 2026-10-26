<script lang="ts">
  import { onMount } from "svelte";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { ChosenStatus, CurrentUser } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import CustomStatusDialog from "./CustomStatusDialog.svelte";
  import { displayName } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { menuItemsIn, moveFocusWithinMenu } from "./menu-keyboard";
  import { createPopoverToggle } from "./popover-toggle";
  import StatusIndicator from "./StatusIndicator.svelte";
  import { durationTokenInMilliseconds } from "./tokens";
  import { UNMUTED_VOICE_TOGGLES, toggleDeafened, toggleMicrophone } from "./voice-toggles";

  // The user card across the navigator's foot (ADR-0022 §4): as tall as the composer's base row
  // and as far from the bottom of its card, so both line up across the window (size.dock.*).
  // The avatar and name open the status menu (DESIGN.md §5.3); the gear opens the settings.
  interface Props {
    client: MellowClient;
    user: CurrentUser;
    onopenusersettings: () => void;
  }

  let { client, user, onopenusersettings }: Props = $props();

  // Voice does not exist yet: the microphone and headset toggles show in development builds only,
  // and the card keeps its height so nothing moves when they ship (ADR-0022 §5).
  const ARE_VOICE_TOGGLES_SHOWN = import.meta.env.DEV;

  const CHOSEN_STATUSES: readonly ChosenStatus[] = ["online", "idle", "do_not_disturb", "invisible"];
  const HINT_BY_STATUS: Partial<Record<ChosenStatus, string>> = {
    do_not_disturb: strings.userCard.doNotDisturbHint,
    invisible: strings.userCard.invisibleHint,
  };

  let voice = $state(UNMUTED_VOICE_TOGGLES);

  const chosenStatus = $derived(user.status ?? "online");
  // Your own card shows your sentence even while invisible; others see none then.
  const customStatus = $derived(user.custom_status ?? null);

  let card = $state<HTMLElement | null>(null);
  let statusButton = $state<HTMLButtonElement | null>(null);
  let menu = $state<HTMLElement | null>(null);
  let isMenuOpen = $state(false);
  let menuPosition = $state({ bottom: 0, left: 0, width: 0 });
  let isSavingStatus = $state(false);
  let menuError = $state("");
  let isCustomStatusDialogOpen = $state(false);
  let isUserIdCopied = $state(false);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  const menuToggle = createPopoverToggle(() => isMenuOpen);

  onMount(() => () => clearTimeout(copiedTimer));

  function openMenu() {
    if (!card || !menu) {
      return;
    }
    // Above the card, across its width: the card sits at the bottom of the window.
    const bounds = card.getBoundingClientRect();
    menuPosition = { bottom: window.innerHeight - bounds.top, left: bounds.left, width: bounds.width };
    menuError = "";
    menu.showPopover();
    const items = menuItemsIn(menu);
    (items.find((item) => item.getAttribute("aria-checked") === "true") ?? items[0])?.focus();
  }

  async function chooseStatus(status: ChosenStatus) {
    if (isSavingStatus) {
      return;
    }
    if (status === chosenStatus) {
      menu?.hidePopover();
      return;
    }
    isSavingStatus = true;
    menuError = "";
    try {
      await client.updatePresence(status, customStatus);
      menu?.hidePopover();
    } catch (error) {
      menuError = describeError(error, strings.userCard.statusFailed);
    } finally {
      isSavingStatus = false;
    }
  }

  function openCustomStatusDialog() {
    menu?.hidePopover();
    isCustomStatusDialogOpen = true;
  }

  async function copyUserId() {
    menuError = "";
    try {
      await navigator.clipboard.writeText(user.id);
    } catch (error) {
      console.warn("copying the user ID failed", error);
      menuError = strings.userCard.copyUserIdFailed;
      return;
    }
    menu?.hidePopover();
    // The card's second line says so where focus lands, and the live region tells screen readers.
    isUserIdCopied = true;
    clearTimeout(copiedTimer);
    copiedTimer = setTimeout(
      () => (isUserIdCopied = false),
      durationTokenInMilliseconds("component-dialog-copied-label-duration"),
    );
  }

  const buttonClass =
    "relative size-(--mw-component-user-panel-button-size) text-(--mw-component-user-panel-button-icon) hover:bg-(--mw-component-user-panel-button-background-hover) hover:text-text-normal";
  const iconClass = "size-(--mw-component-user-panel-button-icon-size)";
  const menuItemClass =
    "flex min-h-(--mw-component-context-menu-item-height) w-full items-center gap-xs rounded-(--mw-component-context-menu-item-radius) px-(--mw-component-context-menu-item-padding-inline) py-2xs text-start text-(--mw-component-context-menu-item-text) hover:bg-(--mw-component-context-menu-item-background-focus) hover:text-(--mw-component-context-menu-item-text-focus) aria-disabled:opacity-(--mw-component-button-disabled-opacity) pointer-coarse:min-h-(--mw-size-hit-web-min)";
  const menuIconBoxClass = "grid size-(--mw-component-context-menu-item-icon-size) shrink-0 place-items-center";
  const menuHintClass = "truncate type-body-sm text-(--mw-component-context-menu-item-hint-text)";
  const separatorClass =
    "mx-(--mw-component-context-menu-separator-margin-inline) my-(--mw-component-context-menu-separator-margin-block) h-(--mw-component-context-menu-separator-height) shrink-0 border-0 bg-(--mw-component-context-menu-separator)";
</script>

<div
  bind:this={card}
  class="col-span-full flex h-(--mw-size-dock-height) min-w-0 items-center gap-2xs rounded-(--mw-component-user-panel-radius) bg-(--mw-component-user-panel-background) ps-2xs pe-2xs mx-(--mw-size-dock-margin) mb-(--mw-size-dock-margin)"
>
  <button
    bind:this={statusButton}
    type="button"
    aria-haspopup="menu"
    aria-expanded={isMenuOpen}
    aria-controls="status-menu"
    aria-label={strings.userCard.statusButtonLabel(displayName(user), strings.presence[chosenStatus], customStatus)}
    class="status-button flex min-w-0 flex-1 items-center gap-xs self-stretch rounded-sm my-2xs px-2xs text-start transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard)"
    onpointerdown={menuToggle.notePress}
    onclick={() => (menuToggle.shouldOpenOnClick() ? openMenu() : menu?.hidePopover())}
  >
    <Avatar {user} sizeToken="--mw-component-user-panel-avatar-size" presence={chosenStatus} ringColorToken="--status-cut" />
    <span class="flex min-w-0 flex-1 flex-col">
      <span class="truncate type-user-panel-name text-(--mw-component-user-panel-name-text)">{displayName(user)}</span>
      {#if isUserIdCopied}
        <span class="flex min-w-0 items-center gap-2xs type-body-sm text-(--mw-component-user-panel-status-text)">
          <Icon name="check" class="size-(--mw-component-status-size-menu)" />
          <span class="truncate">{strings.userCard.userIdCopied}</span>
        </span>
      {:else}
        <span class="truncate type-body-sm text-(--mw-component-user-panel-status-text)" title={customStatus ?? undefined}
          >{customStatus ?? `@${user.username}`}</span
        >
      {/if}
    </span>
  </button>
  {#if ARE_VOICE_TOGGLES_SHOWN}
    <!-- The accessible name stays; the state is aria-pressed, the strike and the colour (never colour alone). -->
    <IconButton
      icon="microphone"
      label={strings.userCard.muteMicrophone}
      title={voice.isMicrophoneMuted ? strings.userCard.unmuteMicrophone : strings.userCard.muteMicrophone}
      aria-pressed={voice.isMicrophoneMuted}
      class="voice-toggle {buttonClass}"
      {iconClass}
      onclick={() => (voice = toggleMicrophone(voice))}
    />
    <IconButton
      icon="headphones"
      label={strings.userCard.deafen}
      title={voice.isDeafened ? strings.userCard.undeafen : strings.userCard.deafen}
      aria-pressed={voice.isDeafened}
      class="voice-toggle {buttonClass}"
      {iconClass}
      onclick={() => (voice = toggleDeafened(voice))}
    />
  {/if}
  <!-- Sign out lives in the settings navigation, as on desktop (DESIGN.md §5.9). -->
  <IconButton icon="gear-six" label={strings.userCard.userSettings} class={buttonClass} {iconClass} onclick={onopenusersettings} />
</div>

<!-- Light dismiss and Esc come from the popover; focus goes back to the card when it closes. -->
<div
  bind:this={menu}
  id="status-menu"
  popover="auto"
  role="menu"
  tabindex="-1"
  aria-label={strings.userCard.statusMenu}
  class="fixed inset-auto m-0 min-w-(--mw-component-context-menu-min-width) flex-col rounded-(--mw-component-context-menu-radius) border-(length:--mw-component-context-menu-border-width) border-(--mw-component-context-menu-border) bg-(--mw-component-context-menu-background) p-(--mw-component-context-menu-padding) shadow-(--mw-component-context-menu-shadow) [&:popover-open]:flex [&:popover-open]:animate-[mw-lift-in_var(--mw-motion-duration-fast)_var(--mw-motion-easing-enter)_both]"
  style:bottom="calc({menuPosition.bottom}px + var(--mw-space-2xs))"
  style:left="{menuPosition.left}px"
  style:width="{menuPosition.width}px"
  onkeydown={(event) => moveFocusWithinMenu(event, menu)}
  ontoggle={(event) => {
    isMenuOpen = event.newState === "open";
    if (!isMenuOpen && (menu?.contains(document.activeElement) || document.activeElement === document.body)) {
      statusButton?.focus();
    }
  }}
>
  {#each CHOSEN_STATUSES as status (status)}
      {@const isChecked = status === chosenStatus}
      {@const hint = HINT_BY_STATUS[status]}
      <button
        type="button"
        role="menuitemradio"
        aria-checked={isChecked}
        aria-disabled={isSavingStatus || undefined}
        class={menuItemClass}
        onclick={() => void chooseStatus(status)}
      >
        <span class={menuIconBoxClass}><StatusIndicator {status} sizeToken="--mw-component-status-size-menu" /></span>
        <span class="flex min-w-0 flex-1 flex-col">
          <span class="type-menu-item">{strings.presence[status]}</span>
          {#if hint}<span class={menuHintClass}>{hint}</span>{/if}
        </span>
        <!-- The check says which one is yours without relying on colour. -->
        {#if isChecked}<Icon name="check" class="size-(--mw-component-context-menu-item-icon-size)" />{/if}
      </button>
  {/each}
  <hr class={separatorClass} />
  <button type="button" role="menuitem" class={menuItemClass} onclick={openCustomStatusDialog}>
    <span class={menuIconBoxClass}><Icon name="smiley" class="size-(--mw-component-context-menu-item-icon-size)" /></span>
    <span class="min-w-0 flex-1 type-menu-item">{customStatus ? strings.userCard.editCustomStatus : strings.userCard.setCustomStatus}</span>
  </button>
  <hr class={separatorClass} />
  <button type="button" role="menuitem" class={menuItemClass} onclick={() => void copyUserId()}>
    <span class={menuIconBoxClass}><Icon name="copy" class="size-(--mw-component-context-menu-item-icon-size)" /></span>
    <span class="min-w-0 flex-1 type-menu-item">{strings.userCard.copyUserId}</span>
  </button>
  <!-- Kept in the menu, so the error is announced as soon as it appears. -->
  <div role="alert" class="contents">
    {#if menuError}
      <p class="flex gap-xs px-(--mw-component-context-menu-item-padding-inline) py-2xs type-body-sm text-text-danger">
        <Icon name="warning-circle" class="size-(--mw-component-context-menu-item-icon-size)" />
        <span>{menuError}</span>
      </p>
    {/if}
  </div>
</div>

<!-- Exists from the start, so its first change is announced. -->
<p class="sr-only" aria-live="polite">{isUserIdCopied ? strings.userCard.userIdCopied : ""}</p>

{#if isCustomStatusDialogOpen}
  <CustomStatusDialog
    {client}
    onclose={() => (isCustomStatusDialogOpen = false)}
    fallbackFocus={() => statusButton?.focus()}
  />
{/if}

<style>
  /* The status shape is cut out of whatever the button shows behind it, hovered or open too. */
  .status-button {
    --status-cut: var(--mw-component-user-panel-background);
    background-color: var(--status-cut);
  }
  .status-button:hover,
  .status-button[aria-expanded="true"] {
    --status-cut: var(--mw-component-user-panel-button-background-hover);
  }
  /* The strike is cut out of the surface behind it, so it reads across the glyph. */
  :global(.voice-toggle) {
    --strike-cut: var(--mw-component-user-panel-background);
  }
  :global(.voice-toggle:hover) {
    --strike-cut: var(--mw-component-user-panel-button-background-hover);
  }
  :global(.voice-toggle[aria-pressed="true"]) {
    color: var(--mw-component-user-panel-toggle-muted-icon);
  }
  :global(.voice-toggle)::after {
    content: "";
    position: absolute;
    left: 50%;
    top: 50%;
    width: var(--mw-component-user-panel-toggle-strike-width);
    height: var(--mw-component-user-panel-toggle-strike-length);
    border-radius: var(--mw-radius-full);
    background: var(--mw-component-user-panel-toggle-muted-icon);
    box-shadow: 0 0 0 var(--mw-component-user-panel-toggle-strike-cut-width) var(--strike-cut);
    translate: -50% -50%;
    rotate: -45deg;
    scale: 1 0;
    transition: scale var(--mw-motion-duration-fast) var(--mw-motion-easing-enter);
  }
  :global(.voice-toggle[aria-pressed="true"])::after {
    scale: 1 1;
  }
</style>

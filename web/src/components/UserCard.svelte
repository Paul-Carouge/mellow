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
  import ContextMenu from "./ContextMenu.svelte";
  import { createPopoverToggle } from "./popover-toggle";
  import StatusIndicator from "./StatusIndicator.svelte";
  import { durationTokenInMilliseconds } from "./tokens";
  import { prepareChimeSound } from "./chime-sound";

  // The user card across the navigator's foot (ADR-0022 §4): as tall as the composer's base row
  // and as far from the bottom of its card, so both line up across the window (size.dock.*).
  // The avatar and name open the status menu (DESIGN.md §5.3); the gear opens the settings.
  interface Props {
    client: MellowClient;
    user: CurrentUser;
    onopenusersettings: () => void;
  }

  let { client, user, onopenusersettings }: Props = $props();

  const CHOSEN_STATUSES: readonly ChosenStatus[] = ["online", "idle", "do_not_disturb", "invisible"];
  const HINT_BY_STATUS: Partial<Record<ChosenStatus, string>> = {
    do_not_disturb: strings.userCard.doNotDisturbHint,
    invisible: strings.userCard.invisibleHint,
  };

  // The microphone and headset toggles drive the voice connection (ADR-0030), and hold before
  // joining too, stored in this browser, as on Discord.
  const voice = $derived(client.voice);
  const isMicrophoneMuted = $derived(voice.isMicrophoneMuted);
  const microphoneProblem = $derived(
    voice.phase !== "idle" && voice.microphoneProblem ? strings.voice.microphoneProblems[voice.microphoneProblem] : null,
  );

  /** A click is the gesture that lets the toggle sounds play. */
  function toggleMicrophoneFromClick() {
    void prepareChimeSound();
    void voice.toggleMicrophone();
  }

  function toggleDeafenedFromClick() {
    void prepareChimeSound();
    voice.toggleDeafened();
  }

  const chosenStatus = $derived(user.status ?? "online");
  // Your own card shows your sentence even while invisible; others see none then.
  const customStatus = $derived(user.custom_status ?? null);

  let card = $state<HTMLElement | null>(null);
  let statusButton = $state<HTMLButtonElement | null>(null);
  let menu = $state<ContextMenu | null>(null);
  let isMenuOpen = $state(false);
  let isSavingStatus = $state(false);
  let menuError = $state("");
  let isCustomStatusDialogOpen = $state(false);
  let isUserIdCopied = $state(false);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  const menuToggle = createPopoverToggle(() => isMenuOpen);

  onMount(() => () => clearTimeout(copiedTimer));

  function openMenu() {
    if (!card) {
      return;
    }
    // Across the card's width, above it: the card sits at the bottom of the window. Focus goes
    // to the current status.
    menuError = "";
    menu?.open({ kind: "anchor", bounds: card.getBoundingClientRect(), align: "stretch" }, statusButton);
  }

  async function chooseStatus(status: ChosenStatus) {
    if (isSavingStatus) {
      return;
    }
    if (status === chosenStatus) {
      menu?.close();
      return;
    }
    isSavingStatus = true;
    menuError = "";
    try {
      await client.updatePresence(status, customStatus);
      menu?.close();
    } catch (error) {
      menuError = describeError(error, strings.userCard.statusFailed);
    } finally {
      isSavingStatus = false;
    }
  }

  function openCustomStatusDialog() {
    menu?.close();
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
    menu?.close();
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
    "group flex min-h-(--mw-component-context-menu-item-height) w-full items-center gap-xs rounded-(--mw-component-context-menu-item-radius) px-(--mw-component-context-menu-item-padding-inline) py-2xs text-start text-(--mw-component-context-menu-item-text) hover:bg-(--mw-component-context-menu-item-background-focus) hover:text-(--mw-component-context-menu-item-text-focus) aria-disabled:opacity-(--mw-component-button-disabled-opacity) pointer-coarse:min-h-(--mw-size-hit-web-min)";
  const menuIconBoxClass = "grid size-(--mw-component-context-menu-item-icon-size) shrink-0 place-items-center";
  const menuHintClass =
    "truncate type-body-sm text-(--mw-component-context-menu-item-hint-text) group-hover:text-(--mw-component-context-menu-item-hint-text-focus)";
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
    onclick={() => (menuToggle.shouldOpenOnClick() ? openMenu() : menu?.close())}
  >
    <Avatar {user} sizeToken="--mw-component-user-panel-avatar-size" presence={chosenStatus} ringColorToken="--status-cut" />
    <span class="flex min-w-0 flex-1 flex-col">
      <span class="truncate type-user-panel-name text-(--mw-component-user-panel-name-text)">{displayName(user)}</span>
      {#if isUserIdCopied}
        <span class="status-line flex min-w-0 items-center gap-2xs type-body-sm">
          <Icon name="check" class="size-(--mw-component-status-size-menu)" />
          <span class="truncate">{strings.userCard.userIdCopied}</span>
        </span>
      {:else}
        <span class="status-line truncate type-body-sm" title={customStatus ?? undefined}
          >{customStatus ?? `@${user.username}`}</span
        >
      {/if}
    </span>
  </button>
  <!-- The accessible name stays; the state is aria-pressed, the strike and the colour (never colour alone).
       Connected with no microphone, the tooltip says why it shows muted. -->
  <IconButton
    icon="microphone"
    label={strings.userCard.muteMicrophone}
    title={microphoneProblem ?? (isMicrophoneMuted ? strings.userCard.unmuteMicrophone : strings.userCard.muteMicrophone)}
    aria-pressed={isMicrophoneMuted}
    aria-keyshortcuts="Control+Shift+M"
    class="voice-toggle {buttonClass}"
    {iconClass}
    onclick={toggleMicrophoneFromClick}
  />
  <IconButton
    icon="headphones"
    label={strings.userCard.deafen}
    title={voice.toggles.isDeafened ? strings.userCard.undeafen : strings.userCard.deafen}
    aria-pressed={voice.toggles.isDeafened}
    aria-keyshortcuts="Control+Shift+D"
    class="voice-toggle {buttonClass}"
    {iconClass}
    onclick={toggleDeafenedFromClick}
  />
  <!-- Sign out lives in the settings navigation, as on desktop (DESIGN.md §5.9). -->
  <IconButton icon="gear-six" label={strings.userCard.userSettings} class={buttonClass} {iconClass} onclick={onopenusersettings} />
</div>

<ContextMenu
  bind:this={menu}
  bind:isOpen={isMenuOpen}
  id="status-menu"
  label={strings.userCard.statusMenu}
  errorMessage={menuError}
  gapToken="space-2xs"
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
</ContextMenu>

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
  .status-line {
    color: var(--mw-component-user-panel-status-text);
  }
  .status-button:hover .status-line,
  .status-button[aria-expanded="true"] .status-line {
    color: var(--mw-component-user-panel-status-text-hover);
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

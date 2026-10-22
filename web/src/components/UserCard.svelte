<script lang="ts">
  import { strings } from "../i18n";
  import type { CurrentUser } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import { displayName } from "./format";
  import IconButton from "./IconButton.svelte";
  import { UNMUTED_VOICE_TOGGLES, toggleDeafened, toggleMicrophone } from "./voice-toggles";

  // The user card across the navigator's foot (ADR-0022 §4): as tall as the composer's base row
  // and as far from the bottom of its card, so both line up across the window (size.dock.*).
  interface Props {
    user: CurrentUser;
    onopenusersettings: () => void;
  }

  let { user, onopenusersettings }: Props = $props();

  // Voice does not exist yet: the microphone and headset toggles show in development builds only,
  // and the card keeps its height so nothing moves when they ship (ADR-0022 §5).
  const ARE_VOICE_TOGGLES_SHOWN = import.meta.env.DEV;

  let voice = $state(UNMUTED_VOICE_TOGGLES);

  const buttonClass =
    "relative size-(--mw-component-user-panel-button-size) text-(--mw-component-user-panel-button-icon) hover:bg-(--mw-component-user-panel-button-background-hover) hover:text-text-normal";
  const iconClass = "size-(--mw-component-user-panel-button-icon-size)";
</script>

<div
  class="col-span-full flex h-(--mw-size-dock-height) min-w-0 items-center gap-xs rounded-(--mw-component-user-panel-radius) bg-(--mw-component-user-panel-background) ps-xs pe-2xs mx-(--mw-size-dock-margin) mb-(--mw-size-dock-margin)"
>
  <Avatar
    {user}
    sizeToken="--mw-component-user-panel-avatar-size"
    presence="online"
    ringColorToken="--mw-component-user-panel-background"
  />
  <div class="flex min-w-0 flex-1 flex-col">
    <p class="truncate type-user-panel-name text-(--mw-component-user-panel-name-text)">{displayName(user)}</p>
    <p class="truncate type-body-sm text-(--mw-component-user-panel-status-text)">@{user.username}</p>
  </div>
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

<style>
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

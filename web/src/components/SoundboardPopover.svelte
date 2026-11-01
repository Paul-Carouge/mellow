<script lang="ts">
  import { onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { MINIMUM_MILLISECONDS_BETWEEN_SOUNDS, Permission, type SoundboardSound } from "../lib/protocol";
  import { soundboardGain } from "../lib/soundboard";
  import Button from "./Button.svelte";
  import ContextMenu, { type ContextMenuEntry } from "./ContextMenu.svelte";
  import { contextMenuTrigger } from "./context-menu-trigger";
  import { nextEmojiCell } from "./emoji-grid";
  import IconButton from "./IconButton.svelte";
  import type { MenuOrigin } from "./menu-placement";
  import { holds } from "./server-settings";
  import SoundEmoji from "./SoundEmoji.svelte";
  import { previewSound } from "./soundboard-audio";

  // The soundboard (docs/design/web-soundboard.md, ADR-0032): a light-dismiss popover above the
  // voice panel's button, or a sheet at the bottom of a phone's screen, holding the connected
  // channel's server sounds. A click plays a sound for everyone in the channel; while it cannot
  // play, every sound dims and one sentence says why, and a bar drains through the 2 s between two
  // sounds. Each sound's menu (right-click, long press, Shift+F10) previews it for you only and
  // makes it your join sound; a pointer also finds the preview on the sound itself. One Tab stop
  // for the sounds, arrows between them, as in the emoji picker.
  interface Props {
    client: MellowClient;
    anchor: HTMLElement;
    isCompact: boolean;
    /** Opens Server settings at Soundboard; `null` without `MANAGE_EXPRESSIONS`. */
    onaddsounds: (() => void) | null;
    onclose: (shouldReturnFocus: boolean) => void;
  }

  let { client, anchor, isCompact, onaddsounds, onclose }: Props = $props();

  const COLUMNS = 3;

  const id = $props.id();
  let panel: HTMLElement;
  let grid = $state<HTMLElement | null>(null);
  let menu = $state<ReturnType<typeof ContextMenu> | null>(null);
  let menuSound = $state<SoundboardSound | null>(null);
  let position = $state({ bottom: 0, left: 0 });
  let activeIndex = $state(0);
  /** Read again when the cooldown ends, so the sounds light up at that moment. */
  let now = $state(Date.now());
  let announcement = $state("");

  const guild = $derived(client.voiceChannel?.guild ?? null);
  const sounds = $derived(guild?.sounds ?? []);
  const block = $derived(client.soundboardBlock(now));
  const remainingCooldown = $derived(Math.max(0, client.soundCooldownEndsAt - now));
  const joinSound = $derived(sounds.find((sound) => sound.id === guild?.my_join_sound_id) ?? null);
  const canChooseJoinSound = $derived(guild !== null && holds(guild, Permission.USE_SOUNDBOARD));
  const gain = $derived(soundboardGain(client.voice.settings.outputVolumePercent, client.voice.settings.soundboardVolumePercent));

  $effect(() => {
    if (remainingCooldown <= 0) {
      return;
    }
    const timer = setTimeout(() => (now = Date.now()), remainingCooldown);
    return () => clearTimeout(timer);
  });

  // Leaving the channel, or the server losing its last sound under the reader, keeps the panel
  // honest: the list empties or the panel closes with the voice panel.
  $effect(() => {
    if (activeIndex >= sounds.length) {
      activeIndex = Math.max(0, sounds.length - 1);
    }
  });

  onMount(() => {
    panel.showPopover();
    place();
    void tick().then(() => (cells()[activeIndex] ?? panel.querySelector<HTMLElement>("[data-initial-focus]"))?.focus());
  });

  /** Above its button, its leading edge on the button's, inside the window. */
  function place() {
    if (isCompact) {
      return;
    }
    const anchorBounds = anchor.getBoundingClientRect();
    const left = Math.min(Math.max(anchorBounds.left, 0), window.innerWidth - panel.offsetWidth);
    position = { bottom: Math.max(window.innerHeight - anchorBounds.top, 0), left: Math.max(left, 0) };
  }

  function cells(): HTMLButtonElement[] {
    return grid ? [...grid.querySelectorAll<HTMLButtonElement>("[data-sound-cell]")] : [];
  }

  function play(sound: SoundboardSound) {
    now = Date.now();
    if (client.playSound(sound.id, now)) {
      now = Date.now();
    }
  }

  async function preview(sound: SoundboardSound) {
    await previewSound(sound.id, gain);
  }

  function handleGridKeydown(event: KeyboardEvent) {
    const current = cells().indexOf(document.activeElement as HTMLButtonElement);
    if (current === -1) {
      return;
    }
    const move = nextEmojiCell([sounds.length], COLUMNS, current, { key: event.key, isControlHeld: event.ctrlKey || event.metaKey });
    if (typeof move !== "number") {
      return;
    }
    event.preventDefault();
    activeIndex = move;
    cells()[move]?.focus();
  }

  function openSoundMenu(sound: SoundboardSound, origin: MenuOrigin, invoker: HTMLElement): boolean {
    menuSound = sound;
    menu?.open(origin, invoker);
    return true;
  }

  const menuEntries = $derived.by((): ContextMenuEntry[] => {
    const sound = menuSound;
    if (!sound || !guild) {
      return [];
    }
    const isJoinSound = guild.my_join_sound_id === sound.id;
    const guildId = guild.id;
    const entries: ContextMenuEntry[] = [{ label: strings.soundboard.preview, icon: "headphones", onselect: () => void preview(sound) }];
    if (canChooseJoinSound) {
      entries.push({
        label: isJoinSound ? strings.soundboard.clearJoinSound : strings.soundboard.setJoinSound,
        icon: isJoinSound ? "x" : "speaker-high",
        onselectAndWait: async () => {
          try {
            await client.setJoinSound(guildId, isJoinSound ? null : sound.id);
            const sentence = isJoinSound ? strings.soundboard.joinSoundCleared : strings.soundboard.joinSoundSet(sound.name);
            announcement = sentence;
            return { announcement: sentence };
          } catch (error) {
            return { error: describeError(error, strings.soundboard.joinSoundFailed) };
          }
        },
      });
    }
    return entries;
  });
</script>

<div
  bind:this={panel}
  popover="auto"
  role="dialog"
  aria-labelledby="{id}-title"
  class={[
    "fixed m-0 flex-col gap-(--mw-component-emoji-picker-gap) border-(length:--mw-component-emoji-picker-border-width) border-(--mw-component-emoji-picker-border) bg-(--mw-component-emoji-picker-background) p-(--mw-component-emoji-picker-padding) shadow-(--mw-component-emoji-picker-shadow) [&:popover-open]:flex",
    isCompact
      ? "inset-x-0 top-auto bottom-0 max-h-[70dvh] w-full max-w-none rounded-t-(--mw-component-emoji-picker-radius) pb-[calc(var(--mw-component-emoji-picker-padding)+env(safe-area-inset-bottom))] [&:popover-open]:animate-[mw-rise_var(--mw-motion-duration-base)_var(--mw-motion-easing-enter)_both]"
      : "inset-auto max-h-[calc(100dvh-2*var(--mw-space-xs))] w-(--mw-component-emoji-picker-width) max-w-[calc(100vw-2*var(--mw-space-xs))] rounded-(--mw-component-emoji-picker-radius) [&:popover-open]:animate-[mw-lift-in_var(--mw-motion-duration-fast)_var(--mw-motion-easing-enter)_both]",
  ]}
  style:bottom={isCompact ? undefined : `${position.bottom}px`}
  style:left={isCompact ? undefined : `${position.left}px`}
  ontoggle={(event) => {
    if (event.newState === "closed") {
      onclose(panel.contains(document.activeElement) || document.activeElement === document.body);
    }
  }}
>
  <div class="flex min-w-0 flex-col gap-3xs px-2xs">
    <h2 id="{id}-title" class="truncate type-category text-(--mw-component-emoji-picker-heading-text)">
      {guild ? strings.soundboard.sounds(guild.name) : strings.soundboard.soundboard}
    </h2>
    {#if joinSound}
      <!-- Named here in words; its tile also carries an outline. -->
      <p class="truncate type-body-sm text-(--mw-component-emoji-picker-empty-text)">{strings.soundboard.yourJoinSound(joinSound.name)}</p>
    {/if}
  </div>

  {#if sounds.length === 0}
    <div class="flex flex-col items-start gap-xs px-2xs pb-2xs">
      <p class="type-body text-(--mw-component-emoji-picker-empty-text)">
        {guild ? strings.soundboard.empty(guild.name) : ""}
        {onaddsounds ? strings.soundboard.emptyCanAdd : strings.soundboard.emptyAsk}
      </p>
      {#if onaddsounds}
        <Button variant="primary" data-initial-focus onclick={onaddsounds}>{strings.soundboard.addSounds}</Button>
      {/if}
    </div>
  {:else}
    <p id="{id}-help" class="sr-only">{strings.soundboard.help}</p>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      bind:this={grid}
      class="grid min-h-0 grid-cols-3 gap-2xs overflow-y-auto [scrollbar-width:thin]"
      onkeydown={handleGridKeydown}
    >
      {#each sounds as sound, index (sound.id)}
        {@const isJoinSound = guild?.my_join_sound_id === sound.id}
        <div class="group/sound relative min-w-0">
          <button
            type="button"
            data-sound-cell
            tabindex={index === activeIndex ? 0 : -1}
            aria-label={strings.soundboard.soundLabel(sound.name, isJoinSound)}
            aria-disabled={block !== null || undefined}
            aria-describedby={block ? `${id}-reason ${id}-help` : `${id}-help`}
            aria-haspopup="menu"
            class={[
              "flex h-full min-h-(--mw-size-hit-web-min) w-full min-w-0 items-center gap-2xs rounded-(--mw-component-emoji-picker-cell-radius) border-(length:--mw-component-text-field-border-width) bg-(--mw-component-custom-emoji-settings-picture-background) px-xs py-2xs text-start pointer-coarse:pe-(--mw-size-hit-web-min)",
              isJoinSound ? "border-(--mw-component-custom-emoji-settings-name-border-hover)" : "border-transparent",
              " transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) focus-visible:focus-ring",
              block === null
                ? "hover:bg-(--mw-component-emoji-picker-cell-background-hover)"
                : "cursor-default opacity-(--mw-component-button-disabled-opacity)",
            ]}
            onfocus={() => (activeIndex = index)}
            onclick={() => play(sound)}
            {@attach contextMenuTrigger((origin, invoker) => openSoundMenu(sound, origin, invoker))}
          >
            <SoundEmoji
              emojiKey={sound.emoji}
              customEmojis={client.knownCustomEmojis}
              sizeToken="--mw-component-emoji-picker-emoji-size"
              glyphClass="text-(--mw-component-emoji-picker-category-icon)"
            />
            <span class="line-clamp-2 min-w-0 type-body-sm break-words hyphens-auto text-text-normal">{sound.name}</span>
          </button>
          <!-- Pointer shortcut to the menu's first item; keyboards reach it through the menu. -->
          <IconButton
            icon="headphones"
            label={strings.soundboard.previewLabel(sound.name)}
            tabindex={-1}
            class="invisible absolute inset-y-0 end-0 my-auto size-(--mw-size-hit-web-min) rounded-(--mw-component-emoji-picker-cell-radius) text-(--mw-component-emoji-picker-category-icon) group-hover/sound:visible hover:text-(--mw-component-emoji-picker-category-icon-hover) pointer-coarse:visible"
            iconClass="size-(--mw-component-emoji-picker-category-icon-size)"
            onclick={() => void preview(sound)}
          />
        </div>
      {/each}
    </div>
    <!-- One reason for every dimmed sound, else the hint; the bar drains through the cooldown
         (it stays still under reduced motion). Both keep their room, so the grid never jumps. -->
    <div class="flex flex-col gap-2xs px-2xs">
      {#key client.soundCooldownEndsAt}
        <div
          aria-hidden="true"
          class={[
            "h-(--mw-component-voice-meter-height) overflow-hidden rounded-(--mw-component-voice-meter-radius) bg-(--mw-component-voice-meter-track)",
            block !== "cooling-down" && "invisible",
          ]}
        >
          {#if block === "cooling-down"}
            <div
              class="cooldown-fill h-full origin-left rounded-(--mw-component-voice-meter-radius) bg-(--mw-component-voice-meter-level-heard) motion-reduce:animate-none rtl:origin-right"
              style:animation-duration="{remainingCooldown}ms"
              style:transform="scaleX({remainingCooldown / MINIMUM_MILLISECONDS_BETWEEN_SOUNDS})"
            ></div>
          {/if}
        </div>
      {/key}
      <p id="{id}-reason" class={["type-body-sm text-(--mw-component-emoji-picker-empty-text)", !block && "pointer-coarse:invisible"]}>
        {block ? strings.soundboard.blocks[block] : strings.soundboard.hint}
      </p>
    </div>
  {/if}

  <ContextMenu
    bind:this={menu}
    label={menuSound ? strings.soundboard.soundOptions(menuSound.name) : strings.soundboard.soundboard}
    entries={menuEntries}
  />
  <p class="sr-only" aria-live="polite">{announcement}</p>
</div>

<style>
  /* The 2 s between two sounds, draining once from what is left (it starts part-way after the echo). */
  .cooldown-fill {
    animation-name: cooldown-drain;
    animation-timing-function: linear;
    animation-fill-mode: forwards;
  }
  @keyframes cooldown-drain {
    to {
      transform: scaleX(0);
    }
  }
</style>

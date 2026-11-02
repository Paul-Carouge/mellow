// The soundboard's client side (ADR-0032): the sounds' indicators by people's names, your own
// cooldown, the plays the gateway announces, and each server's sound list. The sound lists live
// in the guild snapshots `MellowClient` owns, so this slice reads and changes them through ports.

import { SvelteMap } from "svelte/reactivity";

import type { RestClient } from "./api";
import type { Channel, ClientFrame, Guild, PermissionBits, Snowflake, SoundboardSound, VoiceState } from "./protocol";
import {
  type SoundboardBlock,
  cooldownEndAfter,
  joinCueOf,
  shouldHearSound,
  soundIndicatorMilliseconds,
  soundboardBlock,
} from "./soundboard";
import type { VoiceCue, VoiceSession } from "./voice-session.svelte";

/** What the soundboard reads from, and changes in, the rest of the client. */
export interface SoundboardPorts {
  readonly rest: Pick<RestClient, "createSound" | "updateSound" | "deleteSound" | "setJoinSound">;
  readonly voice: Pick<VoiceSession, "phase" | "channelId" | "toggles" | "personSetting">;
  /** `null` while signed out. */
  currentUserId(): Snowflake | null;
  /** The channel `voice` is in or joining, and its server. */
  voiceChannel(): { guild: Guild; channel: Channel } | null;
  voiceStateOf(userId: Snowflake): VoiceState | undefined;
  channelPermissions(channelId: Snowflake): PermissionBits | null;
  /** Whether it left: false while the gateway is not connected. */
  sendFrame(frame: ClientFrame): boolean;
  guilds(): readonly Guild[];
  updateGuild(guildId: Snowflake, change: (guild: Guild) => Guild): void;
}

/** The sound a person played last, while its emoji shows by their name. */
export interface SoundIndicator {
  soundId: Snowflake;
  guildId: Snowflake;
}

export class SoundboardSession {
  /** The sound each person in your channel played last, while its emoji shows by their name. */
  readonly indicators = new SvelteMap<Snowflake, SoundIndicator>();
  /** Your soundboard waits until then between two sounds (`MINIMUM_MILLISECONDS_BETWEEN_SOUNDS`). */
  cooldownEndsAt = $state(0);
  /**
   * Plays one of a server's sounds on this device (set by the shell), for `playerId` (whose
   * deafen-free, unmuted sounds they are); `fallbackCue` plays instead when the sound cannot load,
   * for a join sound standing in for the chime.
   */
  onSound: ((soundId: Snowflake, playerId: Snowflake, fallbackCue: VoiceCue | null) => void) | null = null;

  constructor(private readonly ports: SoundboardPorts) {}

  /** At sign-out; the cooldown stays, as the instance keeps its own. */
  clearIndicators(): void {
    this.indicators.clear();
  }

  /**
   * Starts your own join sound in the channel you just connected to: shows its emoji by your name
   * and returns the sound to play instead of your join chime, or `null` for the chime.
   */
  startOwnJoinSound(): Snowflake | null {
    const place = this.ports.voiceChannel();
    const userId = this.ports.currentUserId();
    const own = userId === null ? undefined : this.ports.voiceStateOf(userId);
    if (!place || own?.channel_id !== place.channel.id) {
      return null;
    }
    const cue = joinCueOf(own.join_sound_id, place.guild.sounds ?? [], {
      isDeafened: this.ports.voice.toggles.isDeafened,
      isMutedForMe: false,
    });
    if (cue.kind === "chime") {
      return null;
    }
    this.showIndicator(own.user_id, place.guild.id, cue.sound);
    return cue.sound.id;
  }

  /**
   * Someone else joined your channel in `guild`: plays their join sound and returns true, or
   * returns false for the caller's soft chime.
   */
  playPeerJoinSound(guild: Guild, userId: Snowflake, joinSoundId: Snowflake | null | undefined): boolean {
    const cue = joinCueOf(joinSoundId, guild.sounds ?? [], {
      isDeafened: this.ports.voice.toggles.isDeafened,
      isMutedForMe: this.ports.voice.personSetting(userId).isLocallyMuted,
    });
    if (cue.kind !== "sound" || !this.onSound) {
      return false;
    }
    this.showIndicator(userId, guild.id, cue.sound);
    this.onSound(cue.sound.id, userId, "peer-join");
    return true;
  }

  /** Why the soundboard cannot play in your channel now, or `null`. */
  block(now: number): SoundboardBlock | null {
    const channelId = this.ports.voice.channelId;
    return soundboardBlock({
      isConnected: this.ports.voice.phase === "connected" && channelId !== null,
      channelPermissions: channelId === null ? null : this.ports.channelPermissions(channelId),
      isDeafened: this.ports.voice.toggles.isDeafened,
      now,
      cooldownEndsAt: this.cooldownEndsAt,
    });
  }

  /** Asks the instance to play a sound to your channel; it answers with `VOICE_SOUND_PLAYED`. */
  playSound(soundId: Snowflake, now: number): boolean {
    if (this.block(now) !== null) {
      return false;
    }
    const isSent = this.ports.sendFrame({ op: "voice_play_sound", d: { sound_id: soundId } });
    if (isSent) {
      this.cooldownEndsAt = cooldownEndAfter(this.cooldownEndsAt, now);
    }
    return isSent;
  }

  /** `VOICE_SOUND_PLAYED`: shows the sound by its player's name, and plays it where it is heard. */
  applySoundPlayed(channelId: Snowflake, userId: Snowflake, soundId: Snowflake, now: number): void {
    const place = this.ports.voiceChannel();
    const isConnected = this.ports.voice.phase === "connected";
    if (!place || place.channel.id !== channelId || !isConnected) {
      return;
    }
    const isMine = userId === this.ports.currentUserId();
    if (isMine) {
      this.cooldownEndsAt = cooldownEndAfter(this.cooldownEndsAt, now);
    }
    const sound = place.guild.sounds?.find((candidate) => candidate.id === soundId);
    this.showIndicator(userId, place.guild.id, sound ?? { id: soundId, name: "", duration_milliseconds: 0 });
    const isHeard = shouldHearSound({
      soundChannelId: channelId,
      myChannelId: place.channel.id,
      isConnected,
      isDeafened: this.ports.voice.toggles.isDeafened,
      isPlayerMutedForMe: !isMine && this.ports.voice.personSetting(userId).isLocallyMuted,
    });
    if (isHeard) {
      this.onSound?.(soundId, userId, null);
    }
  }

  /** The sound's emoji by the player's name, for the sound's length (at least 1.5 s). */
  private showIndicator(userId: Snowflake, guildId: Snowflake, sound: SoundboardSound): void {
    const indicator = { soundId: sound.id, guildId };
    this.indicators.set(userId, indicator);
    setTimeout(() => {
      if (this.indicators.get(userId) === indicator) {
        this.indicators.delete(userId);
      }
    }, soundIndicatorMilliseconds(sound));
  }

  /** The server's sounds, oldest first; none from an older instance. */
  soundsOf(guildId: Snowflake): SoundboardSound[] {
    return this.ports.guilds().find((guild) => guild.id === guildId)?.sounds ?? [];
  }

  private applySounds(guildId: Snowflake, sounds: SoundboardSound[]): void {
    this.ports.updateGuild(guildId, (guild) => ({ ...guild, sounds }));
  }

  async createSound(guildId: Snowflake, name: string, emoji: string | null, wav: Blob): Promise<SoundboardSound> {
    const created = await this.ports.rest.createSound(guildId, name, emoji, wav);
    const sounds = this.soundsOf(guildId);
    // The gateway's list may have come first.
    if (!sounds.some((sound) => sound.id === created.id)) {
      this.applySounds(guildId, [...sounds, created]);
    }
    return created;
  }

  /** Edited from a dialog that closes on success, so the answer shows at once. */
  async updateSound(guildId: Snowflake, soundId: Snowflake, name: string, emoji: string | null): Promise<void> {
    const updated = await this.ports.rest.updateSound(guildId, soundId, name, emoji);
    this.applySounds(
      guildId,
      this.soundsOf(guildId).map((sound) => (sound.id === updated.id ? updated : sound)),
    );
  }

  async deleteSound(guildId: Snowflake, soundId: Snowflake): Promise<void> {
    await this.ports.rest.deleteSound(guildId, soundId);
    this.applySounds(
      guildId,
      this.soundsOf(guildId).filter((sound) => sound.id !== soundId),
    );
    // The instance clears join sounds pointing at it; its event may come later.
    this.ports.updateGuild(guildId, (guild) =>
      guild.my_join_sound_id === soundId ? { ...guild, my_join_sound_id: null } : guild,
    );
  }

  /** `null` removes it. */
  async setJoinSound(guildId: Snowflake, soundId: Snowflake | null): Promise<void> {
    await this.ports.rest.setJoinSound(guildId, soundId);
    this.ports.updateGuild(guildId, (guild) => ({ ...guild, my_join_sound_id: soundId }));
  }
}

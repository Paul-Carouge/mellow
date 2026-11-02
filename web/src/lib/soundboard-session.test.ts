import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type ClientFrame, type Guild, Permission, type SoundboardSound, type VoiceState } from "./protocol";
import { MINIMUM_SOUND_INDICATOR_MILLISECONDS } from "./soundboard";
import { type SoundboardPorts, SoundboardSession } from "./soundboard-session.svelte";
import { DEFAULT_PERSON_VOICE_SETTING } from "./voice-settings";
import { UNMUTED_VOICE_TOGGLES } from "./voice-toggles";

const ME = "1";
const OTHER = "2";
const GUILD = "100";
const LOUNGE = "104";
const HORN: SoundboardSound = { id: "900", name: "horn", emoji: "📯", duration_milliseconds: 800 };
const LONG_HORN: SoundboardSound = { id: "901", name: "long horn", duration_milliseconds: 4_000 };
const ALL_SOUNDBOARD_PERMISSIONS = Permission.SPEAK | Permission.USE_SOUNDBOARD;

function guild(): Guild {
  return {
    id: GUILD,
    name: "Club",
    owner_id: OTHER,
    my_permissions: String(ALL_SOUNDBOARD_PERMISSIONS),
    my_role_ids: [],
    roles: [],
    channels: [
      { id: LOUNGE, guild_id: GUILD, kind: "voice", name: "lounge", topic: "", parent_id: null, position: 0, permission_overwrites: [] },
    ],
    sounds: [HORN, LONG_HORN],
    my_join_sound_id: HORN.id,
  };
}

/** A soundboard connected in `LOUNGE`, its ports as plain fields the test changes. */
function connectedSoundboard() {
  const sentFrames: ClientFrame[] = [];
  const played: [string, string, string | null][] = [];
  const world = {
    guilds: [guild()],
    voice: { phase: "connected" as const, channelId: LOUNGE as string | null, toggles: UNMUTED_VOICE_TOGGLES, mutedUserIds: new Set<string>() },
    permissions: ALL_SOUNDBOARD_PERMISSIONS as bigint | null,
    voiceStates: new Map<string, VoiceState>(),
    isGatewayConnected: true,
    deletedSoundIds: [] as string[],
  };
  const ports: SoundboardPorts = {
    rest: {
      createSound: () => Promise.reject(new Error("unexpected createSound")),
      updateSound: () => Promise.reject(new Error("unexpected updateSound")),
      deleteSound: (_guildId, soundId) => Promise.resolve(void world.deletedSoundIds.push(soundId)),
      setJoinSound: () => Promise.resolve(),
    },
    voice: {
      get phase() {
        return world.voice.phase;
      },
      get channelId() {
        return world.voice.channelId;
      },
      get toggles() {
        return world.voice.toggles;
      },
      personSetting: (userId) => ({ ...DEFAULT_PERSON_VOICE_SETTING, isLocallyMuted: world.voice.mutedUserIds.has(userId) }),
    },
    currentUserId: () => ME,
    voiceChannel: () => {
      const [first] = world.guilds;
      const channel = first?.channels.find((candidate) => candidate.id === world.voice.channelId);
      return first && channel ? { guild: first, channel } : null;
    },
    voiceStateOf: (userId) => world.voiceStates.get(userId),
    channelPermissions: () => (world.permissions === null ? null : String(world.permissions)),
    sendFrame: (frame) => world.isGatewayConnected && sentFrames.push(frame) > 0,
    guilds: () => world.guilds,
    updateGuild: (guildId, change) => {
      world.guilds = world.guilds.map((candidate) => (candidate.id === guildId ? change(candidate) : candidate));
    },
  };
  const soundboard = new SoundboardSession(ports);
  soundboard.onSound = (soundId, playerId, fallbackCue) => played.push([soundId, playerId, fallbackCue]);
  return { soundboard, world, sentFrames, played };
}

describe("SoundboardSession", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("sends a sound, then cools down until the echo's time has passed", () => {
    const { soundboard, sentFrames } = connectedSoundboard();
    expect(soundboard.playSound(HORN.id, 1_000)).toBe(true);
    expect(sentFrames).toEqual([{ op: "voice_play_sound", d: { sound_id: HORN.id } }]);
    expect(soundboard.block(1_000)).toBe("cooling-down");
    expect(soundboard.playSound(HORN.id, 1_500)).toBe(false);
    soundboard.applySoundPlayed(LOUNGE, ME, HORN.id, 2_000);
    expect(soundboard.block(3_500)).toBe("cooling-down");
    expect(soundboard.block(4_000)).toBeNull();
    expect(sentFrames).toHaveLength(1);
  });

  it("keeps no cooldown when the frame could not leave", () => {
    const { soundboard, world } = connectedSoundboard();
    world.isGatewayConnected = false;
    expect(soundboard.playSound(HORN.id, 0)).toBe(false);
    expect(soundboard.block(0)).toBeNull();
  });

  it("says why it cannot play, in the order a person can act on it", () => {
    const { soundboard, world } = connectedSoundboard();
    world.permissions = Permission.SPEAK;
    expect(soundboard.block(0)).toBe("no-permission");
    world.permissions = null;
    world.voice.toggles = { ...UNMUTED_VOICE_TOGGLES, isDeafened: true };
    expect(soundboard.block(0)).toBe("deafened");
    world.voice.channelId = null;
    expect(soundboard.block(0)).toBe("not-connected");
  });

  it("shows a played sound by its player for the sound's length, and plays it unless muted for you", () => {
    const { soundboard, world, played } = connectedSoundboard();
    world.voice.mutedUserIds.add(OTHER);
    soundboard.applySoundPlayed(LOUNGE, OTHER, LONG_HORN.id, 0);
    expect(soundboard.indicators.get(OTHER)).toEqual({ soundId: LONG_HORN.id, guildId: GUILD });
    expect(played).toEqual([]);
    vi.advanceTimersByTime(LONG_HORN.duration_milliseconds - 1);
    expect(soundboard.indicators.has(OTHER)).toBe(true);
    vi.advanceTimersByTime(1);
    expect(soundboard.indicators.has(OTHER)).toBe(false);

    soundboard.applySoundPlayed(LOUNGE, ME, HORN.id, 0);
    expect(played).toEqual([[HORN.id, ME, null]]);
    vi.advanceTimersByTime(MINIMUM_SOUND_INDICATOR_MILLISECONDS);
    expect(soundboard.indicators.size).toBe(0);
  });

  it("ignores sounds played in another channel", () => {
    const { soundboard, played } = connectedSoundboard();
    soundboard.applySoundPlayed("999", OTHER, HORN.id, 0);
    expect(soundboard.indicators.size).toBe(0);
    expect(played).toEqual([]);
  });

  it("keeps a later sound's indicator when an earlier one's time runs out", () => {
    const { soundboard } = connectedSoundboard();
    soundboard.applySoundPlayed(LOUNGE, OTHER, HORN.id, 0);
    vi.advanceTimersByTime(1_000);
    soundboard.applySoundPlayed(LOUNGE, OTHER, LONG_HORN.id, 1_000);
    vi.advanceTimersByTime(MINIMUM_SOUND_INDICATOR_MILLISECONDS);
    expect(soundboard.indicators.get(OTHER)?.soundId).toBe(LONG_HORN.id);
  });

  it("plays a peer's join sound, or leaves the chime to the caller", () => {
    const { soundboard, world, played } = connectedSoundboard();
    expect(soundboard.playPeerJoinSound(world.guilds[0]!, OTHER, HORN.id)).toBe(true);
    expect(played).toEqual([[HORN.id, OTHER, "peer-join"]]);
    expect(soundboard.playPeerJoinSound(world.guilds[0]!, OTHER, null)).toBe(false);
    soundboard.onSound = null;
    expect(soundboard.playPeerJoinSound(world.guilds[0]!, OTHER, HORN.id)).toBe(false);
  });

  it("starts your own join sound only once your state is in your channel", () => {
    const { soundboard, world } = connectedSoundboard();
    expect(soundboard.startOwnJoinSound()).toBeNull();
    world.voiceStates.set(ME, {
      user_id: ME,
      guild_id: GUILD,
      channel_id: LOUNGE,
      self_mute: false,
      self_deaf: false,
      suppressed: false,
      join_sound_id: HORN.id,
    });
    expect(soundboard.startOwnJoinSound()).toBe(HORN.id);
    expect(soundboard.indicators.get(ME)?.soundId).toBe(HORN.id);
  });

  it("drops a deleted sound from the list and from your join sound", async () => {
    const { soundboard, world } = connectedSoundboard();
    await soundboard.deleteSound(GUILD, HORN.id);
    expect(world.deletedSoundIds).toEqual([HORN.id]);
    expect(soundboard.soundsOf(GUILD)).toEqual([LONG_HORN]);
    expect(world.guilds[0]?.my_join_sound_id).toBeNull();
  });
});

// What this tab plays for voice (docs/design/web-voice-channels.md) and the soundboard
// (docs/design/web-soundboard.md): cues, and sounds and join sounds at the output volume times
// the soundboard volume; a join sound that cannot load gives way to its chime. Deafening stops
// every sound, and muting someone for you stops theirs (ADR-0032), including one still
// downloading when you did. Everything goes to the output chosen for voices.

import { onMount } from "svelte";

import type { MellowClient } from "../lib/client.svelte";
import type { Snowflake } from "../lib/protocol";
import { soundboardGain } from "../lib/soundboard";
import type { VoiceCue } from "../lib/voice-session.svelte";
import { routeChimeSoundTo } from "./chime-sound";
import { forgetSoundsExcept, playSoundOnThisDevice, stopPlayingSounds } from "./soundboard-audio";
import { playVoiceCue } from "./voice-sounds";

/**
 * Plays the client's cues and sounds while the calling component is mounted; call it during its
 * initialisation. `currentClient` is read each time, as a component prop would be.
 */
export function playVoiceSoundsWhileMounted(currentClient: () => MellowClient): void {
  function isPlayerHeard(playerId: Snowflake): boolean {
    const client = currentClient();
    return !client.voice.toggles.isDeafened && (playerId === client.user?.id || !client.voice.personSetting(playerId).isLocallyMuted);
  }
  function playSound(soundId: Snowflake, playerId: Snowflake, fallbackCue: VoiceCue | null) {
    const { outputVolumePercent, soundboardVolumePercent } = currentClient().voice.settings;
    const gain = soundboardGain(outputVolumePercent, soundboardVolumePercent);
    void playSoundOnThisDevice(soundId, gain, playerId, () => isPlayerHeard(playerId)).then((playing) => {
      if (playing === "failed" && fallbackCue !== null) {
        playVoiceCue(fallbackCue);
      }
    });
  }
  $effect(() => {
    if (currentClient().voice.toggles.isDeafened) {
      stopPlayingSounds(() => true);
    }
  });
  $effect(() => {
    const mutedForMe = new Set(
      [...currentClient().voice.personSettings].filter(([, setting]) => setting.isLocallyMuted).map(([userId]) => userId),
    );
    if (mutedForMe.size > 0) {
      stopPlayingSounds((playerId) => playerId !== null && mutedForMe.has(playerId));
    }
  });
  function playCue(cue: VoiceCue) {
    const client = currentClient();
    const ownJoinSoundId = cue === "self-join" ? client.startOwnJoinSound() : null;
    if (ownJoinSoundId !== null && client.user) {
      playSound(ownJoinSoundId, client.user.id, cue);
    } else {
      playVoiceCue(cue);
    }
  }
  $effect(() =>
    forgetSoundsExcept(new Set(currentClient().guilds.flatMap((candidate) => (candidate.sounds ?? []).map((sound) => sound.id)))),
  );
  onMount(() => {
    currentClient().voice.onCue = playCue;
    currentClient().onSound = playSound;
    return () => {
      currentClient().voice.onCue = null;
      currentClient().onSound = null;
    };
  });
  $effect(() => {
    const { voice } = currentClient();
    routeChimeSoundTo(voice.isOutputDeviceMissing ? null : voice.settings.outputDeviceId);
  });
}

// What a call does beyond the screen (DESIGN.md §5.6.3, ADR-0062): the ringtone looped while a
// call rings the reader (Christmas's in December), a short cue instead when they are already in a
// call, nothing under Do not disturb; the caller's waiting tone; and a browser notification while
// the window is in the background. Every sound stops when the call stops ringing them, and at its
// 30 seconds at the latest, even without an event. Audio starts only once a gesture allowed it.

import { onMount, untrack } from "svelte";

import { strings } from "../i18n";
import { callSound } from "../lib/calls";
import type { MellowClient } from "../lib/client.svelte";
import type { Call } from "../lib/protocol";
import { ringtoneFor, ringtoneUrl } from "../lib/ringtones";
import type { StopSound } from "./call-audio";
import { incomingCallSentence, ringingEndsAt } from "./calls-view";
import { prepareChimeSound, runningAudioContext } from "./chime-sound";
import { notificationSettings } from "./notification-preferences.svelte";
import { notificationSupport } from "./notifier.svelte";

const NOTIFICATION_ICON_URL = "/icons/mellow-192.png";
/** Events that count as a gesture for audio, listened to in the capture phase (as the notifier does). */
const AUDIO_UNLOCKING_GESTURES = ["pointerup", "keydown"] as const;

/** The sound wanted now, keyed so a call's other changes do not restart it. */
interface WantedSound {
  readonly key: string;
  readonly endsAt: number;
  readonly start: (audio: CallAudio) => StopSound | null;
}

type CallAudio = typeof import("./call-audio");
/** The players load with the first call that sounds, not with the app. */
const loadCallAudio = (): Promise<CallAudio> => import("./call-audio");

/**
 * Plays and shows call alerts while the calling component is mounted; call it during its
 * initialisation. `currentClient` is read each time, as a component prop would be.
 */
export function alertCallsWhileMounted(currentClient: () => MellowClient): void {
  // Audio may start only after a gesture: the effects below run again once it may.
  let isAudioAllowed = $state(runningAudioContext() !== null);
  onMount(() => {
    const allowAudio = () => {
      void prepareChimeSound().then(() => (isAudioAllowed = runningAudioContext() !== null));
    };
    for (const gesture of AUDIO_UNLOCKING_GESTURES) {
      window.addEventListener(gesture, allowAudio, { capture: true });
    }
    return () => {
      for (const gesture of AUDIO_UNLOCKING_GESTURES) {
        window.removeEventListener(gesture, allowAudio, { capture: true });
      }
    };
  });

  const ringingCall = $derived(currentClient().incomingCalls[0] ?? null);
  const isDoNotDisturb = $derived(currentClient().user?.status === "do_not_disturb");
  const outputVolume = $derived(currentClient().voice.settings.outputVolumePercent / 100);

  const incomingSound = $derived.by((): WantedSound | null => {
    if (ringingCall === null) {
      return null;
    }
    const client = currentClient();
    const sound = callSound("incoming", client.isInACallOrVoiceChannel, isDoNotDisturb);
    const endsAt = ringingEndsAt(ringingCall);
    // One sound per ringing call, chosen when it starts ringing: answering puts the reader in a
    // call a moment before the call stops ringing them, which must not play the call-waiting cue.
    // Do not disturb turned on while it rings silences it.
    const callKey = `${ringingCall.channel_id}:${ringingCall.message_id}:${isDoNotDisturb}`;
    if (sound === "ringtone") {
      const url = ringtoneUrl(ringtoneFor(new Date()));
      return { key: callKey, endsAt, start: (audio) => audio.startRingtone(url, outputVolume) };
    }
    if (sound === "call-waiting") {
      // Once per call: it is a cue, not a ring.
      return {
        key: callKey,
        endsAt,
        start: (audio) => {
          audio.playCallWaitingCue();
          return () => {};
        },
      };
    }
    return null;
  });

  const ownCall = $derived.by((): Call | null => {
    const client = currentClient();
    const channelId = client.voice.channelId;
    return channelId !== null && client.voice.phase !== "idle" && client.callPhaseOf(channelId) === "outgoing"
      ? (client.callsByChannelId.get(channelId) ?? null)
      : null;
  });
  const ringbackSound = $derived.by((): WantedSound | null =>
    ownCall === null
      ? null
      : { key: `ringback:${ownCall.channel_id}:${ownCall.message_id}`, endsAt: ringingEndsAt(ownCall), start: (audio) => audio.startRingback(outputVolume) },
  );

  playWhileWanted(() => incomingSound, () => isAudioAllowed);
  playWhileWanted(() => ringbackSound, () => isAudioAllowed);

  // The browser notification, while the window is in the background (docs/design/web-notifications.md).
  const notifiedKey = $derived(ringingCall ? `${ringingCall.channel_id}:${ringingCall.message_id}` : null);
  $effect(() => {
    if (notifiedKey === null || isDoNotDisturb) {
      return;
    }
    // Shown once per ringing call: its other changes do not show it again.
    const call = untrack(() => ringingCall);
    const client = untrack(currentClient);
    const conversation = call ? untrack(() => client.conversations.find((candidate) => candidate.id === call.channel_id)) : undefined;
    const isAway = document.visibilityState !== "visible" || !document.hasFocus();
    const isAllowed = untrack(() => notificationSettings().areDesktopNotificationsEnabled && notificationSupport() === "granted");
    if (!call || !conversation || !isAway || !isAllowed) {
      return;
    }
    let notification: Notification;
    try {
      notification = new Notification(incomingCallSentence(call, conversation, untrack(() => client.user?.id ?? null)), {
        body: strings.calls.notificationBody,
        tag: `mellow-call-${call.channel_id}`,
        icon: NOTIFICATION_ICON_URL,
        requireInteraction: true,
        // The ringtone is ours.
        silent: true,
      });
    } catch (error) {
      console.warn("couldn't show the call's notification", error);
      return;
    }
    notification.onclick = () => {
      window.focus();
      client.selectGuild(null, call.channel_id);
      notification.close();
    };
    return () => notification.close();
  });
}

/** Starts the wanted sound, stops it when it changes, ends, or the component goes. */
function playWhileWanted(wanted: () => WantedSound | null, isAudioAllowed: () => boolean): void {
  const key = $derived(wanted()?.key ?? null);
  $effect(() => {
    if (key === null || !isAudioAllowed()) {
      return;
    }
    // Only a new key restarts it: a call's other changes (someone else answering) or a volume
    // change leave the ring as it is.
    const sound = untrack(wanted);
    const remaining = sound ? sound.endsAt - Date.now() : 0;
    if (!sound || remaining <= 0) {
      return;
    }
    let stop: StopSound | null = null;
    let isOver = false;
    const end = () => {
      isOver = true;
      stop?.();
    };
    loadCallAudio()
      .then((audio) => {
        if (!isOver) {
          stop = sound.start(audio);
        }
      })
      .catch((error: unknown) => console.warn("couldn't load the call sounds", error));
    const timer = setTimeout(end, remaining);
    return () => {
      clearTimeout(timer);
      end();
    };
  });
}

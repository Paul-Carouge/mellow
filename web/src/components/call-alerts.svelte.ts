// What a call does beyond the screen (DESIGN.md §5.6.3, ADR-0062): the ringtone looped while a
// call rings the reader (Christmas's in December), a short cue instead when they are already in a
// call, nothing under Do not disturb; the caller's waiting tone; and a browser notification while
// the window is in the background. Every sound stops when the call stops ringing them, and at its
// 30 seconds at the latest, even without an event. Audio starts only once a gesture allowed it.

import { onMount, untrack } from "svelte";

import { strings } from "../i18n";
import type { MellowClient } from "../lib/client.svelte";
import type { Call } from "../lib/protocol";
import { ringtoneFor, ringtoneUrl } from "../lib/ringtones";
import { type StopSound, playCallWaitingCue, startRingback, startRingtone } from "./call-audio";
import { type PlannedCallSound, plannedIncomingSound, plannedRingback } from "./call-alert-plan";
import { announceIncomingCall } from "./call-announcement.svelte";
import { incomingCallSentence } from "./calls-view";
import { prepareChimeSound, runningAudioContext } from "./chime-sound";
import { notificationSettings } from "./notification-preferences.svelte";
import { notificationSupport } from "./notifier.svelte";

const NOTIFICATION_ICON_URL = "/icons/mellow-192.png";
/** Events after which audio may have been allowed: the shell prepares it on the same gestures. */
const AUDIO_UNLOCKING_GESTURES = ["pointerup", "keydown"] as const;

/**
 * Plays and shows call alerts while the calling component is mounted (`CallAlerts.svelte`, loaded
 * only while a call rings the reader or holds them); call it during its initialisation.
 * `currentClient` is read each time, as a component prop would be.
 */
export function alertCallsWhileMounted(currentClient: () => MellowClient): void {
  // Audio may start only after a gesture (the shell prepares the context on each one): the sounds
  // below start once it runs.
  let isAudioAllowed = $state(runningAudioContext() !== null);
  onMount(() => {
    const noteGesture = () => void prepareChimeSound().then(() => (isAudioAllowed = runningAudioContext() !== null));
    for (const gesture of AUDIO_UNLOCKING_GESTURES) {
      window.addEventListener(gesture, noteGesture);
    }
    return () => {
      for (const gesture of AUDIO_UNLOCKING_GESTURES) {
        window.removeEventListener(gesture, noteGesture);
      }
      announceIncomingCall("");
    };
  });

  const ringingCall = $derived(currentClient().incomingCalls[0] ?? null);
  const isDoNotDisturb = $derived(currentClient().user?.status === "do_not_disturb");
  const outputVolume = $derived(currentClient().voice.settings.outputVolumePercent / 100);

  // Read out once per ringing call, in the shell's region.
  const ringingSentence = $derived.by(() => {
    const client = currentClient();
    const conversation = ringingCall ? client.conversations.find((candidate) => candidate.id === ringingCall.channel_id) : undefined;
    return ringingCall && conversation ? strings.calls.announcement(incomingCallSentence(ringingCall, conversation, client.user?.id ?? null)) : "";
  });
  $effect(() => announceIncomingCall(ringingSentence));

  const incomingPlan = $derived(
    plannedIncomingSound(ringingCall, {
      isInAnotherCallOrVoiceChannel: currentClient().isInACallOrVoiceChannel,
      isDoNotDisturb,
      now: Date.now(),
    }),
  );
  const ownRingingCall = $derived.by((): Call | null => {
    const client = currentClient();
    const channelId = client.voice.channelId;
    return channelId !== null && client.voice.phase !== "idle" && client.callPhaseOf(channelId) === "outgoing"
      ? (client.callsByChannelId.get(channelId) ?? null)
      : null;
  });
  const ringbackPlan = $derived(plannedRingback(ownRingingCall, Date.now()));

  function start(plan: PlannedCallSound): StopSound | null {
    switch (plan.kind) {
      case "ringtone":
        return startRingtone(ringtoneUrl(ringtoneFor(new Date(Date.now()))), outputVolume);
      case "call-waiting":
        // Once per call: a cue, not a ring.
        playCallWaitingCue();
        return () => {};
      case "ringback":
        return startRingback(outputVolume);
    }
  }
  playWhilePlanned(() => incomingPlan, () => isAudioAllowed, start);
  playWhilePlanned(() => ringbackPlan, () => isAudioAllowed, start);

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

/** Starts the planned sound, stops it when its key changes, its time is up, or the component goes. */
function playWhilePlanned(
  planned: () => PlannedCallSound | null,
  isAudioAllowed: () => boolean,
  start: (plan: PlannedCallSound) => StopSound | null,
): void {
  const key = $derived(planned()?.key ?? null);
  $effect(() => {
    if (key === null || !isAudioAllowed()) {
      return;
    }
    const plan = untrack(planned);
    const stop = plan ? untrack(() => start(plan)) : null;
    if (!plan || stop === null) {
      return;
    }
    const timer = setTimeout(stop, plan.durationMilliseconds);
    return () => {
      clearTimeout(timer);
      stop();
    };
  });
}

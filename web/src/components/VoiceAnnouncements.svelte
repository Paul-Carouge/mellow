<script lang="ts">
  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import { soundlessShareAdvice } from "../lib/screen-share";
  import { screenSoundPlatform } from "../lib/voice-browser";
  import { describeMissingDevices, describeSharedSound, screenSoundNoticeSentence } from "./voice-panel";

  // The voice panel's live regions, kept out of its lazily loaded chunk: they exist before voice
  // starts, so "Connecting to …" is read out even though the panel itself is still downloading.
  interface Props {
    client: MellowClient;
  }

  let { client }: Props = $props();

  const soundAdvice = soundlessShareAdvice(screenSoundPlatform().chromiumPlatform);
  const voice = $derived(client.voice);
  const soundNoticeSentence = $derived(screenSoundNoticeSentence(voice.screenSoundNotice, soundAdvice));
  const pushToTalkKey = $derived(voice.settings.pushToTalkKey);
  const pushToTalkSentence = $derived(
    pushToTalkKey ? `${strings.voice.pushToTalkHint(pushToTalkKey.label)}.` : strings.voice.pushToTalkNoKey,
  );
  const phaseSentence = $derived.by(() => {
    const name = client.voiceChannel?.channel.name;
    if (!name) {
      return "";
    }
    switch (voice.phase) {
      case "connecting":
        return strings.voice.connectingTo(name);
      case "connected":
        // Push-to-talk said once on connecting: otherwise silence would be a mystery.
        return voice.settings.inputMode === "push-to-talk"
          ? `${strings.voice.connectedTo(name)} ${pushToTalkSentence}`
          : strings.voice.connectedTo(name);
      case "reconnecting":
        return strings.voice.reconnectingTo(name);
      default:
        return "";
    }
  });
</script>

<p class="sr-only" aria-live="polite">{phaseSentence}</p>
<p class="sr-only" aria-live="polite">{voice.isSharingScreen ? strings.screenShare.youAreSharing : ""}</p>
<p class="sr-only" aria-live="polite">{voice.isCameraOn ? strings.camera.isOn : ""}</p>
<p class="sr-only" aria-live="polite">{voice.phase === "idle" ? "" : (voice.cameraProblem && strings.camera.problems[voice.cameraProblem]) || ""}</p>
<p class="sr-only" aria-live="polite">{voice.phase === "idle" ? "" : (voice.screenShareProblem && strings.screenShare.problems[voice.screenShareProblem]) || ""}</p>
<p class="sr-only" aria-live="polite">{soundNoticeSentence && `${describeSharedSound(voice)}. ${soundNoticeSentence}`}</p>
<p class="sr-only" aria-live="polite">{voice.isMutedMicrophoneHeardInShare ? strings.screenShare.mutedMicrophoneHeard : ""}</p>
<p class="sr-only" aria-live="polite">{client.voiceAnnouncement}</p>
<p class="sr-only" aria-live="polite">{voice.phase === "idle" ? "" : describeMissingDevices(voice).join(" ")}</p>

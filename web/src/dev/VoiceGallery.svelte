<script lang="ts">
  import { tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";

  import Navigator from "../components/Navigator.svelte";
  import StreamView from "../components/StreamView.svelte";
  import VoiceSettings from "../components/VoiceSettings.svelte";
  import AddSoundDialog from "../components/AddSoundDialog.svelte";
  import EditSoundDialog from "../components/EditSoundDialog.svelte";
  import SoundSettings from "../components/SoundSettings.svelte";
  import { BROWSER_VOICE_PORTS } from "../lib/voice-browser";
  import { DEFAULT_VOICE_SETTINGS, type VoiceSettings as Settings, withEqualizerBand } from "../lib/voice-settings";
  import type { RestClient } from "../lib/api";
  import { MellowClient } from "../lib/client.svelte";
  import {
    type Channel,
    type GatewayEvent,
    Permission,
    VIDEO_RECEIVE_SLOTS,
    VOICE_RECEIVE_SLOTS,
    type VoiceState,
  } from "../lib/protocol";
  import ScreenSoundHelpDialog from "../components/ScreenSoundHelpDialog.svelte";
  import type { ScreenShareProblem } from "../lib/screen-share";

  // Development only (web/dev/voice.html): the navigator with voice channels, fed with gateway
  // events by hand, since no instance speaks voice yet. `#state=` connected (default), connecting,
  // reconnecting, no-microphone, refused, moved, timeout, idle; `&theme=light`; `&wide=0` for a phone.
  // Stage 16.3: `&view=settings` shows Settings, Voice, with `&mode=` auto (default), manual or
  // ptt, `&key=0` (no push-to-talk key), `&devices=hidden` (before the permission),
  // `&output=unsupported` (Safari), `&missing=1` (remembered devices unplugged); `&view=menu`
  // opens Ana's menu with her volume. Headless browsers never answer `getUserMedia`: the
  // microphone is a wavering oscillator, so the meter moves. Voice processing (ADR-0031) runs for
  // real on it: `&enhance=1` turns the enhanced voice on, `&preset=custom` moves a slider,
  // `&rnnoise=unsupported|failed` shows the fallbacks, `&listen=1` plays it back ("S'écouter").
  // Soundboard (ADR-0032): `&view=soundboard` opens it from the voice panel, with `&sounds=0`
  // (none), `&block=cooldown|deafened|permission`, `&manage=0` (no "Add sounds"); Ana's sound shows
  // by her name. `&view=sound-settings` is Server settings, Soundboard, with `&dialog=add` (a
  // synthesised 9 s file in the trimmer) or `&dialog=edit`.
  // Screen sharing (ADR-0037): `&stream=share` (you share; Ana shares in Salon, Chloé in Jeux),
  // `&stream=connecting|live|ended` (you watch Ana: before her picture, with it, then she stops and
  // the view closes after its reading time), `&problem=denied|unsupported|stopped|
  // missing_permissions|too_many_cameras|video_unavailable` (the panel's line), `&quality=open`
  // (the share's options). The shared screen is a canvas drawn here, without sound: sharing shows
  // the "no sound" advice; `&source=monitor` shares a monitor input instead, and `&help=1` opens the
  // Linux help. Watching (docs/design/web-stream-watching.md): `&stream=paused` (the instance paused
  // Ana's picture), `&sound=on` (her screen's sound comes; otherwise "Sans son" after 5 s), and
  // `&stream=connecting` says the picture is not coming after 10 s.
  const parameters = new URLSearchParams(location.hash.slice(1));
  const view = parameters.get("view") ?? "navigator";
  const isDevicesHidden = parameters.get("devices") === "hidden";
  const isRememberedMissing = parameters.get("missing") === "1";
  const rnnoise = parameters.get("rnnoise");
  const stream = parameters.get("stream");
  const screenShareProblem = parameters.get("problem") as ScreenShareProblem | null;
  const hasVideo = stream !== null || screenShareProblem !== null || parameters.get("quality") === "open";
  const isStreamSoundOn = parameters.get("sound") === "on";
  /** What the share's options list as sound sources, as Chrome shows them on Linux with PipeWire. */
  const soundSourceDevices = [
    { kind: "audioinput", deviceId: "default", label: "Par défaut" },
    { kind: "audioinput", deviceId: "mic", label: "Micro interne" },
    { kind: "audioinput", deviceId: "monitor", label: "Monitor of Audio interne Stéréo analogique" },
    { kind: "audioinput", deviceId: "partage", label: "Monitor of Partage-Mellow" },
  ] as const;

  /** A slide someone shares: a code review, redrawn so the canvas keeps sending frames. */
  function canvasScreen(): MediaStream {
    const canvas = document.createElement("canvas");
    canvas.width = 1920;
    canvas.height = 1080;
    const context = canvas.getContext("2d");
    const draw = () => {
      if (!context) {
        return;
      }
      context.fillStyle = "#1e1e2a";
      context.fillRect(0, 0, 1920, 1080);
      context.fillStyle = "#2a2a3a";
      context.fillRect(0, 0, 1920, 64);
      context.fillStyle = "#c8c8d8";
      context.font = "28px monospace";
      context.fillText("crates/mellow-server/src/gateway/fan_out.rs", 32, 42);
      const lines = [
        "pub async fn fan_out(event: &GatewayEvent, sessions: &Sessions) {",
        "    // Serialised once, never under the lock.",
        "    let frame = Arc::<str>::from(serde_json::to_string(event)?);",
        "    for session in sessions.snapshot().iter() {",
        "        session.try_send(frame.clone());",
        "    }",
        "}",
      ];
      context.font = "40px monospace";
      lines.forEach((line, index) => {
        context.fillStyle = index === 1 ? "#7f8c6a" : "#e6e1ea";
        context.fillText(line, 96, 200 + index * 64);
      });
      context.fillStyle = "#e8894b";
      context.fillRect(96, 680 + (Date.now() / 40) % 200, 8, 48);
    };
    draw();
    setInterval(draw, 200);
    return canvas.captureStream(15);
  }

  /** A microphone that hums between quiet and loud, about once a second. */
  function oscillatorMicrophone(): MediaStream {
    const context = new AudioContext();
    const voice = context.createOscillator();
    voice.frequency.value = 220;
    const loudness = context.createGain();
    loudness.gain.value = 0.05;
    const wobble = context.createOscillator();
    wobble.frequency.value = 0.8;
    const wobbleDepth = context.createGain();
    wobbleDepth.gain.value = 0.05;
    wobble.connect(wobbleDepth).connect(loudness.gain);
    const destination = context.createMediaStreamDestination();
    voice.connect(loudness).connect(destination);
    voice.start();
    wobble.start();
    return destination.stream;
  }
  const galleryDevices = isDevicesHidden
    ? [
        { kind: "audioinput", deviceId: "", label: "", groupId: "" },
        { kind: "audiooutput", deviceId: "", label: "", groupId: "" },
      ]
    : [
        { kind: "audioinput", deviceId: "default", label: "Par défaut - Micro MacBook Pro", groupId: "a" },
        { kind: "audioinput", deviceId: "mac", label: "Micro MacBook Pro", groupId: "a" },
        { kind: "audioinput", deviceId: "yeti", label: "Yeti Stereo Microphone", groupId: "b" },
        { kind: "audiooutput", deviceId: "default", label: "Par défaut - Haut-parleurs MacBook Pro", groupId: "c" },
        { kind: "audiooutput", deviceId: "speakers", label: "Haut-parleurs MacBook Pro", groupId: "c" },
        { kind: "audiooutput", deviceId: "headset", label: "Casque WH-1000XM4", groupId: "d" },
      ];
  if (view === "settings") {
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: {
        getUserMedia: () => Promise.resolve(oscillatorMicrophone()),
        enumerateDevices: () => Promise.resolve(galleryDevices),
        addEventListener: () => {},
        removeEventListener: () => {},
      },
    });
  }
  const state = parameters.get("state") ?? "connected";
  const theme = parameters.get("theme");
  if (theme === "light" || theme === "dark") {
    document.documentElement.dataset["theme"] = theme;
  }

  const ME = "1";
  const GUILD = "100";
  const channel = (id: string, kind: Channel["kind"], name: string, position: number, parentId: string | null, extra: Partial<Channel> = {}): Channel => ({
    id,
    guild_id: GUILD,
    kind,
    name,
    topic: "",
    parent_id: parentId,
    position,
    permission_overwrites: [],
    ...extra,
  });
  const voiceState = (userId: string, channelId: string, extra: Partial<VoiceState> = {}): VoiceState => ({
    user_id: userId,
    guild_id: GUILD,
    channel_id: channelId,
    self_mute: false,
    self_deaf: false,
    suppressed: false,
    ...extra,
  });

  /** Answers nothing: the gallery has no instance. */
  const silentRest = new Proxy({}, { get: () => () => new Promise(() => {}) }) as unknown as RestClient;
  let connectPeer = () => {};
  let videoMid = 0;
  let deliverTrack: (event: { track: MediaStreamTrack; transceiver: { mid: string } }) => void = () => {};
  const fakePeer = {
    iceGatheringState: "complete",
    iceConnectionState: "new",
    signalingState: "stable",
    localDescription: { sdp: "offer" },
    addTransceiver: () => ({
      mid: `m${(videoMid += 1)}`,
      sender: {
        replaceTrack: () => Promise.resolve(),
        getParameters: () => ({ encodings: [] }),
        setParameters: () => Promise.resolve(),
      },
      setCodecPreferences: () => {},
    }),
    createOffer: () => Promise.resolve({ type: "offer", sdp: "offer" }),
    setLocalDescription: () => Promise.resolve(),
    addEventListener: (type: string, listener: () => void) => {
      if (type === "track") {
        deliverTrack = listener as unknown as typeof deliverTrack;
      }
      if (type === "iceconnectionstatechange") {
        connectPeer = () => {
          fakePeer.iceConnectionState = "connected";
          listener();
        };
      }
    },
    removeEventListener: () => {},
    getStats: () =>
      Promise.resolve(new Map([["p", { type: "candidate-pair", id: "p", nominated: true, state: "succeeded", currentRoundTripTime: 0.042 }]])),
    close: () => {},
  };
  const fakeTrack = { enabled: true, stop: () => {}, addEventListener: () => {} };
  const client = new MellowClient(silentRest, {
    createPeerConnection: () => fakePeer as unknown as RTCPeerConnection,
    meterLevel:
      view === "settings"
        ? BROWSER_VOICE_PORTS.meterLevel
        : () => (isStreamSoundOn ? { level: () => 0.1, stop: () => {} } : null),
    prepareNoiseSuppressor:
      rnnoise === "unsupported" || rnnoise === "failed" ? () => Promise.resolve(rnnoise) : BROWSER_VOICE_PORTS.prepareNoiseSuppressor,
    processMicrophone: view === "settings" ? BROWSER_VOICE_PORTS.processMicrophone : () => Promise.resolve(null),
    requestMicrophone: () =>
      view === "settings"
        ? Promise.resolve(oscillatorMicrophone())
        : state === "no-microphone"
        ? Promise.reject(Object.assign(new Error("none"), { name: "NotFoundError" }))
        : Promise.resolve({ getAudioTracks: () => [fakeTrack], getTracks: () => [fakeTrack] } as unknown as MediaStream),
    playTrack: view === "settings" ? BROWSER_VOICE_PORTS.playTrack : () => ({ setGain: () => {}, setOutputDevice: () => {}, stop: () => {} }),
    amplify: () => null,
    listDevices: () => Promise.resolve(hasVideo ? soundSourceDevices : []),
    watchDevices: () => () => {},
    storage: () => null,
    requestScreen: () => Promise.resolve(canvasScreen()),
    requestCamera: () => Promise.resolve(canvasScreen()),
    listCameras: () => Promise.resolve([{ deviceId: "facetime", label: "FaceTime HD Camera" }]),
    requestSoundSource: () => Promise.resolve(oscillatorMicrophone()),
    codecPreferences: () => (hasVideo ? { audio: [], video: [] } : null),
  });

  const people = [
    { id: ME, username: "paul", display_name: "Paul" },
    { id: "2", username: "ana", display_name: "Ana" },
    { id: "3", username: "bob", display_name: "Bob" },
    { id: "4", username: "chloe", display_name: "Chloé" },
    { id: "5", username: "dimitri", display_name: "Dimitri Kowalczyk-Lefèvre" },
  ];
  const gallerySounds = [
    { id: "901", name: "Klaxon", emoji: "📯", duration_milliseconds: 1_200 },
    { id: "902", name: "Applaudissements", emoji: "👏", duration_milliseconds: 4_800 },
    { id: "903", name: "Ta-da", emoji: "🎉", duration_milliseconds: 1_600 },
    { id: "904", name: "Trombone triste", emoji: "🎺", duration_milliseconds: 3_100 },
    { id: "905", name: "Bruh", duration_milliseconds: 700 },
    { id: "906", name: "Cri de chèvre", emoji: "🐐", duration_milliseconds: 2_200 },
    { id: "907", name: "Ding", emoji: "🔔", duration_milliseconds: 900 },
    { id: "908", name: "Roulement de tambour très long", emoji: "🥁", duration_milliseconds: 5_200 },
  ];
  /**
   * Nine seconds of a falling tone with a beat, in stereo at 44.1 kHz (so decoding resamples and
   * the render downmixes, as with a real MP3), written as a plain WAV by hand.
   */
  function galleryFile(): File {
    const rate = 44_100;
    const frames = rate * 9;
    const bytes = new DataView(new ArrayBuffer(44 + frames * 4));
    const text = (offset: number, value: string) => [...value].forEach((character, index) => bytes.setUint8(offset + index, character.charCodeAt(0)));
    text(0, "RIFF");
    bytes.setUint32(4, 36 + frames * 4, true);
    text(8, "WAVEfmt ");
    bytes.setUint32(16, 16, true);
    bytes.setUint16(20, 1, true);
    bytes.setUint16(22, 2, true);
    bytes.setUint32(24, rate, true);
    bytes.setUint32(28, rate * 4, true);
    bytes.setUint16(32, 4, true);
    bytes.setUint16(34, 16, true);
    text(36, "data");
    bytes.setUint32(40, frames * 4, true);
    for (let frame = 0; frame < frames; frame += 1) {
      const seconds = frame / rate;
      const beat = Math.abs(Math.sin(Math.PI * seconds * 1.5));
      const sample = Math.sin(2 * Math.PI * (440 - 30 * seconds) * seconds) * beat * (0.3 + 0.7 * Math.abs(Math.sin(seconds * 0.7)));
      bytes.setInt16(44 + frame * 4, Math.round(sample * 0x7fff), true);
      bytes.setInt16(46 + frame * 4, Math.round(sample * 0.5 * 0x7fff), true);
    }
    return new File([bytes.buffer], "klaxon_de_velo-final.wav", { type: "audio/wav" });
  }

  client.applyReady({
    session_id: "gallery",
    user: { ...people[0]!, email: "paul@example.org", is_instance_administrator: true },
    guilds: [
      {
        id: GUILD,
        name: "Rust Lyon",
        owner_id: ME,
        my_permissions: String(
          parameters.get("block") === "permission"
            ? Permission.VIEW_CHANNEL | Permission.CONNECT | Permission.SPEAK
            : parameters.get("manage") === "0"
              ? Permission.VIEW_CHANNEL | Permission.CONNECT | Permission.SPEAK | Permission.USE_SOUNDBOARD
              : Permission.ADMINISTRATOR,
        ),
        my_role_ids: [],
        roles: [],
        sounds: parameters.get("sounds") === "0" ? [] : gallerySounds,
        my_join_sound_id: "903",
        emojis: [],
        channels: [
          channel("10", "category", "Salons textuels", 0, null),
          channel("11", "text", "général", 0, "10"),
          channel("12", "text", "annonces", 1, "10"),
          channel("20", "category", "Salons vocaux", 1, null),
          channel("21", "voice", "Salon", 0, "20"),
          channel("22", "voice", "Jeux", 1, "20", { user_limit: 5 }),
          channel("23", "voice", "Réunion du jeudi soir", 2, "20"),
        ],
      },
    ],
    read_states: [],
    online_user_ids: people.map((person) => person.id),
    voice_states: [
      voiceState("2", "21", { self_stream: stream !== null }),
      voiceState("3", "21", { self_mute: true }),
      voiceState("4", "22", { self_mute: true, self_deaf: true, self_stream: stream !== null }),
      voiceState("5", "22", { suppressed: true }),
    ],
    voice_enabled: true,
  });
  client.membersByGuild.set(
    GUILD,
    people.map((user) => ({ user, role_ids: [], joined_at_unix_milliseconds: 0 })),
  );
  client.status = "connected";
  const dispatch = (event: GatewayEvent) => client.applyEvent(event, Date.now());

  const mode = parameters.get("mode") ?? "auto";
  const gallerySettings: Settings = {
    ...DEFAULT_VOICE_SETTINGS,
    inputMode: mode === "ptt" ? "push-to-talk" : "voice-activity",
    isSensitivityAutomatic: mode !== "manual",
    sensitivityThresholdDecibels: -32,
    pushToTalkKey: parameters.get("key") === "0" ? null : { code: "ControlRight", label: "Ctrl droit" },
    inputDeviceId: isRememberedMissing ? "usb-gone" : "yeti",
    outputDeviceId: isRememberedMissing ? "dock-gone" : "headset",
    inputVolumePercent: 120,
    isVoiceEnhancementOn: parameters.get("enhance") === "1",
  };
  client.voice.updateSettings(
    parameters.get("preset") === "custom" ? withEqualizerBand(gallerySettings, "treble", 7) : gallerySettings,
  );
  client.voice.setPersonVolume("2", 150);
  client.voice.togglePersonMuted("3");

  async function stage() {
    if (view === "settings") {
      if (!isDevicesHidden) {
        await client.voice.startMicrophoneTest();
      }
      if (parameters.get("listen") === "1") {
        client.voice.setMonitoring(true);
      }
      return;
    }
    if (state === "idle") {
      return;
    }
    if (state === "refused" || state === "moved" || state === "timeout") {
      await client.voice.join("22");
      if (state === "refused") {
        dispatch({
          t: "VOICE_JOIN_REFUSED",
          data: { channel_id: "22", offer_id: client.voice.currentOfferId, reason: "channel_full" },
        });
      } else if (state === "moved") {
        connectPeer();
        dispatch({ t: "VOICE_DISCONNECTED", data: { channel_id: "22", reason: "moved_elsewhere" } });
      } else {
        await client.voice.applyAnswer("22", client.voice.currentOfferId, "answer");
        fakePeer.iceConnectionState = "failed";
        connectPeer = () => {};
        client.voice.disconnectedByServer("22", "media_timeout");
        client.voice.disconnectedByServer("22", "media_timeout");
      }
      return;
    }
    // The mids this join's lines get, numbered from here as the fake peer hands them out.
    const firstMid = videoMid + 1;
    await client.voice.join("21");
    if (state === "connecting") {
      return;
    }
    connectPeer();
    dispatch({ t: "VOICE_STATE_UPDATE", data: { voice_state: voiceState(ME, "21", { self_mute: state === "no-microphone" }) } });
    dispatch({ t: "SPEAKING", data: { guild_id: GUILD, channel_id: "21", user_id: "2", speaking: true } });
    if (state === "reconnecting") {
      client.voice.disconnectedByServer("21", "media_timeout");
    }
    if (stream === "share") {
      if (parameters.get("source") === "monitor") {
        client.voice.setScreenSoundDevice("monitor");
      }
      await client.voice.startScreenShare();
      dispatch({ t: "VOICE_STATE_UPDATE", data: { voice_state: voiceState(ME, "21", { self_stream: true }) } });
    }
    if (screenShareProblem !== null) {
      client.voice.screenShareProblem = screenShareProblem;
    }
    if (parameters.get("quality") === "open") {
      await tick();
      document.querySelector<HTMLButtonElement>("button[aria-controls=screen-share-options-menu]")?.click();
    }
    if (stream === "connecting" || stream === "live" || stream === "ended" || stream === "paused") {
      client.voice.watchStream("2");
      if (stream !== "connecting") {
        // The first video receive slot: after the voice line, its receive slots and the camera line.
        const mid = `m${firstMid + 1 + VOICE_RECEIVE_SLOTS + 1}`;
        dispatch({ t: "VOICE_SLOT_ASSIGNED", data: { channel_id: "21", mid, user_id: "2" } });
        const [screenTrack] = canvasScreen().getVideoTracks();
        if (screenTrack) {
          deliverTrack({ track: screenTrack, transceiver: { mid } });
        }
        if (isStreamSoundOn) {
          // The first screen sound receive slot: after the video slots and your own screen's sound line.
          const soundMid = `m${firstMid + 1 + VOICE_RECEIVE_SLOTS + 1 + VIDEO_RECEIVE_SLOTS + 1}`;
          const [soundTrack] = oscillatorMicrophone().getAudioTracks();
          if (soundTrack) {
            deliverTrack({ track: soundTrack, transceiver: { mid: soundMid } });
          }
          dispatch({ t: "VOICE_SLOT_ASSIGNED", data: { channel_id: "21", mid: soundMid, user_id: "2" } });
        }
      }
      if (stream === "paused") {
        dispatch({ t: "VOICE_VIDEO_PAUSED", data: { channel_id: "21", user_id: "2", is_paused: true } });
      }
      if (stream === "ended") {
        setTimeout(() => dispatch({ t: "VOICE_STATE_UPDATE", data: { voice_state: voiceState("2", "21") } }), 1_500);
      }
    }
    if (view === "soundboard") {
      dispatch({ t: "VOICE_SOUND_PLAYED", data: { channel_id: "21", user_id: "2", sound_id: "904" } });
      if (parameters.get("block") === "cooldown") {
        dispatch({ t: "VOICE_SOUND_PLAYED", data: { channel_id: "21", user_id: ME, sound_id: "901" } });
      } else if (parameters.get("block") === "deafened") {
        client.voice.toggleDeafened();
      }
      await tick();
      document.querySelector<HTMLButtonElement>("button[aria-haspopup=dialog][aria-expanded]")?.click();
    }
    if (view === "menu") {
      await tick();
      const ana = [...document.querySelectorAll<HTMLButtonElement>("button[aria-haspopup=menu]")].find((row) =>
        row.textContent?.includes("Ana"),
      );
      ana?.click();
    }
  }
  void stage();

  const collapsedCategoryIds = new SvelteSet<string>();
  const isWide = parameters.get("wide") !== "0";
</script>

{#if view === "sound-settings"}
  {@const guild = client.guilds[0]!}
  <div class="min-h-dvh bg-(--mw-component-settings-content-background) font-sans text-text-normal">
    <div class="mx-auto max-w-(--mw-component-settings-content-max-width) px-(--mw-component-settings-content-padding-inline) pt-(--mw-component-settings-content-padding-top)">
      <SoundSettings {client} {guild} isCompact={!isWide} />
    </div>
  </div>
  {#if parameters.get("dialog") === "add"}
    <AddSoundDialog {client} {guild} file={galleryFile()} isCompact={!isWide} onclose={() => {}} onadded={() => {}} />
  {:else if parameters.get("dialog") === "edit"}
    <EditSoundDialog {client} {guild} sound={gallerySounds[3]!} isCompact={!isWide} onclose={() => {}} onsaved={() => {}} />
  {/if}
{:else if view === "settings"}
  <div class="min-h-dvh bg-(--mw-component-settings-content-background) font-sans text-text-normal">
    <div
      class={[
        "mx-auto max-w-(--mw-component-settings-content-max-width) pb-(--mw-component-settings-content-padding-bottom)",
        isWide ? "px-(--mw-component-settings-content-padding-inline) pt-(--mw-component-settings-content-padding-top)" : "px-(--mw-component-settings-content-padding-inline-narrow) pt-xs",
      ]}
    >
      <VoiceSettings
        voice={client.voice}
        canChooseOutputDevice={parameters.get("output") !== "unsupported"}
        canSuppressNoiseInPage={rnnoise !== "unsupported"}
      />
    </div>
  </div>
{:else}
<div class="flex h-dvh gap-(--mw-size-sheet-inset) bg-surface-rail p-(--mw-size-sheet-inset) font-sans text-text-normal">
  <div class={isWide ? "h-full" : "h-full w-full"}>
    <Navigator
      {client}
      {collapsedCategoryIds}
      isInDrawer={!isWide}
      onaddserver={() => {}}
      onnewmessage={() => {}}
      onopendialog={() => {}}
      oncreatechannelin={() => {}}
      oneditchannel={() => {}}
      onnavigate={() => {}}
      onopenusersettings={() => {}}
    />
  </div>
  {#if isWide && stream !== null}
    <main class="relative flex min-w-0 flex-1 rounded-(--mw-component-sheet-radius) bg-surface-chat">
      {#if client.voice.watchedStreamUserId !== null}
        {#key client.voice.watchedStreamUserId}
          <StreamView {client} userId={client.voice.watchedStreamUserId} hasNavigationDrawer={false} onopennavigation={() => {}} />
        {/key}
      {/if}
    </main>
  {/if}
</div>
{#if parameters.get("help") === "1"}
  <ScreenSoundHelpDialog onclose={() => {}} />
{/if}
{/if}

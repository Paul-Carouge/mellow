<script lang="ts">
  import { tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";

  import Navigator from "../components/Navigator.svelte";
  import VoiceSettings from "../components/VoiceSettings.svelte";
  import { BROWSER_VOICE_PORTS } from "../lib/voice-browser";
  import { DEFAULT_VOICE_SETTINGS, type VoiceSettings as Settings, withEqualizerBand } from "../lib/voice-settings";
  import type { RestClient } from "../lib/api";
  import { MellowClient } from "../lib/client.svelte";
  import { type Channel, type GatewayEvent, Permission, type VoiceState } from "../lib/protocol";

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
  const parameters = new URLSearchParams(location.hash.slice(1));
  const view = parameters.get("view") ?? "navigator";
  const isDevicesHidden = parameters.get("devices") === "hidden";
  const isRememberedMissing = parameters.get("missing") === "1";
  const rnnoise = parameters.get("rnnoise");

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
  const fakePeer = {
    iceGatheringState: "complete",
    iceConnectionState: "new",
    signalingState: "stable",
    localDescription: { sdp: "offer" },
    addTransceiver: () => ({ sender: { replaceTrack: () => Promise.resolve() } }),
    createOffer: () => Promise.resolve({ type: "offer", sdp: "offer" }),
    setLocalDescription: () => Promise.resolve(),
    addEventListener: (type: string, listener: () => void) => {
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
    meterLevel: view === "settings" ? BROWSER_VOICE_PORTS.meterLevel : () => null,
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
    listDevices: () => Promise.resolve([]),
    watchDevices: () => () => {},
    storage: () => null,
  });

  const people = [
    { id: ME, username: "paul", display_name: "Paul" },
    { id: "2", username: "ana", display_name: "Ana" },
    { id: "3", username: "bob", display_name: "Bob" },
    { id: "4", username: "chloe", display_name: "Chloé" },
    { id: "5", username: "dimitri", display_name: "Dimitri Kowalczyk-Lefèvre" },
  ];
  client.applyReady({
    session_id: "gallery",
    user: { ...people[0]!, email: "paul@example.org", is_instance_administrator: true },
    guilds: [
      {
        id: GUILD,
        name: "Rust Lyon",
        owner_id: ME,
        my_permissions: String(Permission.ADMINISTRATOR),
        my_role_ids: [],
        roles: [],
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
      voiceState("2", "21"),
      voiceState("3", "21", { self_mute: true }),
      voiceState("4", "22", { self_mute: true, self_deaf: true }),
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

{#if view === "settings"}
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
<div class="flex h-dvh bg-surface-rail p-(--mw-size-sheet-inset) font-sans text-text-normal">
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
</div>
{/if}

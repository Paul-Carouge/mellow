# 0062. Private calls in conversations

- Status: Proposed (phase 1: protocol, server and client core; the web and desktop interfaces follow)
- Date: 2026-11-15
- Builds on: [ADR-0030](0030-voice-channels-with-an-sfu-in-the-server-binary.md) (the SFU), [ADR-0035](0035-video-in-voice-channels.md) and [ADR-0037](0037-screen-sharing-on-the-camera-path.md) (cameras and screens), [ADR-0025](0025-friends-and-blocking.md) (who may reach whom), [ADR-0026](0026-group-conversations.md) (groups), [ADR-0007](0007-permission-model.md) (permissions decided in one place)

## Context
The owner asked for custom ringtones, above all for an incoming call. Mellow has no calls: people only join a guild's voice channel. The owner chose to build **private calls**: calling one person or a group from their conversation or profile; the other app rings with its ringtone, and they answer or decline. A call uses the voice channels' engine, cameras and screen sharing included. The ringtones are the owner's own files (`design/sounds/`): "Friendly Chime", and "Jingle of the Handbells" for Christmas, played from 1 to 31 December without any setting.

## Decision

### Model: a call is a voice session of a conversation
- **One active call per conversation** (one-to-one or group), kept in memory by the voice service beside its participants. Its media go through the existing SFU as a **virtual voice channel whose id is the conversation's id**: the same `voice_join`, `voice_leave`, `voice_state`, `voice_video_watch` frames, the same `VOICE_SESSION_DESCRIPTION`, `VOICE_SLOT_ASSIGNED`, `VOICE_VIDEO_*` and `VOICE_DISCONNECTED` events, the same media loop, limits (instance `max_participants`, cameras per channel and instance) and one-connection-per-user rule (joining a call leaves a voice channel, and the reverse). No second media path.
- Only the parts that name a guild differ: a call participant is announced in the call (`CALL_UPDATE`), not in `VOICE_STATE_UPDATE`, and speaks through `CALL_SPEAKING`, not `SPEAKING`; both of those carry a required `guild_id` that older clients would misread. Soundboard sounds and join sounds are guild features and stay out of calls.
- **Everyone in a call may speak and show a camera or screen**: there are no roles in a conversation, so `SPEAK` and `STREAM` are implied.

### Lifecycle and signalling
- **Starting**: `POST /api/v1/channels/{channel_id}/call` (the conversation's id) creates the call, or answers the one already going on (idempotent: then the caller just joins). It answers the `Call`; the caller then sends `voice_join` with the conversation's id. Creating it posts the call's **message** in the conversation (below) and sends `CALL_CREATE` to every participant of the conversation.
- **Ringing**: `Call.ringing_user_ids` lists who is being rung: everyone in the conversation but the caller. Each client of a listed user rings, on every device; a client in another call or voice channel plays a short "call waiting" cue instead. Some of them ring **silently**: listed for everyone like anyone who does not answer, until ringing ends, but left out of their own view of the call, so their devices stay quiet. That keeps blocks secret (below).
- **Answering** is joining: `voice_join` with the conversation's id. The user leaves `ringing_user_ids` (`CALL_UPDATE`), so **their other devices stop ringing**; only the device that joined holds the connection (one per user).
- **Declining**: `POST /api/v1/channels/{channel_id}/call/decline` takes the user out of `ringing_user_ids` on all their devices (`CALL_UPDATE`). They can still join later.
- **Ringing ends after 30 s** (`CALL_RING_TIMEOUT_MILLISECONDS`, shared with clients): `ringing_user_ids` empties. A bounded background task per active call waits for it, stopped when the call ends first; active calls are capped by the instance's `max_participants`. **One unanswered call per caller at a time** (`409 CONFLICT` otherwise).
- **Ending**: the call ends (`CALL_DELETE`) when
  - nobody answered (no one but the caller ever joined) and no one is ringing any more: everyone declined, ringing timed out, or the caller hung up before an answer (a caller who never joined counts as hung up once ringing ends);
  - or, once someone answered, **the last participant leaves**. In a group, the call goes on while anyone remains, and the others may join or rejoin until then; in a one-to-one call the same rule applies (the one left alone hangs up).
- **Busy** is not a server state and is never told to the caller (it would reveal presence): a busy callee's client shows a waiting call.
- **The call's message**: starting a call posts a message by the caller in the conversation, with an empty text and `Message.call = { participant_ids, ended_at_unix_milliseconds }` (new tables `call_messages` and `call_message_participants`, migration 0044). It is the conversation's trace: "Call · 12 min" when it ended with people who joined, "Missed call" for a reader who is not in `participant_ids` once it ended, "Unanswered call" for a reader who was its only participant (the caller who rang alone, owner 2026-11-15), "Ongoing call" before. Each participant is recorded at their first join; the end time, with everyone who joined, is written when the call ends (retried once), with a `MESSAGE_UPDATE`. `Message.call.participant_ids` is shown only once the call ended. Duration counts from the message's time (the call's start). A call message cannot be edited (like a poll); its author may delete it as any message. A conversation's first message being a call also announces the conversation to the callee (`DIRECT_MESSAGE_CHANNEL_CREATE`), as a first text message does. A stopping server ends every call going on and writes their records before closing the database; a call a crash left unfinished is closed at the next start with that start's time, with the participants recorded at their joins.
- **`Ready.calls`** lists the active calls of the user's conversations; `Ready.calls_enabled` says the server knows calls and voice works (clients hide the call button otherwise).

### Permissions and safety
- **Who may call or join**: a participant of the conversation who may write there (`domain::permissions::may_take_part_in_call`, from `direct_message_permissions`): in a one-to-one conversation, two people who share a guild or are friends and have not blocked each other (ADR-0025); in a group, every participant (ADR-0026). Anyone else gets `NOT_FOUND`, like a hidden conversation. Guild channels are not callable (`NOT_FOUND`).
- **Ringing never crosses a block** (`domain::permissions::rings_in_call`): in a group, someone the caller blocked, or who blocked them, rings silently, from the start or from the block if it is placed while the call rings; they still see that a call is going on, as they see the group's messages, and may join it. The list of silent people stays on the server, so neither the caller nor the group learns of a block.
- **Losing access ends one's part**: the hub notes which group changed and which two people's relationship changed; the voice service re-checks only the calls holding them (a group someone joined or left, a conversation holding both people), and every one-to-one call after a guild change (the only shared guild may be gone: one reachability read each). Reads happen before the voice lock (a join meanwhile is checked again by `admit`), one pass per conversation (a group allows all its participants, a one-to-one conversation both or neither); a failed read refuses (fail closed). Who lost access is disconnected (`VOICE_DISCONNECTED`, `permissions_changed`), no longer rung, and gets `CALL_DELETE`; someone added to a group gets `CALL_CREATE`.
- **Do not disturb** is the callee's client's decision: it shows the call without sound. The server never consults presence for calls, so a caller learns nothing from ringing (an invisible or offline callee is "ringing" like anyone): **no presence leaks**. Call events reach the conversation's participants only.
- **Rate limits**: 6 calls started per user per minute and 30 per hour, and 3 per conversation per 10 minutes whoever starts them (each posts a message and rings people); 30 declines per user per minute; joins keep the voice limits (10 per minute). A person rings for at most 10 calls per 10 minutes from people who are not their friends: past that, such a call still starts but rings them silently, without a `429`, so the caller learns nothing.

### Compatibility
Additions only: `/api/v1` and `/gateway?v=1` keep every released shape.
- New events `CALL_CREATE`, `CALL_UPDATE`, `CALL_DELETE` and `CALL_SPEAKING`; new routes; `Ready.calls` and `Ready.calls_enabled`; `Message.call`; `CALL_RING_TIMEOUT_MILLISECONDS`. Older clients skip the unknown events, ignore the new fields, and show the call message as an empty message by the caller (as they show a poll).
- `VoiceState` and `SPEAKING` are unchanged and never carry a call; `Ready.voice_states` never lists call participants, so an older client never reads a voice state without a guild.
- A newer client on an older server sees `calls_enabled` false and hides calls.

### Ringtones
- The files are the owner's (`design/sounds/README.md`): `ringtone.opus` and `ringtone-christmas.opus`, Ogg Opus, 10 s, looped while ringing. The web client imports them through Vite (hashed, cached for good, ADR-0019); the desktop embeds them (`include_bytes!` in `mellow-client-core::ringtones`).
- **The Christmas ringtone plays from 1 to 31 December, the usual one otherwise**, by the client's local date, without a setting: `mellow_client_core::ringtones::ringtone_for_date` and `ringtoneFor` on the web, both pure and tested.
- Ringing stops when the call leaves the user's `ringing_user_ids` (answered elsewhere, declined, timed out, call ended) and after `CALL_RING_TIMEOUT_MILLISECONDS` at the latest, even without an event. Its volume is the client's notification volume; do not disturb mutes it.
- **The caller's waiting tone** (owner, 2026-11-15): a soft generated beep, 425 Hz, 1 s on and 4 s off (the European ringback), so "I am calling" never sounds like "I am called". No file: each client generates it.
- **Blocked people in a call**: a client starts with the sound of people the user blocked turned off (`ClientState::is_muted_by_default_in_calls`); the user may turn them up.

## Consequences
- One migration (0044: `call_messages`, `call_message_participants`), forward-only. Two routes, four events, two `Ready` fields, one `Message` field.
- The voice service now tells a guild participant from a call participant (`VoiceScope`); every guild path is unchanged, which the existing voice tests check.
- An access change re-checks only the calls it may concern, reading before the voice lock.
- shortcut: no "ring again" for someone who declined or missed it in a group; they join from the conversation. Add `POST …/call/ring` when asked.
- shortcut: the duration of a call a crash interrupted counts up to the next start; a clean stop ends it at once.

## Owner's choices (2026-11-15)
- Friends and people who share a guild may call each other, never across a block (as messaging, ADR-0025, and as on Discord).
- The waiting tone is the generated beep above.
- The ringtones' metadata was removed; their licence is to be confirmed by the owner.

## Alternatives considered
- **A second media path for calls**: duplicates the SFU's limits, budgets and tests for no gain.
- **Making `VoiceState.guild_id` optional**: older clients would fail to read `Ready` as soon as a call is listed in `voice_states`.
- **A hidden voice channel row per conversation**: a fake guild channel would leak into permission, audience and audit code built for guilds.
- **Ringing only online users, or telling the caller "busy" or "unavailable"**: reveals presence, invisible status and activity to anyone who may call.
- **A `calls` table for the active call**: the state is live and in memory like voice participants; only the trace in the conversation needs to last.

# Sounds

Sounds the clients play, shared by the web client (imported through Vite, hashed and cached for good) and the desktop client (embedded by `mellow-client-core::ringtones`).

| File | What | Used for |
|---|---|---|
| `ringtone.opus` | "Friendly Chime", 10 s, Ogg Opus, 48 kHz stereo | The ringtone of an incoming call ([ADR-0062](../../docs/adr/0062-private-calls.md)), looped while the call rings, from 1 January to 30 November |
| `ringtone-christmas.opus` | "Jingle of the Handbells", 10 s, Ogg Opus, 48 kHz stereo | The same ringtone from 1 to 31 December, by the device's local date, without a setting |

## Origin and licence
- Provided by the owner on 2026-11-15, converted to Ogg Opus from their MP3 files.
- **Licence: to be confirmed by the owner** before a release ships them: who made them, and under which terms they may be redistributed with an AGPL-3.0 project. Until then they are not covered by the repository's licence.
- Their source's metadata was removed: only the `encoder` tag remains.

## Adding a sound
Ogg Opus, 48 kHz, as short as it can be; name it for what it is used for, list it here with its origin and licence, and import it from the code that plays it on both clients.

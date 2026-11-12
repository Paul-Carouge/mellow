//! The soundboard's part of the session (`soundboard-session.svelte.ts`, ADR-0032): your own
//! sounds sent to the channel, and the emoji shown by a join sound's player. Played sounds,
//! the cooldown's echo and the sound lists come with the gateway (`ClientState`).

use gpui::Context;
use mellow_protocol::{ClientFrame, Snowflake};

use super::{Session, StateChanged, now_unix_milliseconds};

impl Session {
    /// Asks the instance to play `sound_id` in the reader's channel (it answers with
    /// `VOICE_SOUND_PLAYED`); the cooldown starts now. False when the gateway is not connected.
    /// The caller checks `soundboard_block` first: the instance silently drops a refused frame.
    pub fn send_soundboard_sound(&mut self, sound_id: Snowflake, cx: &mut Context<Self>) -> bool {
        let is_sent = self
            .gateway_handle()
            .is_some_and(|gateway| gateway.send(ClientFrame::VoicePlaySound { sound_id }));
        if is_sent {
            self.state.note_own_sound_sent(now_unix_milliseconds());
            cx.notify();
        }
        is_sent
    }

    /// A join sound this app plays: its emoji shows by `user_id`'s name for the sound's length.
    pub fn show_sound_indicator(
        &mut self,
        user_id: Snowflake,
        guild_id: Snowflake,
        sound_id: Snowflake,
        cx: &mut Context<Self>,
    ) {
        let change =
            self.state
                .show_sound_indicator(user_id, guild_id, sound_id, now_unix_milliseconds());
        cx.emit(StateChanged(vec![change]));
        cx.notify();
    }
}

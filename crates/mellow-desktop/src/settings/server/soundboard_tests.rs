//! The Soundboard page headless, without an instance: the list, the count, the dialogs.

use gpui::TestAppContext;
use mellow_client_core::shared_logic::server_settings::ServerSettingsSection;
use mellow_protocol::GatewayEvent;

use super::*;
use crate::settings::server::test_support::{open_server_settings, receive, shown_page};
use crate::shell_fixtures::GUILD_RUST;

fn sound(id: u64, name: &str) -> SoundboardSound {
    SoundboardSound {
        id: Snowflake::from_raw(id),
        name: name.to_owned(),
        emoji: None,
        duration_milliseconds: 1_200,
    }
}

#[gpui::test]
fn the_page_lists_the_gateway_s_sounds_and_closes_upload_when_full(cx: &mut TestAppContext) {
    let (session, settings, cx) = open_server_settings(cx, ServerSettingsSection::Soundboard);
    let page = shown_page::<ServerSoundboardPage>(&settings, cx);
    assert!(page.read_with(cx, |page, cx| page.sounds(cx).is_empty()));

    receive(
        &session,
        GatewayEvent::GuildSoundsUpdate {
            guild_id: GUILD_RUST,
            sounds: vec![sound(1, "horn"), sound(2, "drum")],
        },
        cx,
    );
    page.read_with(cx, |page, cx| {
        assert_eq!(page.sounds(cx).len(), 2);
        assert!(!page.is_full(cx));
    });

    let full: Vec<SoundboardSound> = (0..MAXIMUM_SOUNDS_PER_GUILD as u64)
        .map(|id| sound(id + 10, "beep"))
        .collect();
    receive(
        &session,
        GatewayEvent::GuildSoundsUpdate {
            guild_id: GUILD_RUST,
            sounds: full,
        },
        cx,
    );
    assert!(page.read_with(cx, ServerSoundboardPage::is_full));
}

#[gpui::test]
fn editing_opens_with_the_sound_s_name_and_refuses_a_bad_one_before_sending(
    cx: &mut TestAppContext,
) {
    let (session, settings, cx) = open_server_settings(cx, ServerSettingsSection::Soundboard);
    receive(
        &session,
        GatewayEvent::GuildSoundsUpdate {
            guild_id: GUILD_RUST,
            sounds: vec![sound(1, "horn")],
        },
        cx,
    );
    let page = shown_page::<ServerSoundboardPage>(&settings, cx);
    cx.update(|window, cx| {
        page.update(cx, |page, cx| page.edit(sound(1, "horn"), window, cx));
    });
    cx.run_until_parked();
    let form = page.read_with(cx, |page, _| {
        page.adding_or_editing
            .as_ref()
            .map(|open| open.form.clone())
            .expect("the edit dialog is open")
    });
    assert_eq!(
        form.read_with(cx, |form, cx| form.name_input.read(cx).value().to_string()),
        "horn"
    );
    cx.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.name_input
                .update(cx, |input, cx| input.set_value("x", window, cx));
            form.submit(cx);
        });
    });
    form.read_with(cx, |form, cx| {
        assert_eq!(
            form.name_error.as_deref(),
            Some(strings(cx).server_settings().sounds().name_length())
        );
        assert!(
            !form.dialog().unwrap().read(cx).is_busy(),
            "nothing was sent"
        );
    });
}

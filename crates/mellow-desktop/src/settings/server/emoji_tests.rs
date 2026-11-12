//! The emoji page headless, without an instance: what it refuses before sending anything.

use gpui::{TestAppContext, VisualTestContext, WindowAppearance};
use mellow_client_core::state::ClientState;
use mellow_protocol::{ApiErrorCode, GatewayEvent};

use super::*;
use crate::shell_fixtures::{GUILD_RUST, sample_state_administering_rust};

const PARTY: Snowflake = Snowflake::from_raw(77);

fn state_with_party() -> ClientState {
    let mut state = sample_state_administering_rust();
    let mut guild = state
        .guild(GUILD_RUST)
        .cloned()
        .expect("the sample world has Rust Lyon");
    guild.emojis = vec![CustomEmoji {
        id: PARTY,
        name: "party".to_owned(),
        is_animated: false,
    }];
    state.apply_event(GatewayEvent::GuildCreate(Box::new(guild)), 0);
    state
}

fn open_page(cx: &mut TestAppContext) -> (Entity<ServerEmojiPage>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(crate::strings_pending_i18n::ActiveStrings(
            &crate::strings_pending_i18n::ENGLISH,
        ));
        cx.set_global(crate::theme::ActiveTheme::for_appearance(
            WindowAppearance::Dark,
        ));
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| *session.state_mut() = state_with_party());
    let (page, cx) =
        cx.add_window_view(|window, cx| ServerEmojiPage::new(session, GUILD_RUST, window, cx));
    cx.run_until_parked();
    (page, cx)
}

#[gpui::test]
fn files_that_cannot_be_emoji_fail_at_once_and_only_a_full_kind_may_be_retried(
    cx: &mut TestAppContext,
) {
    let (page, cx) = open_page(cx);
    page.update(cx, |page, cx| {
        page.add_files(
            vec![
                (PathBuf::from("notes.txt"), Some(10)),
                (
                    PathBuf::from("party.png"),
                    Some(MAXIMUM_EMOJI_UPLOAD_BYTES + 1),
                ),
                (PathBuf::from("gone.png"), None),
            ],
            cx,
        );
    });
    page.read_with(cx, |page, _| {
        assert!(!page.is_uploading, "nothing was sent");
        let [text, large, unreadable] = page.uploads.as_slice() else {
            panic!("every file is listed, the unreadable one too");
        };
        assert_eq!(text.status, UploadStatus::Failed);
        assert!(!text.can_retry);
        assert!(text.error.contains("isn't a PNG, JPEG, GIF or WebP"));
        assert_eq!(large.status, UploadStatus::Failed);
        assert!(large.error.contains("emoji are at most 256 KB"));
        // Named after the file, past the emoji already called "party".
        assert_eq!(large.name, "party_2");
        assert_eq!(unreadable.status, UploadStatus::Failed);
        assert!(!unreadable.can_retry, "reading it again here won't help");
        assert!(unreadable.error.contains("can't be read on this computer"));
    });
    let first = page.read_with(cx, |page, _| page.uploads[0].key);
    page.update(cx, |page, cx| page.dismiss_upload(first, cx));
    assert_eq!(page.read_with(cx, |page, _| page.uploads.len()), 2);
}

fn api_refusal(code: ApiErrorCode) -> ClientError {
    ClientError::Api {
        status: 429,
        code,
        message: String::new(),
    }
}

/// Two files queued: the first is being sent, the second waits. Checked before the first file's
/// read (of a path that does not exist) runs, so no answer but the one given here arrives.
fn queue_two_uploads(page: &Entity<ServerEmojiPage>, cx: &mut VisualTestContext) -> [u64; 2] {
    page.update(cx, |page, cx| {
        page.add_files(
            vec![
                (PathBuf::from("first.png"), Some(10)),
                (PathBuf::from("second.png"), Some(10)),
            ],
            cx,
        );
        let [first, second] = page.uploads.as_slice() else {
            panic!("both files are listed");
        };
        assert_eq!(first.status, UploadStatus::Adding);
        assert_eq!(second.status, UploadStatus::Waiting);
        [first.key, second.key]
    })
}

#[gpui::test]
fn too_many_uploads_fail_the_waiting_files_too(cx: &mut TestAppContext) {
    let (page, cx) = open_page(cx);
    let [first, _] = queue_two_uploads(&page, cx);
    page.update(cx, |page, cx| {
        page.finish_upload(first, Err(api_refusal(ApiErrorCode::RateLimited)), cx);
        assert!(!page.is_uploading);
        assert!(
            page.uploads
                .iter()
                .all(|upload| upload.status == UploadStatus::Failed && upload.can_retry),
            "the waiting file fails with the sent one, and both may be tried again"
        );
    });
}

#[gpui::test]
fn another_refusal_lets_the_next_file_go(cx: &mut TestAppContext) {
    let (page, cx) = open_page(cx);
    let [first, second] = queue_two_uploads(&page, cx);
    page.update(cx, |page, cx| {
        page.finish_upload(first, Err(api_refusal(ApiErrorCode::ServerBusy)), cx);
        let status = |key| {
            page.uploads
                .iter()
                .find(|upload| upload.key == key)
                .map(|upload| upload.status)
        };
        assert_eq!(status(first), Some(UploadStatus::Failed));
        assert_eq!(
            status(second),
            Some(UploadStatus::Adding),
            "the next file goes"
        );
    });
}

#[gpui::test]
fn a_name_the_instance_would_refuse_is_said_and_esc_puts_the_stored_one_back(
    cx: &mut TestAppContext,
) {
    let (page, cx) = open_page(cx);
    let input = page.read_with(cx, |page, _| page.name_fields[&PARTY].input.clone());
    cx.update(|window, cx| {
        input.update(cx, |input, cx| input.set_value("two words", window, cx));
    });
    page.update(cx, |page, cx| page.rename(PARTY, cx));
    let error = page.read_with(cx, |page, _| page.name_fields[&PARTY].error.clone());
    assert_eq!(
        error.as_deref(),
        Some("Use 2 to 32 letters, digits or _."),
        "refused before asking the instance"
    );
    let is_restored =
        cx.update(|window, cx| page.update(cx, |page, cx| page.restore_name(PARTY, window, cx)));
    assert!(is_restored);
    assert_eq!(
        input.read_with(cx, |input, _| input.value().to_string()),
        "party"
    );
    let is_restored_again =
        cx.update(|window, cx| page.update(cx, |page, cx| page.restore_name(PARTY, window, cx)));
    assert!(!is_restored_again, "a second Esc closes the settings");
}

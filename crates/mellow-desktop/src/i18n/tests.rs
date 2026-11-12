//! The web catalogues' own tests (`web/src/i18n/i18n.test.ts`), against the generated Rust: the
//! same inputs give the same text.

use gpui::TestAppContext;

use super::catalogue::KbOrMbOrGb;
use super::*;

const ENGLISH: Strings = Strings::new(Language::English);
const FRENCH: Strings = Strings::new(Language::French);

#[test]
fn picks_the_first_language_mellow_speaks() {
    assert_eq!(
        Language::from_locale_tags(["fr-FR", "en-US"]),
        Language::French
    );
    assert_eq!(
        Language::from_locale_tags(["de-DE", "fr-CA", "en"]),
        Language::French
    );
    assert_eq!(Language::from_locale_tags(["FR"]), Language::French);
    assert_eq!(
        Language::from_locale_tags(["fr_FR.UTF-8"]),
        Language::French
    );
    assert_eq!(
        Language::from_locale_tags(["en-GB", "fr-FR"]),
        Language::English
    );
}

#[gpui::test]
fn a_chosen_language_wins_over_the_systems(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(SystemLanguage(Language::English));
        apply_chosen_language(cx);
        assert_eq!(strings(cx).language(), Language::English);
        crate::preferences::change_preferences(cx, |preferences| {
            preferences.language = LanguageChoice::French;
        });
        apply_chosen_language(cx);
        assert_eq!(strings(cx).language(), Language::French);
        assert!(std::ptr::eq(
            crate::strings_pending_i18n::strings(cx),
            &crate::strings_pending_i18n::FRENCH
        ));
        crate::preferences::change_preferences(cx, |preferences| {
            preferences.language = LanguageChoice::System;
        });
        apply_chosen_language(cx);
        assert_eq!(strings(cx).language(), Language::English);
    });
}

#[test]
fn falls_back_to_english() {
    assert_eq!(
        Language::from_locale_tags(Vec::<String>::new()),
        Language::English
    );
    assert_eq!(
        Language::from_locale_tags(["de-DE", "es"]),
        Language::English
    );
    // Starts like French without being French.
    assert_eq!(Language::from_locale_tags(["fy-NL"]), Language::English);
}

#[test]
fn says_who_reacted_with_french_verb_agreement() {
    let english = ENGLISH.reactions();
    let french = FRENCH.reactions();
    assert_eq!(
        english.reacted_with(false, &["Ana"], 0, "👍"),
        "Ana reacted with\u{a0}👍"
    );
    assert_eq!(
        english.reacted_with(true, &["Ana", "Bo"], 3, "👍"),
        "You, Ana, Bo and 3 others reacted with\u{a0}👍"
    );
    assert_eq!(
        english.reacted_with(true, &[], 1, "👍"),
        "You and 1 other reacted with\u{a0}👍"
    );
    assert_eq!(
        french.reacted_with(false, &["Ana"], 0, "👍"),
        "Ana a réagi avec\u{a0}👍"
    );
    assert_eq!(
        french.reacted_with(true, &[], 0, "👍"),
        "Tu as réagi avec\u{a0}👍"
    );
    assert_eq!(
        french.reacted_with(true, &["Ana"], 2, "👍"),
        "Toi, Ana et 2\u{a0}autres avez réagi avec\u{a0}👍"
    );
    assert_eq!(
        french.reacted_with(false, &["Ana", "Bo"], 0, "👍"),
        "Ana et Bo ont réagi avec\u{a0}👍"
    );
    assert_eq!(english.chip("👍", 1), "👍, 1 reaction");
    assert_eq!(french.chip("👍", 2), "👍, 2\u{a0}réactions");
}

#[test]
fn counts_with_each_languages_plural_rules() {
    assert_eq!(ENGLISH.messages().attachment_count(0), "0 attachments");
    assert_eq!(ENGLISH.messages().attachment_count(1), "1 attachment");
    assert_eq!(ENGLISH.messages().attachment_count(2), "2 attachments");
    // French treats 0 and 1 as singular.
    assert_eq!(FRENCH.messages().attachment_count(0), "0 pièce jointe");
    assert_eq!(FRENCH.messages().attachment_count(1), "1 pièce jointe");
    assert_eq!(FRENCH.messages().attachment_count(2), "2 pièces jointes");
}

#[test]
fn words_the_unread_bar_with_its_count_and_date() {
    assert_eq!(
        ENGLISH
            .chat()
            .new_messages_since(1, false, "today at 14:32"),
        "1 new message since today at 14:32"
    );
    assert_eq!(
        ENGLISH.chat().new_messages_since(50, true, "21/10/2026"),
        "50+ new messages since 21/10/2026"
    );
    assert_eq!(
        FRENCH
            .chat()
            .new_messages_since(1, false, "aujourd’hui à 14:32"),
        "1 nouveau message depuis aujourd’hui à 14:32"
    );
    assert_eq!(
        FRENCH.chat().new_messages_since(1, true, "hier à 09:15"),
        "1+ nouveaux messages depuis hier à 09:15"
    );
    assert_eq!(ENGLISH.chat().new_messages_short(12, false), "12 new");
    assert_eq!(FRENCH.chat().new_messages_short(1, false), "1 nouveau");
    assert_eq!(FRENCH.chat().new_messages_short(1, true), "1+ nouveaux");
}

#[test]
fn groups_large_numbers_and_sizes_the_french_way() {
    assert_eq!(
        FRENCH.members().group_label("En ligne", 1204),
        "En ligne, 1\u{202f}204 membres"
    );
    assert_eq!(
        FRENCH.file_size().in_unit("2,4", KbOrMbOrGb::Mb),
        "2,4\u{a0}Mo"
    );
    assert_eq!(ENGLISH.file_size().in_unit("2.4", KbOrMbOrGb::Mb), "2.4 MB");
}

#[test]
fn sets_french_punctuation_apart_with_no_break_spaces() {
    assert_eq!(
        ENGLISH
            .errors()
            .with_reason("Couldn't sign in", "the login or the password is wrong."),
        "Couldn't sign in: the login or the password is wrong."
    );
    assert_eq!(
        FRENCH.errors().with_reason(
            "Impossible de te connecter",
            FRENCH.errors().reasons().invalid_credentials()
        ),
        "Impossible de te connecter\u{a0}: l’identifiant ou le mot de passe est incorrect."
    );
    assert_eq!(
        FRENCH.delete_message().title(),
        "Supprimer ce message\u{202f}?"
    );
    assert_eq!(
        FRENCH.leave_server().title("Rust Lyon"),
        "Quitter «\u{a0}Rust Lyon\u{a0}»"
    );
}

#[test]
fn lists_the_people_typing_and_mentions_up_to_the_limit() {
    assert_eq!(
        ENGLISH.typing().and_others_are_typing(1),
        " and 1 other are typing…"
    );
    assert_eq!(
        FRENCH.typing().and_others_are_typing(2),
        " et 2 autres écrivent…"
    );
    assert_eq!(
        ENGLISH.common().with_mentions("general", 1),
        "general, 1 mention"
    );
    assert_eq!(
        ENGLISH.common().with_mentions("general, unread", 3),
        "general, unread, 3 mentions"
    );
    assert_eq!(
        FRENCH.common().with_mentions("général", 100),
        "général, 99+ mentions"
    );
}

#[test]
fn says_a_duration_in_words_leaving_out_zero_parts() {
    assert_eq!(
        ENGLISH.link_previews().spoken_duration(0, 1, 30),
        "1 minute 30 seconds"
    );
    assert_eq!(
        ENGLISH.link_previews().spoken_duration(1, 0, 1),
        "1 hour 1 second"
    );
    assert_eq!(
        FRENCH.link_previews().spoken_duration(1, 2, 3),
        "1\u{a0}h 2\u{a0}min 3\u{a0}s"
    );
}

#[test]
fn narrows_optional_parameters_as_typescript_does() {
    let voice = ENGLISH.voice();
    assert_eq!(
        voice.row_label("lounge", 2, Some(5), true, true),
        "lounge, voice channel, 2 of 5 people, you're here. Show the call"
    );
    assert_eq!(
        voice.row_label("lounge", 1, None, false, false),
        "lounge, voice channel, 1 person, you can't connect"
    );
    assert_eq!(
        voice.row_label("lounge", 0, None, false, true),
        "lounge, voice channel"
    );
    assert_eq!(
        ENGLISH
            .user_card()
            .status_button_label("Ana", "Online", None),
        "Ana, Online. Set your status"
    );
    assert_eq!(
        ENGLISH
            .user_card()
            .status_button_label("Ana", "Online", Some("")),
        "Ana, Online. Set your status"
    );
    assert_eq!(
        ENGLISH
            .user_card()
            .status_button_label("Ana", "Online", Some("away")),
        "Ana, Online, away. Set your status"
    );
}

#[test]
fn formats_fractions_and_signs() {
    assert_eq!(ENGLISH.soundboard().duration(5200), "5.2 s");
    assert_eq!(FRENCH.soundboard().duration(5200), "5,2\u{a0}s");
    assert_eq!(ENGLISH.voice_settings().decibels(-12), "\u{2212}12 dB");
    assert_eq!(ENGLISH.voice_settings().equalizer_decibels(3), "+3 dB");
    assert_eq!(
        ENGLISH.voice_settings().named_keys("ShiftLeft"),
        Some("Left Shift")
    );
    assert_eq!(ENGLISH.voice_settings().named_keys("NotAKey"), None);
}

#[test]
fn keeps_lists_and_constants_from_the_catalogues() {
    let templates = FRENCH.add_server().templates();
    let community = templates.community();
    assert_eq!(community.name(), "Communauté");
    assert!(
        community.categories()[0]
            .channels
            .iter()
            .any(|channel| channel.topic.is_some())
    );
    assert_eq!(ENGLISH.search().tips().len(), 3);
    assert_eq!(ENGLISH.chat().slash_commands().bold().name(), "Bold");
}

#[gpui::test]
fn switches_language_at_run_time(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert_eq!(strings(cx).language(), Language::English);
        use_language(Language::French, cx);
        assert_eq!(strings(cx).common().cancel(), "Annuler");
        use_language(Language::English, cx);
        assert_eq!(strings(cx).common().cancel(), "Cancel");
    });
}

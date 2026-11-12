//! Appearance, the desktop's own page (the web follows the browser for both): the theme and the
//! interface language, each "Same as this computer" by default, kept in the preferences file and
//! applied at once (`theme::apply_chosen_theme`, `i18n::apply_chosen_language`).

use gpui::{
    Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Window, div,
};

use super::parts::body_text;
use crate::components::{ChoiceGroup, ChoiceOption};
use crate::i18n::strings;
use crate::preferences::{
    LanguageChoice, PreferencesStore, ThemeChoice, change_preferences, preferences,
};
use crate::settings::{SettingsPage, SettingsPageEvent};
use crate::theme::length;
use crate::theme::tokens::{COMPONENT_SETTINGS_PAGE_TITLE_GAP, COMPONENT_SETTINGS_SECTION_GAP};

const THEME_CHOICES: [ThemeChoice; 3] =
    [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];
const LANGUAGE_CHOICES: [LanguageChoice; 3] = [
    LanguageChoice::System,
    LanguageChoice::English,
    LanguageChoice::French,
];

pub struct AppearancePage {
    _preferences_observation: Subscription,
}

impl std::fmt::Debug for AppearancePage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppearancePage")
            .finish_non_exhaustive()
    }
}

impl EventEmitter<SettingsPageEvent> for AppearancePage {}

impl SettingsPage for AppearancePage {
    fn set_attention(&mut self, _: u32, _: &mut Context<Self>) {}
}

impl AppearancePage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            _preferences_observation: cx.observe_global::<PreferencesStore>(|_, cx| cx.notify()),
        }
    }
}

impl Render for AppearancePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let words = strings(cx).desktop();
        let (theme_choice, language_choice) = preferences(cx)
            .map_or_else(Default::default, |preferences| {
                (preferences.theme, preferences.language)
            });
        let theme_index = THEME_CHOICES
            .iter()
            .position(|choice| *choice == theme_choice)
            .unwrap_or_default();
        let language_index = LANGUAGE_CHOICES
            .iter()
            .position(|choice| *choice == language_choice)
            .unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_PAGE_TITLE_GAP))
                    .child(body_text(words.appearance_intro(), cx)),
            )
            .child(
                div().mt(length(COMPONENT_SETTINGS_SECTION_GAP)).child(
                    ChoiceGroup::new(
                        "appearance-theme",
                        words.theme(),
                        vec![
                            ChoiceOption::new(words.theme_system()),
                            ChoiceOption::new(words.theme_light()),
                            ChoiceOption::new(words.theme_dark()),
                        ],
                        theme_index,
                    )
                    .on_select(|index, _, cx| {
                        if let Some(choice) = THEME_CHOICES.get(index).copied() {
                            change_preferences(cx, move |preferences| preferences.theme = choice);
                        }
                    }),
                ),
            )
            .child(
                div().mt(length(COMPONENT_SETTINGS_SECTION_GAP)).child(
                    ChoiceGroup::new(
                        "appearance-language",
                        words.language(),
                        vec![
                            ChoiceOption::new(words.language_system()),
                            ChoiceOption::new(words.language_english()),
                            ChoiceOption::new(words.language_french()),
                        ],
                        language_index,
                    )
                    .on_select(|index, _, cx| {
                        if let Some(choice) = LANGUAGE_CHOICES.get(index).copied() {
                            change_preferences(cx, move |preferences| {
                                preferences.language = choice;
                            });
                        }
                    }),
                ),
            )
    }
}

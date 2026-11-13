//! Appearance, the desktop's own page (the web follows the browser for both): the theme and the
//! interface language, each "Same as this computer" by default, kept in the preferences file and
//! applied at once (`theme::apply_chosen_theme`, `i18n::apply_chosen_language`); and whether closing
//! the window keeps Mellow running in the background (ADR-0055), on by default.

use gpui::{
    App, Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Window, div,
};

use super::parts::body_text;
use crate::components::{ChoiceGroup, ChoiceOption, Switch};
use crate::dock_and_tray;
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
        let (theme_choice, language_choice, does_closing_the_window_quit) = preferences(cx)
            .map_or_else(Default::default, |preferences| {
                (
                    preferences.theme,
                    preferences.language,
                    preferences.does_closing_the_window_quit,
                )
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
            .child(
                div().mt(length(COMPONENT_SETTINGS_SECTION_GAP)).child(
                    Switch::new(
                        "appearance-keep-running",
                        !does_closing_the_window_quit,
                        words.keep_running_in_background(),
                    )
                    .description(keep_running_description(cx))
                    .on_change(|is_on, _, cx| {
                        change_preferences(cx, move |preferences| {
                            preferences.does_closing_the_window_quit = !is_on;
                        });
                    }),
                ),
            )
    }
}

/// Where Mellow stays once its window closes, on this system (ADR-0055).
fn keep_running_description(cx: &App) -> &'static str {
    let words = strings(cx).desktop();
    if cfg!(target_os = "macos") {
        words.keep_running_in_background_menu_bar()
    } else if dock_and_tray::can_show_mellow_again(cx) {
        words.keep_running_in_background_tray()
    } else {
        words.keep_running_in_background_no_tray()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext as _, FocusHandle, InputEvent as _, KeyUpEvent, Keystroke, TestAppContext,
        VisualTestContext, WindowAppearance,
    };

    use super::*;
    use crate::components::{bind_tab_navigation, with_tab_navigation};
    use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
    use crate::theme::ActiveTheme;

    struct AppearanceHarness {
        root: FocusHandle,
        page: gpui::Entity<AppearancePage>,
    }

    impl Render for AppearanceHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            with_tab_navigation(div().size_full(), &self.root).child(self.page.clone())
        }
    }

    /// A full key press: GPUI activates a focused control on the release.
    fn press(cx: &mut VisualTestContext, key: &'static str) {
        cx.simulate_keystrokes(key);
        let keystroke = Keystroke::parse(key).unwrap();
        cx.update(|window, cx| {
            window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
        });
        cx.run_until_parked();
    }

    fn does_closing_the_window_quit(cx: &mut VisualTestContext) -> bool {
        cx.update(|_, cx| {
            preferences(cx).is_some_and(|preferences| preferences.does_closing_the_window_quit)
        })
    }

    #[gpui::test]
    fn keeping_mellow_running_is_on_by_default_and_turns_off_from_the_keyboard(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
            cx.set_global(ActiveStrings(&ENGLISH));
            bind_tab_navigation(cx);
        });
        let (harness, cx) = cx.add_window_view(|_, cx| AppearanceHarness {
            root: cx.focus_handle(),
            page: cx.new(AppearancePage::new),
        });
        let root = harness.read_with(cx, |harness, _| harness.root.clone());
        cx.update(|window, cx| {
            window.activate_window();
            window.focus(&root, cx);
        });
        cx.run_until_parked();
        assert!(!does_closing_the_window_quit(cx), "on by default");

        // The theme's and the language's groups come first, one stop each.
        press(cx, "tab");
        press(cx, "tab");
        press(cx, "tab");
        press(cx, "space");
        assert!(
            does_closing_the_window_quit(cx),
            "off: the close button quits"
        );
        press(cx, "space");
        assert!(!does_closing_the_window_quit(cx));
    }
}

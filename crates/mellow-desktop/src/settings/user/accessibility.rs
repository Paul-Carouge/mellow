//! Accessibility (`AccessibilitySettings.svelte`, docs/design/web-animation-setting.md): when
//! animated pictures and Melli play, kept in the preferences file. While the system asks to reduce
//! motion, a sentence under the choice says what that does to "Animate on hover", so a still
//! avatar is never a mystery. Interface motion keeps following the system alone.

use gpui::{
    Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Window, div, prelude::FluentBuilder as _,
};

use super::parts::body_text;
use crate::components::{ChoiceGroup, ChoiceOption, with_type};
use crate::i18n::strings;
use crate::preferences::{
    PictureAnimationChoice, PreferencesStore, change_preferences, preferences,
};
use crate::settings::{SettingsPage, SettingsPageEvent};
use crate::theme::tokens::{COMPONENT_SETTINGS_SECTION_GAP, SPACE_XS, TYPE_BODY};
use crate::theme::{color, length, theme};

const PICTURE_ANIMATION_CHOICES: [PictureAnimationChoice; 2] = [
    PictureAnimationChoice::OnHover,
    PictureAnimationChoice::Always,
];

pub struct AccessibilityPage {
    _preferences_observation: Subscription,
}

impl std::fmt::Debug for AccessibilityPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccessibilityPage")
            .finish_non_exhaustive()
    }
}

impl EventEmitter<SettingsPageEvent> for AccessibilityPage {}

impl SettingsPage for AccessibilityPage {
    fn set_attention(&mut self, _: u32, _: &mut Context<Self>) {}
}

impl AccessibilityPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            _preferences_observation: cx.observe_global::<PreferencesStore>(|_, cx| cx.notify()),
        }
    }
}

impl Render for AccessibilityPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let words = catalogue.accessibility_settings();
        let choice = preferences(cx)
            .map(|preferences| preferences.picture_animation)
            .unwrap_or_default();
        let checked_index = PICTURE_ANIMATION_CHOICES
            .iter()
            .position(|known| *known == choice)
            .unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .child(div().child(body_text(catalogue.desktop().accessibility_intro(), cx)))
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_SECTION_GAP))
                    .child(
                        ChoiceGroup::new(
                            "accessibility-picture-animation",
                            words.animated_pictures(),
                            vec![
                                ChoiceOption::new(words.animate_on_hover())
                                    .description(words.animate_on_hover_description()),
                                ChoiceOption::new(words.always_animate())
                                    .description(words.always_animate_description()),
                            ],
                            checked_index,
                        )
                        .on_select(|index, _, cx| {
                            if let Some(picked) = PICTURE_ANIMATION_CHOICES.get(index).copied() {
                                change_preferences(cx, move |preferences| {
                                    preferences.picture_animation = picked;
                                });
                            }
                        }),
                    )
                    // Only while the system asks: it explains a still avatar where it is fixed.
                    .when(cx.reduce_motion(), |section| {
                        section.child(
                            with_type(div(), TYPE_BODY)
                                .mt(length(SPACE_XS))
                                .text_color(color(theme(cx).component_text_field_helper_text))
                                .child(words.reduced_motion_note()),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, FocusHandle, TestAppContext, VisualTestContext, WindowAppearance};

    use super::*;
    use crate::components::{bind_tab_navigation, with_tab_navigation};
    use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
    use crate::theme::ActiveTheme;

    struct AccessibilityHarness {
        root: FocusHandle,
        page: gpui::Entity<AccessibilityPage>,
    }

    impl Render for AccessibilityHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            with_tab_navigation(div().size_full(), &self.root).child(self.page.clone())
        }
    }

    fn chosen(cx: &mut VisualTestContext) -> PictureAnimationChoice {
        cx.update(|_, cx| {
            preferences(cx)
                .map(|preferences| preferences.picture_animation)
                .unwrap_or_default()
        })
    }

    #[gpui::test]
    fn animate_on_hover_is_the_default_and_always_animate_is_chosen_from_the_keyboard(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
            cx.set_global(ActiveStrings(&ENGLISH));
            bind_tab_navigation(cx);
        });
        let (harness, cx) = cx.add_window_view(|_, cx| AccessibilityHarness {
            root: cx.focus_handle(),
            page: cx.new(AccessibilityPage::new),
        });
        let root = harness.read_with(cx, |harness, _| harness.root.clone());
        cx.update(|window, cx| {
            window.activate_window();
            window.focus(&root, cx);
        });
        cx.run_until_parked();
        assert_eq!(chosen(cx), PictureAnimationChoice::OnHover);

        cx.simulate_keystrokes("tab down");
        cx.run_until_parked();
        assert_eq!(chosen(cx), PictureAnimationChoice::Always);
        cx.simulate_keystrokes("up");
        cx.run_until_parked();
        assert_eq!(chosen(cx), PictureAnimationChoice::OnHover);
    }
}

//! About (`UserSettings.svelte`'s last section): the app's name, its licence and where its code
//! lives. The web's "Third-party licences" link opens the notices the instance serves with the
//! web client, which do not describe this app, so the desktop leaves it out.

use gpui::{
    Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Window, div,
};

use super::parts::{body_text, section_label};
use crate::components::{Button, ButtonVariant, with_type};
use crate::i18n::strings;
use crate::settings::{SettingsPage, SettingsPageEvent};
use crate::theme::tokens::{
    COMPONENT_SETTINGS_ACTION_GAP, COMPONENT_SETTINGS_PAGE_TITLE_GAP,
    COMPONENT_SETTINGS_SECTION_GAP, COMPONENT_SETTINGS_SECTION_LABEL_GAP, TYPE_HEADING_MD,
};
use crate::theme::{color, length, theme};

const SOURCE_CODE_URL: &str = "https://github.com/Paul-Carouge/mellow";

#[derive(Debug, Default)]
pub struct AboutPage;

impl EventEmitter<SettingsPageEvent> for AboutPage {}

impl SettingsPage for AboutPage {
    fn set_attention(&mut self, _: u32, _: &mut Context<Self>) {}
}

impl Render for AboutPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let words = catalogue.settings();
        div()
            .flex()
            .flex_col()
            .items_start()
            .child(
                with_type(div(), TYPE_HEADING_MD)
                    .mt(length(COMPONENT_SETTINGS_PAGE_TITLE_GAP))
                    .text_color(color(theme(cx).component_settings_app_name_text))
                    .child(catalogue.desktop().desktop_app()),
            )
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_SECTION_GAP))
                    .child(section_label("about-source-code", words.source_code(), cx)),
            )
            .child(body_text(words.licence(), cx).mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP)))
            .child(
                div().mt(length(COMPONENT_SETTINGS_ACTION_GAP)).child(
                    Button::new("about-source-code-link", words.source_code_on_git_hub())
                        .variant(ButtonVariant::Ghost)
                        .on_click(|_, cx| cx.open_url(SOURCE_CODE_URL)),
                ),
            )
    }
}

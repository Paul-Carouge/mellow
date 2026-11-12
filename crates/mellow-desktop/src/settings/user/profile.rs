//! Profile (`ProfileSettings.svelte` and `ProfileDetailsSettings.svelte`, DESIGN.md §5.9,
//! ADR-0021, ADR-0050): the avatar and banner editors, then "About me" and links, saved with
//! their own button since a text is typed in several goes. The instance crops, resizes and
//! compresses a picture before answering, a second or two for a large GIF, so the buttons say so
//! meanwhile; one change runs at a time.
//!
//! shortcut: a chosen picture is sent as it is, so the instance keeps its centred square (avatar)
//! or band (banner): the crop dialog (`PictureCropDialog.svelte`) and the live profile card
//! preview beside the editors are left out until the desktop has the crop dialog, which the
//! server icon needs too.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render,
    SharedString, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState, Textarea, TextareaState};
use mellow_client_core::ClientError;
use mellow_client_core::shared_logic::user_settings::{
    MAXIMUM_PROFILE_IMAGE_BYTES, ProfileDraft, ProfileImageFileProblem, ProfileLinkDraft,
    ProfileLinkProblem, link_problem, profile_image_file_problem,
};
use mellow_protocol::{
    ApiErrorCode, BannerCropQuery, IconCropQuery, MAXIMUM_ABOUT_ME_LENGTH,
    MAXIMUM_PROFILE_LINK_LABEL_LENGTH, MAXIMUM_PROFILE_LINKS, UserProfile,
};

use super::parts::{body_text, error_line, section_label};
use crate::components::{Button, ButtonVariant, IconButton, TextField, focus_ring, with_type};
use crate::i18n::{Strings, strings};
use crate::messages::file_size_label;
use crate::session::Session;
use crate::settings::{SettingsPage, SettingsPageEvent, saved_notice};
use crate::sign_in_form::describe_error;
use crate::strings_pending_i18n::strings as pending_strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP, COMPONENT_SETTINGS_ERROR_ICON_SIZE,
    COMPONENT_SETTINGS_PAGE_TITLE_GAP, COMPONENT_SETTINGS_SECTION_GAP,
    COMPONENT_SETTINGS_SECTION_LABEL_GAP, COMPONENT_TEXT_FIELD_BORDER_WIDTH,
    COMPONENT_TEXT_FIELD_FIELD_GAP, COMPONENT_TEXT_FIELD_LABEL_GAP,
    COMPONENT_TEXT_FIELD_PADDING_INLINE, COMPONENT_TEXT_FIELD_RADIUS, SIZE_HIT_WEB_MIN, SPACE_XS,
    TYPE_BODY_LG, TYPE_BODY_SM, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

/// Rows of the "About me" field: four, growing to eight before it scrolls.
const ABOUT_ME_ROWS: (usize, usize) = (4, 8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageKind {
    Avatar,
    Banner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageAction {
    Upload,
    Remove,
}

/// Why a picked file was not sent.
enum PictureReadProblem {
    File(ProfileImageFileProblem),
    Unreadable,
}

/// One link row: its address and its name.
struct LinkRow {
    url: Entity<InputState>,
    label: Entity<InputState>,
}

enum DetailsLoad {
    Loading,
    Failed,
    /// The answer, turned into fields when next drawn (they need the window).
    Arrived(UserProfile),
    Ready,
}

pub struct ProfilePage {
    session: Entity<Session>,
    busy: Option<(ImageKind, ImageAction)>,
    image_error: Option<(ImageKind, SharedString)>,
    image_notice: Option<(ImageKind, SharedString)>,
    change_focus: [FocusHandle; 2],
    details: DetailsLoad,
    saved: ProfileDraft,
    about_input: Option<Entity<TextareaState>>,
    link_rows: Vec<LinkRow>,
    field_subscriptions: Vec<Subscription>,
    is_saving: bool,
    details_error: Option<SharedString>,
    details_notice: Option<SharedString>,
    add_link_focus: FocusHandle,
    focus_on_render: Option<FocusHandle>,
    _session_observation: Subscription,
}

impl EventEmitter<SettingsPageEvent> for ProfilePage {}

impl SettingsPage for ProfilePage {
    fn set_attention(&mut self, _: u32, _: &mut Context<Self>) {}
}

impl std::fmt::Debug for ProfilePage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProfilePage")
            .field("busy", &self.busy)
            .field("is_saving", &self.is_saving)
            .finish_non_exhaustive()
    }
}

impl ImageKind {
    fn index(self) -> usize {
        match self {
            Self::Avatar => 0,
            Self::Banner => 1,
        }
    }
}

/// The words of one picture editor.
struct ImageWords {
    title: &'static str,
    empty_text: &'static str,
    saved: &'static str,
    removed: &'static str,
    change: &'static str,
    uploading: &'static str,
    remove: &'static str,
    removing: &'static str,
    change_failed: &'static str,
    remove_failed: &'static str,
}

fn image_words(kind: ImageKind, catalogue: Strings) -> ImageWords {
    let profile = catalogue.profile();
    match kind {
        ImageKind::Avatar => {
            let words = profile.avatar();
            ImageWords {
                title: words.title(),
                empty_text: words.empty_text(),
                saved: words.saved(),
                removed: words.removed(),
                change: words.change(),
                uploading: words.uploading(),
                remove: words.remove(),
                removing: words.removing(),
                change_failed: words.change_failed(),
                remove_failed: words.remove_failed(),
            }
        }
        ImageKind::Banner => {
            let words = profile.banner();
            ImageWords {
                title: words.title(),
                empty_text: words.empty_text(),
                saved: words.saved(),
                removed: words.removed(),
                change: words.change(),
                uploading: words.uploading(),
                remove: words.remove(),
                removing: words.removing(),
                change_failed: words.change_failed(),
                remove_failed: words.remove_failed(),
            }
        }
    }
}

/// Reads a picked file off the UI thread, never more than the instance takes.
fn read_picture(path: &Path) -> Result<Vec<u8>, PictureReadProblem> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let size = std::fs::metadata(path)
        .map_err(|_| PictureReadProblem::Unreadable)?
        .len();
    if let Some(problem) = profile_image_file_problem(&file_name, size) {
        return Err(PictureReadProblem::File(problem));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| {
            file.take(MAXIMUM_PROFILE_IMAGE_BYTES + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| PictureReadProblem::Unreadable)?;
    // The file grew since its size was read.
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAXIMUM_PROFILE_IMAGE_BYTES {
        return Err(PictureReadProblem::File(ProfileImageFileProblem::TooLarge));
    }
    Ok(bytes)
}

fn too_large_reason(cx: &App) -> String {
    let catalogue = strings(cx);
    catalogue
        .profile()
        .too_large(&file_size_label(MAXIMUM_PROFILE_IMAGE_BYTES, catalogue))
}

/// Says what to do about each refusal, in the words of this page rather than the generic ones.
fn describe_image_failure(error: &ClientError, summary: &str, cx: &App) -> String {
    let pending = pending_strings(cx);
    match error {
        ClientError::Api {
            code: ApiErrorCode::ValidationFailed,
            ..
        } => (pending.error_with_reason)(summary, strings(cx).profile().unreadable()),
        ClientError::FileTooLarge => (pending.error_with_reason)(summary, &too_large_reason(cx)),
        _ => describe_error(error, summary, pending),
    }
}

impl ProfilePage {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe(&session, |_, _, cx| cx.notify());
        let mut page = Self {
            session,
            busy: None,
            image_error: None,
            image_notice: None,
            change_focus: [cx.focus_handle(), cx.focus_handle()],
            details: DetailsLoad::Loading,
            saved: ProfileDraft::default(),
            about_input: None,
            link_rows: Vec::new(),
            field_subscriptions: Vec::new(),
            is_saving: false,
            details_error: None,
            details_notice: None,
            add_link_focus: cx.focus_handle(),
            focus_on_render: None,
            _session_observation: observation,
        };
        page.load_details(cx);
        page
    }

    // --- Pictures ---------------------------------------------------------------------------

    fn has_picture(&self, kind: ImageKind, cx: &App) -> bool {
        self.session
            .read(cx)
            .state()
            .current_user()
            .is_some_and(|user| match kind {
                ImageKind::Avatar => user.avatar.is_some(),
                ImageKind::Banner => user.banner.is_some(),
            })
    }

    fn choose_picture(&mut self, kind: ImageKind, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |page, cx| {
            let path: PathBuf = match picked.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => path,
                    None => return,
                },
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "the file picker failed");
                    return;
                }
            };
            let read = cx
                .background_spawn(async move { read_picture(&path) })
                .await;
            let _ = page.update(cx, |page, cx| page.upload(kind, read, cx));
        })
        .detach();
    }

    fn upload(
        &mut self,
        kind: ImageKind,
        read: Result<Vec<u8>, PictureReadProblem>,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        let catalogue = strings(cx);
        let summary = image_words(kind, catalogue).change_failed;
        let bytes = match read {
            Ok(bytes) => bytes,
            Err(problem) => {
                let reason = match problem {
                    PictureReadProblem::File(ProfileImageFileProblem::NotAPicture) => {
                        catalogue.profile().not_a_picture().to_owned()
                    }
                    PictureReadProblem::File(ProfileImageFileProblem::TooLarge) => {
                        too_large_reason(cx)
                    }
                    PictureReadProblem::Unreadable => catalogue.profile().unreadable().to_owned(),
                };
                self.image_notice = None;
                self.image_error = Some((
                    kind,
                    (pending_strings(cx).error_with_reason)(summary, &reason).into(),
                ));
                cx.notify();
                return;
            }
        };
        self.run(kind, ImageAction::Upload, cx, move |rest| async move {
            match kind {
                ImageKind::Avatar => rest.set_avatar(bytes, &IconCropQuery::default()).await,
                ImageKind::Banner => rest.set_banner(bytes, &BannerCropQuery::default()).await,
            }
        });
    }

    fn remove(&mut self, kind: ImageKind, cx: &mut Context<Self>) {
        self.run(kind, ImageAction::Remove, cx, move |rest| async move {
            match kind {
                ImageKind::Avatar => rest.remove_avatar().await,
                ImageKind::Banner => rest.remove_banner().await,
            }
        });
    }

    fn run<Request>(
        &mut self,
        kind: ImageKind,
        action: ImageAction,
        cx: &mut Context<Self>,
        request: impl FnOnce(mellow_client_core::RestClient) -> Request,
    ) where
        Request: std::future::Future<Output = Result<mellow_protocol::CurrentUser, ClientError>>
            + Send
            + 'static,
    {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some((kind, action));
        self.image_error = None;
        self.image_notice = None;
        cx.notify();
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_current_user_change(
                match action {
                    ImageAction::Upload => "change profile picture",
                    ImageAction::Remove => "remove profile picture",
                },
                request,
                cx,
                move |outcome, cx| {
                    let _ =
                        page.update(cx, |page, cx| page.finish_image(kind, action, outcome, cx));
                },
            );
        });
    }

    fn finish_image(
        &mut self,
        kind: ImageKind,
        action: ImageAction,
        outcome: Result<(), ClientError>,
        cx: &mut Context<Self>,
    ) {
        self.busy = None;
        let words = image_words(kind, strings(cx));
        match outcome {
            Ok(()) => {
                let text = match action {
                    ImageAction::Upload => words.saved,
                    ImageAction::Remove => words.removed,
                };
                self.image_notice = Some((kind, text.into()));
                // The Remove button is gone once it worked: focus stays in the editor.
                if action == ImageAction::Remove {
                    self.focus_on_render = Some(self.change_focus[kind.index()].clone());
                }
            }
            Err(error) => {
                let summary = match action {
                    ImageAction::Upload => words.change_failed,
                    ImageAction::Remove => words.remove_failed,
                };
                self.image_error = Some((kind, describe_image_failure(&error, summary, cx).into()));
            }
        }
        cx.notify();
    }

    // --- About me and links -----------------------------------------------------------------

    fn load_details(&mut self, cx: &mut Context<Self>) {
        self.details = DetailsLoad::Loading;
        cx.notify();
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_account_request(
                "load own profile",
                |rest| async move { rest.my_profile().await },
                cx,
                move |answer, cx| {
                    let _ = page.update(cx, |page, cx| {
                        page.details = answer.map_or(DetailsLoad::Failed, DetailsLoad::Arrived);
                        cx.notify();
                    });
                },
            );
        });
    }

    fn watch(&mut self, input: &Entity<InputState>, cx: &mut Context<Self>) {
        let subscription = cx.subscribe(input, |page, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                page.details_notice = None;
                cx.notify();
            }
        });
        self.field_subscriptions.push(subscription);
    }

    fn new_link_row(
        &mut self,
        link: &ProfileLinkDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> LinkRow {
        let placeholder = strings(cx).profile_details().link_name_placeholder();
        let url = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            input.set_value(link.url.clone(), window, cx);
            input
        });
        let label = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .placeholder(placeholder)
                .validate(|text, _| text.chars().count() <= MAXIMUM_PROFILE_LINK_LABEL_LENGTH);
            input.set_value(link.label.clone(), window, cx);
            input
        });
        self.watch(&url, cx);
        self.watch(&label, cx);
        LinkRow { url, label }
    }

    /// Puts `draft` in the fields, replacing what they held.
    fn fill_fields(&mut self, draft: &ProfileDraft, window: &mut Window, cx: &mut Context<Self>) {
        self.field_subscriptions.clear();
        let about = cx.new(|cx| {
            let mut input =
                TextareaState::new(window, cx).auto_grow(ABOUT_ME_ROWS.0, ABOUT_ME_ROWS.1);
            input.set_value(draft.about_me.clone(), window, cx);
            input
        });
        let subscription = cx.subscribe(&about, |page, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                page.details_notice = None;
                cx.notify();
            }
        });
        self.field_subscriptions.push(subscription);
        self.about_input = Some(about);
        let rows = draft
            .links
            .iter()
            .map(|link| self.new_link_row(link, window, cx))
            .collect();
        self.link_rows = rows;
    }

    fn draft(&self, cx: &App) -> ProfileDraft {
        ProfileDraft {
            about_me: self
                .about_input
                .as_ref()
                .map(|input| input.read(cx).value().to_string())
                .unwrap_or_default(),
            links: self
                .link_rows
                .iter()
                .map(|row| ProfileLinkDraft {
                    url: row.url.read(cx).value().to_string(),
                    label: row.label.read(cx).value().to_string(),
                })
                .collect(),
        }
    }

    fn can_save_details(&self, cx: &App) -> bool {
        let draft = self.draft(cx);
        matches!(self.details, DetailsLoad::Ready)
            && !self.is_saving
            && draft.request() != self.saved.request()
            && draft.can_save()
    }

    fn add_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.link_rows.len() >= MAXIMUM_PROFILE_LINKS {
            return;
        }
        let row = self.new_link_row(&ProfileLinkDraft::default(), window, cx);
        row.url.update(cx, |input, cx| input.focus(window, cx));
        self.link_rows.push(row);
        self.details_notice = None;
        cx.notify();
    }

    fn remove_link(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.link_rows.len() {
            return;
        }
        self.link_rows.remove(index);
        self.details_notice = None;
        // Focus stays in the list: the row now in its place, else the one above, else "Add".
        match self.link_rows.get(index).or_else(|| self.link_rows.last()) {
            Some(row) => row.url.update(cx, |input, cx| input.focus(window, cx)),
            None => window.focus(&self.add_link_focus, cx),
        }
        cx.notify();
    }

    fn save_details(&mut self, cx: &mut Context<Self>) {
        if !self.can_save_details(cx) {
            return;
        }
        self.is_saving = true;
        self.details_error = None;
        self.details_notice = None;
        cx.notify();
        let request = self.draft(cx).request();
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_account_request(
                "save own profile",
                move |rest| async move { rest.update_my_profile(&request).await },
                cx,
                move |answer, cx| {
                    let _ = page.update(cx, |page, cx| page.finish_details(answer, cx));
                },
            );
        });
    }

    fn finish_details(&mut self, answer: Result<UserProfile, ClientError>, cx: &mut Context<Self>) {
        self.is_saving = false;
        let words = strings(cx).profile_details();
        match answer {
            Ok(profile) => {
                self.details_notice = Some(words.saved().into());
                // The fields take what was stored when next drawn.
                self.details = DetailsLoad::Arrived(profile);
            }
            Err(error) => {
                self.details_error =
                    Some(describe_error(&error, words.save_failed(), pending_strings(cx)).into());
            }
        }
        cx.notify();
    }

    // --- Drawing ----------------------------------------------------------------------------

    fn render_image_editor(&self, kind: ImageKind, cx: &mut Context<Self>) -> AnyElement {
        let words = image_words(kind, strings(cx));
        let has_picture = self.has_picture(kind, cx);
        let is_this_busy = self.busy.map(|(busy_kind, _)| busy_kind) == Some(kind);
        let busy_action = self.busy.map(|(_, action)| action);
        let id = match kind {
            ImageKind::Avatar => "profile-avatar",
            ImageKind::Banner => "profile-banner",
        };
        let change_page = cx.entity();
        let remove_page = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
            .child(section_label(
                SharedString::from(format!("{id}-title")),
                words.title,
                cx,
            ))
            .when(!has_picture, |editor| {
                editor.child(body_text(words.empty_text, cx))
            })
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                    .flex()
                    .flex_wrap()
                    .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
                    .child(
                        Button::new(
                            SharedString::from(format!("{id}-change")),
                            if is_this_busy && busy_action == Some(ImageAction::Upload) {
                                words.uploading
                            } else {
                                words.change
                            },
                        )
                        .disabled(self.busy.is_some())
                        .track_focus(&self.change_focus[kind.index()])
                        .on_click(move |_, cx| {
                            change_page.update(cx, |page, cx| page.choose_picture(kind, cx));
                        }),
                    )
                    .when(has_picture, |buttons| {
                        buttons.child(
                            Button::new(
                                SharedString::from(format!("{id}-remove")),
                                if is_this_busy && busy_action == Some(ImageAction::Remove) {
                                    words.removing
                                } else {
                                    words.remove
                                },
                            )
                            .disabled(self.busy.is_some())
                            .on_click(move |_, cx| {
                                remove_page.update(cx, |page, cx| page.remove(kind, cx));
                            }),
                        )
                    }),
            )
            .child(error_line(
                SharedString::from(format!("{id}-error")),
                self.image_error
                    .as_ref()
                    .filter(|(error_kind, _)| *error_kind == kind)
                    .map(|(_, text)| text.clone()),
                cx,
            ))
            .child(saved_notice(
                match kind {
                    ImageKind::Avatar => "profile-avatar-notice",
                    ImageKind::Banner => "profile-banner-notice",
                },
                self.image_notice
                    .as_ref()
                    .filter(|(notice_kind, _)| *notice_kind == kind)
                    .map(|(_, text)| text.clone()),
                cx,
            ))
            .into_any_element()
    }

    fn render_about_me(input: &Entity<TextareaState>, window: &Window, cx: &App) -> AnyElement {
        let tokens = theme(cx);
        let words = strings(cx).profile_details();
        let draft_length = input.read(cx).value().trim().chars().count();
        let is_too_long = draft_length > MAXIMUM_ABOUT_ME_LENGTH;
        let maximum = i64::try_from(MAXIMUM_ABOUT_ME_LENGTH).unwrap_or(i64::MAX);
        let left = maximum - i64::try_from(draft_length).unwrap_or(i64::MAX);
        let helper = words.about_me_helper(maximum);
        let remaining = words.remaining(left);
        let is_focused = input.read(cx).focus_handle(cx).is_focused(window);
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_TEXT_FIELD_LABEL_GAP))
            .child(
                with_type(div(), TYPE_CATEGORY)
                    .text_color(color(tokens.component_text_field_label_text))
                    .child(words.about_me()),
            )
            .child(
                with_type(div(), TYPE_BODY_LG)
                    .relative()
                    .px(length(COMPONENT_TEXT_FIELD_PADDING_INLINE))
                    .py(length(SPACE_XS))
                    .rounded(length(COMPONENT_TEXT_FIELD_RADIUS))
                    .border(length(COMPONENT_TEXT_FIELD_BORDER_WIDTH))
                    .border_color(color(if is_too_long {
                        tokens.color_text_danger
                    } else {
                        tokens.component_text_field_border
                    }))
                    .bg(color(tokens.component_text_field_background))
                    .text_color(color(tokens.component_text_field_text))
                    .child(
                        Textarea::new(input)
                            .appearance(false)
                            .aria_label(SharedString::from(format!(
                                "{}, {helper} {remaining}",
                                words.about_me()
                            ))),
                    )
                    .when(is_focused, |field| {
                        field.child(focus_ring(COMPONENT_TEXT_FIELD_RADIUS, cx))
                    }),
            )
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .flex()
                    .justify_between()
                    .gap(length(SPACE_XS))
                    .text_color(color(tokens.component_text_field_helper_text))
                    .child(helper)
                    .child(
                        div()
                            .id("profile-about-count")
                            .flex_none()
                            .when(is_too_long, |count| {
                                count.text_color(color(tokens.color_text_danger))
                            })
                            .child(remaining),
                    ),
            )
            .into_any_element()
    }

    fn render_link_row(&self, index: usize, row: &LinkRow, cx: &mut Context<Self>) -> AnyElement {
        let words = strings(cx).profile_details();
        let position = i64::try_from(index + 1).unwrap_or(i64::MAX);
        let problem = link_problem(&ProfileLinkDraft {
            url: row.url.read(cx).value().to_string(),
            label: row.label.read(cx).value().to_string(),
        });
        let mut url_field = TextField::new(
            SharedString::from(format!("profile-link-{index}-url")),
            words.link_address(position),
            &row.url,
        )
        .read_only(self.is_saving);
        if let Some(problem) = problem {
            url_field = url_field.error(match problem {
                ProfileLinkProblem::NotHttps => words.not_https(),
                ProfileLinkProblem::TooLong => words.too_long(),
            });
        }
        let page = cx.entity();
        div()
            .flex()
            .items_end()
            .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
            // The address takes the larger share, as on the web.
            .child(div().flex_1().min_w_0().child(url_field))
            .child(
                div().flex_1().min_w_0().child(
                    TextField::new(
                        SharedString::from(format!("profile-link-{index}-label")),
                        words.link_name(position),
                        &row.label,
                    )
                    .read_only(self.is_saving),
                ),
            )
            .child(
                IconButton::new(
                    SharedString::from(format!("profile-link-{index}-remove")),
                    IconName::X,
                    words.remove_link(position),
                )
                .sizes(SIZE_HIT_WEB_MIN, COMPONENT_SETTINGS_ERROR_ICON_SIZE)
                .on_click(move |window, cx| {
                    page.update(cx, |page, cx| page.remove_link(index, window, cx));
                }),
            )
            .into_any_element()
    }

    fn render_links(&self, cx: &mut Context<Self>) -> AnyElement {
        let words = strings(cx).profile_details();
        let rows: Vec<AnyElement> = self
            .link_rows
            .iter()
            .enumerate()
            .map(|(index, row)| self.render_link_row(index, row, cx))
            .collect();
        let maximum_links = i64::try_from(MAXIMUM_PROFILE_LINKS).unwrap_or(i64::MAX);
        let can_add = self.link_rows.len() < MAXIMUM_PROFILE_LINKS;
        let add_page = cx.entity();
        div()
            .mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
            .flex()
            .flex_col()
            .gap(length(COMPONENT_TEXT_FIELD_FIELD_GAP))
            .child(section_label("profile-links-title", words.links(), cx))
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .text_color(color(theme(cx).component_text_field_helper_text))
                    .child(words.links_helper(maximum_links)),
            )
            .children(rows)
            .when(can_add, |links| {
                links.child(
                    div().child(
                        Button::new("profile-add-link", words.add_link())
                            .variant(ButtonVariant::Ghost)
                            .track_focus(&self.add_link_focus)
                            .on_click(move |window, cx| {
                                add_page.update(cx, |page, cx| page.add_link(window, cx));
                            }),
                    ),
                )
            })
            .into_any_element()
    }

    fn render_details(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let words = strings(cx).profile_details();
        let editor = div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
            .child(section_label(
                "profile-details-title",
                words.editor_title(),
                cx,
            ))
            .child(body_text(words.editor_intro(), cx));
        match &self.details {
            DetailsLoad::Loading => {
                return editor
                    .child(body_text(words.loading(), cx))
                    .into_any_element();
            }
            DetailsLoad::Failed => {
                let page = cx.entity();
                return editor
                    .child(error_line(
                        "profile-details-load-error",
                        Some(words.load_editor_failed().into()),
                        cx,
                    ))
                    .child(
                        div().child(
                            Button::new(
                                "profile-details-try-again",
                                strings(cx).common().try_again(),
                            )
                            .on_click(move |_, cx| page.update(cx, Self::load_details)),
                        ),
                    )
                    .into_any_element();
            }
            DetailsLoad::Arrived(_) | DetailsLoad::Ready => {}
        }
        let Some(about) = self.about_input.clone() else {
            return editor.into_any_element();
        };
        let save_page = cx.entity();
        editor
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                    .child(Self::render_about_me(&about, window, cx)),
            )
            .child(self.render_links(cx))
            .child(error_line(
                "profile-details-error",
                self.details_error.clone(),
                cx,
            ))
            .child(
                div().flex().justify_end().child(
                    Button::new(
                        "profile-details-save",
                        if self.is_saving {
                            words.saving()
                        } else {
                            words.save()
                        },
                    )
                    .variant(ButtonVariant::Primary)
                    .disabled(!self.can_save_details(cx))
                    .on_click(move |_, cx| save_page.update(cx, Self::save_details)),
                ),
            )
            .child(saved_notice(
                "profile-details-notice",
                self.details_notice.clone(),
                cx,
            ))
            .into_any_element()
    }
}

impl Render for ProfilePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let DetailsLoad::Arrived(profile) =
            std::mem::replace(&mut self.details, DetailsLoad::Ready)
        {
            self.saved = ProfileDraft::of_profile(&profile);
            let saved = self.saved.clone();
            self.fill_fields(&saved, window, cx);
        }
        if let Some(focus) = self.focus_on_render.take() {
            window.focus(&focus, cx);
        }
        let catalogue = strings(cx);
        let size_label = file_size_label(MAXIMUM_PROFILE_IMAGE_BYTES, catalogue);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                    .child(body_text(catalogue.profile().formats(&size_label), cx)),
            )
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_PAGE_TITLE_GAP))
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_SETTINGS_SECTION_GAP))
                    .child(self.render_image_editor(ImageKind::Avatar, cx))
                    .child(self.render_image_editor(ImageKind::Banner, cx))
                    .child(self.render_details(window, cx)),
            )
    }
}

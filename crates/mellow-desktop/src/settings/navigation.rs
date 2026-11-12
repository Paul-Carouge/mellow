//! What every settings screen keeps of its navigation (`ServerSettings.svelte`,
//! `ChannelSettings.svelte`): the chosen section, the page holding unsaved changes and how often
//! leaving it was refused, which half shows when narrow, and the focus handles of the sheet.

use std::collections::HashMap;
use std::hash::Hash;

use gpui::{
    AnyView, App, Context, Entity, EventEmitter, FocusHandle, Render, Subscription, Window,
};
use mellow_client_core::shared_logic::server_settings::must_stay_on_page;

/// What a page tells its screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPageEvent {
    /// Its form differs from what is stored (true), or not any more (false).
    UnsavedChanges(bool),
}

/// A section's page: a view that reports unsaved changes and shakes its bar when asked.
pub trait SettingsPage: Render + EventEmitter<SettingsPageEvent> {
    /// Raised by each refused attempt to leave the page; its unsaved changes bar shakes.
    fn set_attention(&mut self, attention: u32, cx: &mut Context<Self>);
}

/// Passes a refusal count on to the page.
type AttentionSetter = Box<dyn Fn(u32, &mut App)>;

/// The page shown, kept while its section stays chosen.
pub struct ShownPage<Section> {
    pub section: Section,
    pub view: AnyView,
    set_attention: AttentionSetter,
    _subscription: Subscription,
}

impl<Section: std::fmt::Debug> std::fmt::Debug for ShownPage<Section> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ShownPage")
            .field("section", &self.section)
            .finish_non_exhaustive()
    }
}

/// Wraps `page` for `Host`, whose `navigation` hears its unsaved changes.
pub fn shown_page<Host: 'static, Section: Copy + PartialEq + Eq + Hash + 'static, Page>(
    section: Section,
    page: Entity<Page>,
    navigation: fn(&mut Host) -> &mut SettingsNavigation<Section>,
    cx: &mut Context<Host>,
) -> ShownPage<Section>
where
    Page: SettingsPage + 'static,
{
    let subscription = cx.subscribe(&page, move |host, _, event: &SettingsPageEvent, cx| {
        let SettingsPageEvent::UnsavedChanges(has_changes) = *event;
        navigation(host).note_unsaved_changes(section, has_changes);
        cx.notify();
    });
    let attention_target = page.clone();
    ShownPage {
        section,
        view: page.into(),
        set_attention: Box::new(move |attention, cx| {
            attention_target.update(cx, |page, cx| page.set_attention(attention, cx));
        }),
        _subscription: subscription,
    }
}

pub struct SettingsNavigation<Section> {
    chosen: Option<Section>,
    /// The page holding unsaved changes, if any; see `must_stay_on_page`.
    unsaved: Option<Section>,
    attention: u32,
    is_page_shown_when_narrow: bool,
    page: Option<ShownPage<Section>>,
    nav_focus: HashMap<Section, FocusHandle>,
    /// "Delete server", "Delete channel".
    pub danger_focus: FocusHandle,
    pub container_focus: FocusHandle,
    pub page_focus: FocusHandle,
}

impl<Section: std::fmt::Debug> std::fmt::Debug for SettingsNavigation<Section> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SettingsNavigation")
            .field("chosen", &self.chosen)
            .field("unsaved", &self.unsaved)
            .finish_non_exhaustive()
    }
}

impl<Section: Copy + PartialEq + Eq + Hash + 'static> SettingsNavigation<Section> {
    /// Opens on `initial_section` (the first one shown when `None`), its page alone when narrow.
    pub fn new(initial_section: Option<Section>, is_wide: bool, cx: &mut App) -> Self {
        Self {
            chosen: initial_section,
            unsaved: None,
            attention: 0,
            is_page_shown_when_narrow: initial_section.is_some() && !is_wide,
            page: None,
            nav_focus: HashMap::new(),
            danger_focus: cx.focus_handle(),
            container_focus: cx.focus_handle(),
            page_focus: cx.focus_handle(),
        }
    }

    /// The section shown: the chosen one while it is still offered, else the first offered.
    #[must_use]
    pub fn section(&self, offered: &[Section]) -> Option<Section> {
        self.chosen
            .filter(|chosen| offered.contains(chosen))
            .or_else(|| offered.first().copied())
    }

    #[must_use]
    pub fn is_page_shown_when_narrow(&self) -> bool {
        self.is_page_shown_when_narrow
    }

    /// The nav item's handle, made the first time it is asked for.
    pub fn nav_focus(&mut self, section: Section, cx: &mut App) -> FocusHandle {
        self.nav_focus
            .entry(section)
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }

    pub fn note_unsaved_changes(&mut self, section: Section, has_changes: bool) {
        if has_changes {
            self.unsaved = Some(section);
        } else if self.unsaved == Some(section) {
            self.unsaved = None;
        }
    }

    /// Whether the page shown is `section`'s (none for `None`).
    #[must_use]
    pub fn is_showing(&self, section: Option<Section>) -> bool {
        self.page.as_ref().map(|page| page.section) == section
    }

    /// Shows `page` in place of the one shown; a page going away holds no unsaved changes any more.
    pub fn replace_page(&mut self, page: Option<ShownPage<Section>>) {
        if let Some(gone) = self.page.as_ref().map(|page| page.section) {
            self.note_unsaved_changes(gone, false);
        }
        self.page = page;
    }

    #[must_use]
    pub fn page_view(&self) -> Option<AnyView> {
        self.page.as_ref().map(|page| page.view.clone())
    }

    /// Refused while the shown page holds unsaved changes: its bar asks to save or reset first.
    pub fn can_leave_page(&mut self, shown: Option<Section>, cx: &mut App) -> bool {
        if !must_stay_on_page(self.unsaved, shown) {
            return true;
        }
        self.attention += 1;
        if let Some(page) = self.page.as_ref() {
            (page.set_attention)(self.attention, cx);
        }
        false
    }

    /// Shows `target`'s page unless leaving the shown one is refused; narrow, the page replaces
    /// the navigation and takes focus.
    pub fn choose(
        &mut self,
        target: Section,
        shown: Option<Section>,
        is_wide: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if Some(target) != shown && !self.can_leave_page(shown, cx) {
            return;
        }
        self.chosen = Some(target);
        self.attention = 0;
        if !is_wide {
            self.is_page_shown_when_narrow = true;
            window.focus(&self.page_focus, cx);
        }
    }

    /// Narrow: back to the navigation, on the shown section's item.
    pub fn back(&mut self, shown: Option<Section>, window: &mut Window, cx: &mut App) {
        if !self.can_leave_page(shown, cx) {
            return;
        }
        self.is_page_shown_when_narrow = false;
        if let Some(section) = shown {
            let focus = self.nav_focus(section, cx);
            window.focus(&focus, cx);
        }
    }
}

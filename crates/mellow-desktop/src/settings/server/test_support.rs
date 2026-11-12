//! Server settings headless, on the shell's sample world where the reader holds a role above
//! "Moderators" and every permission, so its pages offer what a manager may change.

use gpui::{
    AppContext as _, Entity, TestAppContext, VisualTestContext, WindowAppearance, px, size,
};
use mellow_client_core::GatewayUpdate;
use mellow_client_core::shared_logic::server_settings::ServerSettingsSection;
use mellow_client_core::state::ClientState;
use mellow_protocol::{GatewayEvent, Permissions, Role, Snowflake};

use super::ServerSettings;
use crate::session::Session;
use crate::shell_fixtures::{GUILD_RUST, sample_state};

/// The reader's own role, above "Moderators".
pub const ADMINS: Snowflake = Snowflake::from_raw(91);

pub fn role(id: Snowflake, name: &str, position: u32, permissions: Permissions) -> Role {
    Role {
        id,
        name: name.to_owned(),
        color: 0,
        position,
        permissions,
        is_displayed_separately: false,
        is_mentionable: false,
        icon: None,
        unicode_emoji: None,
    }
}

pub fn state_managing_rust() -> ClientState {
    let mut state = sample_state();
    let mut guild = state
        .guild(GUILD_RUST)
        .cloned()
        .expect("the sample world has Rust Lyon");
    guild.my_permissions = Permissions::all();
    guild.my_role_ids = vec![ADMINS];
    guild
        .roles
        .push(role(GUILD_RUST, "@everyone", 0, Permissions::VIEW_CHANNEL));
    guild
        .roles
        .push(role(ADMINS, "Admins", 2, Permissions::ADMINISTRATOR));
    state.apply_event(GatewayEvent::GuildCreate(Box::new(guild)), 0);
    state
}

/// Server settings for Rust Lyon on `section`, in a wide window.
pub fn open_server_settings(
    cx: &mut TestAppContext,
    section: ServerSettingsSection,
) -> (
    Entity<Session>,
    Entity<ServerSettings>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(crate::strings_pending_i18n::ActiveStrings(
            &crate::strings_pending_i18n::ENGLISH,
        ));
        cx.set_global(crate::theme::ActiveTheme::for_appearance(
            WindowAppearance::Dark,
        ));
        crate::components::bind_tab_navigation(cx);
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| {
        *session.state_mut() = state_managing_rust();
    });
    let screen_session = session.clone();
    let (settings, cx) = cx.add_window_view(|window, cx| {
        ServerSettings::new(screen_session, GUILD_RUST, Some(section), window, cx)
    });
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (session, settings, cx)
}

/// The page shown, as its own view.
pub fn shown_page<Page: 'static>(
    settings: &Entity<ServerSettings>,
    cx: &mut VisualTestContext,
) -> Entity<Page> {
    settings.read_with(cx, |settings, _| {
        settings
            .navigation
            .page_view()
            .and_then(|view| view.downcast::<Page>().ok())
            .expect("the section's page is shown")
    })
}

pub fn receive(session: &Entity<Session>, event: GatewayEvent, cx: &mut VisualTestContext) {
    session.update(cx, |session, cx| {
        session.apply_gateway_updates_of_this_session(vec![GatewayUpdate::Event(event)], cx);
    });
    cx.run_until_parked();
}

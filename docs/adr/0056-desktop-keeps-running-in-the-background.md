# 0056. The desktop keeps running in the background

- Status: Accepted
- Date: 2026-11-13
- Builds on: [ADR-0051](0051-gpui-desktop-client.md), whose window closing quit the app ("one window")

## Context
Until now, closing the desktop's window quit Mellow, which also dropped a voice call without telling the server (it timed the person out). Discord keeps running when its window closes: the call goes on, notifications still come, and the app is reached again from the menu bar, the Dock or the system tray. The owner asked for the same on 2026-11-13, on macOS, Linux (CachyOS on Wayland) and Windows.

What each system offers:
- **macOS**: an app may live without a window; the Dock icon stays, and `applicationShouldHandleReopen` tells the app when it is clicked. Mellow already has a menu bar item.
- **Linux**: Wayland has no way for a client to hide a toplevel window and show it again; only destroying it and making a new one. The tray is the StatusNotifierItem specification over D-Bus: KDE Plasma, Waybar (Hyprland, Sway), XFCE and Cinnamon host it; GNOME only with the AppIndicator extension; some desktops have no host at all.
- **Windows**: an app may live without a window; the notification area icon is `Shell_NotifyIconW`.
- GPUI has no API to hide a window on any system, nor a tray.

## Decision
- **Closing really closes the window, on every system**, and the app keeps running: the session, the gateway, the voice call and the notifier live at the app's level, not the window's. Opening Mellow again builds a new window with the last one's size, place and state (maximised, full screen), remembered in memory (drafts are kept on disk). One path everywhere, the only one Wayland allows, and a closed window frees its views. The notifier now belongs to the app (`notifier::follow_session_for_notifications`) and shows its count in whichever window is open.
- **Mellow quits on close instead** when the person turned "Keep running in the background" off (User settings, Appearance, on by default, kept in the preferences file as `close-quits=true`), when nothing outside the window could bring it back (a Linux desktop without a tray host), or when signed out (there is nothing to keep running). Hidden with no visible trace, a call would keep the microphone open unseen. The decision is a pure function (`background::window_close_outcome`).
- **Reaching Mellow again**: the menu bar item and the Dock on macOS (`on_reopen`), the tray icon on Linux (`ksni`, a click or "Open Mellow") and Windows (a hand-written `Shell_NotifyIconW` icon on a hidden window of the UI thread, re-added when Explorer restarts), a click on a notification, and launching Mellow again.
- **One instance per person**, checked in `main` before GPUI starts, so a second launch opens no window and reads no secret: on macOS and Linux a lock file held for the app's life and a Unix socket beside it, in a directory only the person can open (`$XDG_RUNTIME_DIR/mellow` on Linux); a second launch finds the lock taken, connects, and exits once the running Mellow answers that it took the request. The lock tells a socket left by a crash from a live one. On Windows, a named pipe per account (named after its domain and full name) created with `FILE_FLAG_FIRST_PIPE_INSTANCE` and rejecting remote clients; a pipe the launch cannot open (another account's) lets it start. No answer means the running Mellow is quitting: the launch waits up to 5 seconds, longer than a quit takes, for it to let go, then starts, so Mellow launched right after Quit opens. The component gallery (`MELLOW_COMPONENT_GALLERY`) does not take part. Launch Services already keeps one instance of a macOS bundle; this also covers a binary started directly. A failure to set this up lets the launch go on.
- **The tray menu** has "Open Mellow", the microphone and headset toggles (as the user card words them, applying to a call or the next one), and "Quit Mellow". Its tooltip says "(3) Mellow" as the window title does; the Linux item turns to `NeedsAttention` while something waits.
- **Quitting** (Cmd/Ctrl+Q, the app menu, the tray, the Dock, a logout) leaves the call first: a quit handler registered before the session's queues `voice_leave` on the gateway itself, rather than from the engine's thread which the exit may cut short, and waits up to 500 ms for the gateway to take it, before the session ends the connection as before.
- **New crates**: `ksni` 0.3 (Unlicense) with its `async-io` runtime, on the zbus the portals already build, and `task-local`; the Windows code uses the `windows` 0.62 crate GPUI already builds. No GTK: `tray-icon` would bring GTK and libappindicator into the Linux build.

## Consequences
- A call, notifications and the unread count carry on with the window closed, on all three systems.
- What a window held in memory is gone when it closes: the scroll position, an open dialog, the call view's layout. Drafts and the place come back.
- On GNOME without the AppIndicator extension, closing the window quits, as before; the setting's description says so.
- On Wayland, a window opened again from a second launch or the tray gets no activation token, so some compositors may open it without focus. Forwarding the launcher's `XDG_ACTIVATION_TOKEN` through the socket needs a GPUI API that does not exist yet.
- The Windows tray icon is drawn from the web app's 192 px icon scaled to 32 px; Windows scales it again for the display. Its menu and the tray's behaviour have only been built by CI, not run.

# 0053. Global push-to-talk on the desktop

- Status: Accepted
- Date: 2026-11-12
- Builds on: [ADR-0051](0051-gpui-desktop-client.md), whose step 7 asks for push-to-talk while the app is not focused

## Context
Push-to-talk in a browser only hears keys while the tab has focus. The desktop client (ADR-0051) is expected to do what Discord's does: the key works while a game or another app is in front. Each system offers this differently, and none lets an app read keys globally for free:
- **macOS**: Carbon hot keys (`RegisterEventHotKey`) need no permission. Reading every key instead (a `CGEventTap`, `NSEvent` global monitors) needs Input Monitoring or Accessibility, which the person must grant in System Settings.
- **Windows**: `RegisterHotKey` grabs one key for the app; a low-level keyboard hook reads every key and is what anti-cheat tools look at.
- **Linux X11**: any client can grab a key on the root window (`XGrabKey`).
- **Linux Wayland**: no client sees keys aimed at another. The only way is the XDG desktop portal's `GlobalShortcuts` interface (KDE Plasma, GNOME 48 and later, Hyprland through its own portal; not every wlroots compositor), where the compositor shows the shortcut to the person, who confirms or picks another binding.

The owner runs CachyOS on Wayland, so the Wayland path is as important as macOS.

## Decision
- **macOS, Windows and X11 use [`global-hotkey`](https://github.com/tauri-apps/global-hotkey)** (Tauri, Apache-2.0 or MIT), pinned in the workspace. It wraps the three grab APIs above, reports both press and release, and builds without system packages (its X11 side is `x11rb`, pure Rust). It brings `keyboard-types` 0.8, `crossbeam-channel` and `windows-sys` 0.59 next to versions already in the tree. Writing the three bindings ourselves would mean three `unsafe` FFI layers to maintain for the same behaviour.
- **Wayland uses the portal through `ashpd`**, already in the workspace for Vesper's capture, with its `global_shortcuts` feature. The stored key is passed as the preferred trigger (an xkb keysym name); the compositor's choice wins, and the voice panel shows the binding it reports. The process registers itself as `org.mellow.desktop` with the portal's host registry where it exists, so the compositor names and remembers the shortcut as Mellow's.
- **macOS follows the keyboard layout**: the desktop records a key by the character the layout types (GPUI reads keys through the layout), while Carbon names a key by its position on an ANSI keyboard. Before registering, the desktop asks `UCKeyTranslate` which position types that character on the current layout (on AZERTY, "a" is at ANSI's Q), and registers again when the layout changes. When no key of the layout types it, the panel says the key is not on the current layout. Windows (virtual keys) and X11 and the portal (keysyms) already follow the layout.
- **Only keys of the keyboard itself** become hot keys: letters, digits, punctuation, F1 to F24, navigation and editing keys. Media keys are left out, since `global-hotkey` hears them on macOS through an event tap, which needs the Accessibility permission.
- **Wayland without the portal** falls back to the X11 grab when an XWayland display exists, and the panel says it only hears keys while an X11 window is focused. Without either, the panel says push-to-talk works only in Mellow's window.
- The session kind is read at run time as GPUI does: `WAYLAND_DISPLAY`, then `XDG_SESSION_TYPE`.
- **The key is held only while needed**: in a voice channel (connecting included), with push-to-talk chosen and a key recorded. Leaving, changing the mode or the key lets it go. A grab keeps a character key from typing in every other app, so it must not outlive the call, and the panel warns about it.
- **A modifier key alone stays in the window**: hot key APIs grab a key with modifiers, not a modifier by itself. The panel says so.
- The in-window handler keeps running. Pressing and releasing push-to-talk are idempotent, so a key both hear presses once; losing focus does not release a key the global listener holds, since it hears the release wherever focus went.
- **A portal that cannot open a session counts as missing**: without the portal service, `ashpd` still builds the proxy and only the session fails, so that case also tries the XWayland grab. An error after the session opened has its own sentence. A shortcut the portal takes without a key (Hyprland until the person binds `org.mellow.desktop:push-to-talk`, or a binding removed in the system's settings) is not called working: the panel asks the person to choose a key in the desktop's shortcut settings.
- **A release is never lost**: key events only wake the UI through the bounded queue, and the newest key state is kept apart, so a full queue cannot drop the last release. Losing the window's focus still releases the key until the current binding has delivered at least one release, so a listener that never reports one (a portal backend sending only `Activated`) cannot leave the microphone open through a focus change.
- Threads: macOS and Windows register on the UI thread, whose event loop receives their hot keys (short system calls). On Linux each binding runs on its own thread (portal calls on `async-io`, or the X11 grab), which waits for the previous binding's thread before taking the key. Every portal call waits under the binding's stop signal, so a portal that never answers keeps no thread once the binding ends. Key events reach the UI through a bounded queue and are never logged.

## Consequences
- Push-to-talk works in other apps on all three systems without asking for a permission; on Wayland the person confirms the shortcut once in the compositor's dialog.
- On macOS, Secure Input (a focused password field anywhere) blocks hot keys, as for every app.
- Windows notices a release by polling the key every 50 ms, and `global-hotkey`'s X11 thread reads the server every 50 ms: the microphone may open or close up to 50 ms late there.
- `global-hotkey` reports success on X11 when it could not reach the X server at all; the panel would then claim the key works in other apps. Checking the connection ourselves is left until it is ever seen.
- **A held modifier silences the key in other apps**: Carbon hot keys, `RegisterHotKey` and the X11 grab fire only with exactly the registered modifiers. With none registered, Shift+V or Ctrl+V never fires, so a player holding Shift to sprint or Ctrl to crouch cannot talk. Registering every combination would take Ctrl+V and Shift+letters from every app, so this stays a limit; the in-window handler still hears the key with modifiers held.
- A portal backend that sends `Activated` but never `Deactivated` would keep the microphone open after the first press while another app is focused; the focus safety net above covers only a focus change. This has not been checked on hardware yet (wlroots and Hyprland portals first).
- A person who wants, in other apps, a modifier key alone, a combination, or the key to work while a modifier is held needs a different listener (an event tap with Input Monitoring on macOS, a low-level hook on Windows). That would be a new ADR.

//! The Windows notification area icon, on Win32's `Shell_NotifyIconW`: a hidden window on the UI
//! thread (GPUI's message loop dispatches its messages) receives the icon's clicks; a left click
//! opens Mellow, a right click shows the menu. The icon comes back when Explorer restarts
//! (`TaskbarCreated`) and goes when the app quits.

// Win32's C API: every call is commented.
#![allow(unsafe_code)]

use std::cell::RefCell;

use gpui::App;
use tokio::sync::mpsc::Sender;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFY_ICON_MESSAGE,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    GetCursorPos, HICON, MF_SEPARATOR, MF_STRING, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
    WNDCLASSW,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::{TrayCommand, TrayMenu, tray_icon_rgba};

/// The icon's clicks reach the window as this message.
const TRAY_CALLBACK_MESSAGE: u32 = WM_APP + 1;
const TRAY_ICON_ID: u32 = 1;
/// The notification area's small icon size at 100 % scaling; Windows scales it for the display.
const TRAY_ICON_SIZE: u32 = 32;
const MENU_OPEN: usize = 1;
const MENU_MICROPHONE: usize = 2;
const MENU_HEADSET: usize = 3;
const MENU_QUIT: usize = 4;

/// The tray, owned by the UI thread its window belongs to.
struct WindowsTray {
    window: HWND,
    icon: HICON,
    menu: TrayMenu,
    commands: Sender<TrayCommand>,
    taskbar_created_message: u32,
    is_shown: bool,
}

thread_local! {
    /// Read by the window procedure, which Win32 calls on this same thread.
    static TRAY: RefCell<Option<WindowsTray>> = const { RefCell::new(None) };
}

/// A wide, NUL-terminated copy of `text` cut to the buffer, as `szTip` wants it.
fn wide_into<const LENGTH: usize>(text: &str, buffer: &mut [u16; LENGTH]) {
    let mut written = 0;
    for (slot, unit) in buffer
        .iter_mut()
        .take(LENGTH.saturating_sub(1))
        .zip(text.encode_utf16())
    {
        *slot = unit;
        written += 1;
    }
    if let Some(end) = buffer.get_mut(written) {
        *end = 0;
    }
}

fn icon_data(tray: &WindowsTray) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(u32::MAX),
        hWnd: tray.window,
        uID: TRAY_ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: TRAY_CALLBACK_MESSAGE,
        hIcon: tray.icon,
        ..NOTIFYICONDATAW::default()
    };
    wide_into(&tray.menu.tooltip, &mut data.szTip);
    data
}

fn notify(message: NOTIFY_ICON_MESSAGE, tray: &WindowsTray) -> bool {
    let data = icon_data(tray);
    // SAFETY: `data` is a fully initialised `NOTIFYICONDATAW` with its own size, living for the
    // call.
    unsafe { Shell_NotifyIconW(message, &raw const data) }.as_bool()
}

/// The icon as Win32 wants it: BGRA rows from the top, and an all-zero mask, the alpha channel
/// carrying the shape.
fn create_icon(instance: HINSTANCE) -> Option<HICON> {
    let rgba = tray_icon_rgba(TRAY_ICON_SIZE)?;
    let bgra: Vec<u8> = rgba
        .pixels()
        .flat_map(|pixel| {
            let [red, green, blue, alpha] = pixel.0;
            [blue, green, red, alpha]
        })
        .collect();
    let side = i32::try_from(TRAY_ICON_SIZE).ok()?;
    // One bit a pixel, rows padded to 16 bits.
    let mask_row_bytes = TRAY_ICON_SIZE.div_ceil(16) * 2;
    let mask = vec![0_u8; usize::try_from(mask_row_bytes * TRAY_ICON_SIZE).ok()?];
    // SAFETY: both buffers hold `side` rows of the sizes given (1 bit and 32 bits a pixel) and
    // live for the call, which copies them.
    unsafe {
        CreateIcon(
            Some(instance),
            side,
            side,
            1,
            32,
            mask.as_ptr(),
            bgra.as_ptr(),
        )
    }
    .inspect_err(|error| tracing::warn!(code = error.code().0, "cannot create the tray icon"))
    .ok()
}

pub(super) fn install_tray(commands: Sender<TrayCommand>, cx: &mut App) {
    // Quitting takes the icon away; otherwise it stays until the pointer passes over it.
    cx.on_app_quit(|_| {
        TRAY.with_borrow_mut(|tray| {
            if let Some(tray) = tray.take() {
                notify(NIM_DELETE, &tray);
            }
        });
        std::future::ready(())
    })
    .detach();
    // SAFETY: no module name asks for the running executable's handle, which is never freed.
    let instance: HINSTANCE = match unsafe { GetModuleHandleW(PCWSTR::null()) } {
        Ok(module) => module.into(),
        Err(error) => {
            tracing::warn!(code = error.code().0, "no module handle for the tray");
            return;
        }
    };
    let class_name = w!("MellowTrayWindow");
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_procedure),
        hInstance: instance,
        lpszClassName: class_name,
        ..WNDCLASSW::default()
    };
    // SAFETY: `class` is fully initialised and its strings are static.
    if unsafe { RegisterClassW(&raw const class) } == 0 {
        tracing::warn!("cannot register the tray's window class");
        return;
    }
    // A hidden top-level window rather than a message-only one: only top-level windows hear
    // Explorer's `TaskbarCreated` broadcast.
    // SAFETY: the class was just registered with this instance; no creation data is passed.
    let window = match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("Mellow"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
    } {
        Ok(window) => window,
        Err(error) => {
            tracing::warn!(code = error.code().0, "cannot create the tray's window");
            return;
        }
    };
    let Some(icon) = create_icon(instance) else {
        return;
    };
    // SAFETY: a static, NUL-terminated name.
    let taskbar_created_message = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    let mut tray = WindowsTray {
        window,
        icon,
        menu: TrayMenu::default(),
        commands,
        taskbar_created_message,
        is_shown: false,
    };
    tray.is_shown = notify(NIM_ADD, &tray);
    if !tray.is_shown {
        tracing::warn!("the notification area refused Mellow's icon");
    }
    TRAY.set(Some(tray));
}

pub(super) fn show_menu(menu: &TrayMenu, _cx: &mut App) {
    TRAY.with_borrow_mut(|tray| {
        if let Some(tray) = tray {
            tray.menu = menu.clone();
            if tray.is_shown {
                notify(NIM_MODIFY, tray);
            }
        }
    });
}

pub(super) fn is_tray_shown(_cx: &App) -> bool {
    TRAY.with_borrow(|tray| tray.as_ref().is_some_and(|tray| tray.is_shown))
}

/// The right-click menu, at the pointer; returns the chosen entry, or zero.
fn choose_from_menu(window: HWND, menu: &TrayMenu) -> usize {
    // SAFETY: a new, empty menu, destroyed below.
    let Ok(popup) = (unsafe { CreatePopupMenu() }) else {
        return 0;
    };
    let entries = [
        (MENU_OPEN, Some(menu.open.as_str())),
        (0, None),
        (MENU_MICROPHONE, Some(menu.microphone.as_str())),
        (MENU_HEADSET, Some(menu.headset.as_str())),
        (0, None),
        (MENU_QUIT, Some(menu.quit.as_str())),
    ];
    for (id, label) in entries {
        // SAFETY: `popup` is a live menu; each label lives for its call, which copies it.
        let appended = match label {
            Some(label) => unsafe { AppendMenuW(popup, MF_STRING, id, &HSTRING::from(label)) },
            None => unsafe { AppendMenuW(popup, MF_SEPARATOR, 0, PCWSTR::null()) },
        };
        if appended.is_err() {
            tracing::warn!("cannot build the tray's menu");
        }
    }
    let mut pointer = POINT::default();
    // SAFETY: `pointer` is a valid out-parameter.
    let _ = unsafe { GetCursorPos(&raw mut pointer) };
    // The menu closes when the person clicks elsewhere only if the window is in front (Win32's
    // documented `TrackPopupMenu` remark), and the posted message lets it close at once.
    // SAFETY: `window` and `popup` are live; `TPM_RETURNCMD` returns the chosen id instead of
    // posting it.
    let chosen = unsafe {
        let _ = SetForegroundWindow(window);
        let chosen = TrackPopupMenu(
            popup,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            pointer.x,
            pointer.y,
            None,
            window,
            None,
        );
        let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(popup);
        chosen
    };
    usize::try_from(chosen.0).unwrap_or(0)
}

unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Copied out, so the menu's own message loop below never meets a borrowed tray.
    let tray_facts = TRAY.with_borrow(|tray| {
        tray.as_ref().map(|tray| {
            (
                tray.menu.clone(),
                tray.commands.clone(),
                tray.taskbar_created_message,
            )
        })
    });
    if let Some((menu, commands, taskbar_created_message)) = tray_facts {
        if message == taskbar_created_message {
            TRAY.with_borrow_mut(|tray| {
                if let Some(tray) = tray {
                    tray.is_shown = notify(NIM_ADD, tray);
                }
            });
            return LRESULT(0);
        }
        if message == TRAY_CALLBACK_MESSAGE {
            // Without `NOTIFYICON_VERSION_4`, the mouse message is the whole `lparam`.
            let command = match u32::try_from(lparam.0).unwrap_or(0) {
                WM_LBUTTONUP => Some(TrayCommand::OpenMellow),
                WM_RBUTTONUP | WM_CONTEXTMENU => match choose_from_menu(window, &menu) {
                    MENU_OPEN => Some(TrayCommand::OpenMellow),
                    MENU_MICROPHONE => Some(TrayCommand::ToggleMicrophone),
                    MENU_HEADSET => Some(TrayCommand::ToggleDeafened),
                    MENU_QUIT => Some(TrayCommand::QuitMellow),
                    _ => None,
                },
                _ => None,
            };
            if let Some(command) = command {
                // Full: the person clicked faster than the app follows; this click is dropped.
                let _ = commands.try_send(command);
            }
            return LRESULT(0);
        }
    }
    // SAFETY: the arguments are the ones Win32 handed this procedure.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tooltip_is_cut_to_its_buffer_and_ends_with_nul() {
        let mut buffer = [1_u16; 4];
        wide_into("Mellow", &mut buffer);
        assert_eq!(buffer, [77, 101, 108, 0]);
        wide_into("é", &mut buffer);
        assert_eq!(&buffer[..2], &[0xE9, 0]);
    }
}

//! Screen sharing on Windows (ADR-0054's amendment): Mellow's own picker lists the displays
//! (`EnumDisplayMonitors`) and the windows a person would switch to (`EnumWindows`, Mellow's own
//! left out), with GDI thumbnails; the chosen one is captured through `Windows.Graphics.Capture`
//! on a free-threaded frame pool, copied from the GPU into a reused BGRA buffer at 30 frames per
//! second, and encoded in VP8 like every CPU capture. While a display is shared, Mellow's own
//! windows are excluded from capture (`WDA_EXCLUDEFROMCAPTURE`), so the call never films itself;
//! its sound comes from WASAPI's process loopback (`screen_sound_windows`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{E_FAIL, HMODULE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC,
    GetMonitorInfoW, HALFTONE, HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFO, ReleaseDC, SRCCOPY,
    SelectObject, SetStretchBltMode, StretchBlt,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetClientRect, GetWindow, GetWindowLongW,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, IInspectable, Interface};

use crate::screen_capture::{
    RunningScreenCapture, ScreenCaptureError, ScreenChoice, ScreenPicker, ScreenShareContext,
    ScreenSourceDescription, ScreenSourceKind, ScreenSources, ShareSound, ShareSoundState,
    SoundApplication, SystemScreenSources, THUMBNAIL_LONGEST_SIDE, pack_bgra_rows,
};
use crate::screen_sound_windows::{
    RunningShareSound, loopback_target, own_process_id, process_name, start_share_sound,
};
use crate::session::ScreenShareEnd;
use crate::simulcast::FRAMES_PER_SECOND;
use crate::video_frame::CapturedFrame;
use crate::video_sender::VideoInput;
use crate::windows_media_foundation::{ensure_com, size_as_u32};

const DISPLAY_ID_PREFIX: &str = "display:";
const WINDOW_ID_PREFIX: &str = "window:";
/// `PW_RENDERFULLCONTENT`: lets `PrintWindow` read windows drawn by the GPU.
const PRINT_FULL_CONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);
/// Frames the pool keeps: one being read, one being filled.
const FRAME_POOL_BUFFERS: i32 = 2;
/// How long the capture may take to start delivering before the share starts anyway.
const START_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Display(HMONITOR),
    Window(HWND),
}

fn handle_id(pointer: *mut std::ffi::c_void) -> usize {
    pointer.addr()
}

fn source_id(source: Source) -> String {
    match source {
        Source::Display(monitor) => format!("{DISPLAY_ID_PREFIX}{}", handle_id(monitor.0)),
        Source::Window(window) => format!("{WINDOW_ID_PREFIX}{}", handle_id(window.0)),
    }
}

fn parse_source(id: &str) -> Option<Source> {
    let handle = |text: &str| {
        text.parse::<usize>()
            .ok()
            .filter(|value| *value != 0)
            .map(std::ptr::without_provenance_mut)
    };
    if let Some(monitor) = id.strip_prefix(DISPLAY_ID_PREFIX) {
        return handle(monitor).map(|monitor| Source::Display(HMONITOR(monitor)));
    }
    id.strip_prefix(WINDOW_ID_PREFIX)
        .and_then(handle)
        .map(|window| Source::Window(HWND(window)))
}

fn system_error(error: impl std::fmt::Display) -> ScreenCaptureError {
    ScreenCaptureError::System(error.to_string())
}

/// The displays and their areas on the virtual screen.
fn displays() -> Vec<(HMONITOR, RECT)> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _context: HDC,
        _area: *mut RECT,
        found: LPARAM,
    ) -> BOOL {
        // SAFETY: `found` is the vector `displays` passes, alive for the enumeration.
        let found = unsafe { &mut *(found.0 as *mut Vec<(HMONITOR, RECT)>) };
        let mut info = MONITORINFO {
            cbSize: size_as_u32::<MONITORINFO>(),
            ..Default::default()
        };
        // SAFETY: a monitor Windows just named, and an information block sized for it.
        if unsafe { GetMonitorInfoW(monitor, &raw mut info) }.as_bool() {
            found.push((monitor, info.rcMonitor));
        }
        BOOL::from(true)
    }
    let mut found: Vec<(HMONITOR, RECT)> = Vec::new();
    // SAFETY: the callback only pushes into `found`, which outlives the call.
    let _ = unsafe {
        EnumDisplayMonitors(None, None, Some(collect), LPARAM((&raw mut found) as isize))
    };
    found
}

/// A window a person would switch to: visible, not cloaked (another virtual desktop, a
/// suspended app), not a tool window or an owned dialog, with a title, and not Mellow's.
fn is_shareable_window(window: HWND, own_process: u32) -> bool {
    // SAFETY: plain queries on a window handle Windows just named; a stale one fails them.
    unsafe {
        if !IsWindowVisible(window).as_bool()
            || GetWindow(window, GW_OWNER).is_ok_and(|owner| !owner.is_invalid())
            || GetWindowTextLengthW(window) == 0
            || (GetWindowLongW(window, GWL_EXSTYLE).cast_unsigned() & WS_EX_TOOLWINDOW.0) != 0
        {
            return false;
        }
        let mut cloaked = 0_u32;
        let is_cloaked = DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            size_as_u32::<u32>(),
        )
        .is_ok()
            && cloaked != 0;
        !is_cloaked && window_process(window) != own_process
    }
}

fn window_process(window: HWND) -> u32 {
    let mut process = 0;
    // SAFETY: a plain query writing one integer.
    unsafe { GetWindowThreadProcessId(window, Some(&raw mut process)) };
    process
}

/// Every top-level window, in Z order (front first).
fn top_level_windows() -> Vec<HWND> {
    unsafe extern "system" fn collect(window: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the vector `top_level_windows` passes, alive for the enumeration.
        let found = unsafe { &mut *(found.0 as *mut Vec<HWND>) };
        found.push(window);
        BOOL::from(true)
    }
    let mut found: Vec<HWND> = Vec::new();
    // SAFETY: the callback only pushes into `found`, which outlives the call.
    let _ = unsafe { EnumWindows(Some(collect), LPARAM((&raw mut found) as isize)) };
    found
}

fn window_title(window: HWND) -> String {
    // SAFETY: the buffer is ours and sized for the title Windows reports.
    unsafe {
        let length = usize::try_from(GetWindowTextLengthW(window)).unwrap_or(0);
        let mut title = vec![0_u16; length + 1];
        let copied = usize::try_from(GetWindowTextW(window, &mut title)).unwrap_or(0);
        String::from_utf16_lossy(&title[..copied.min(length)])
    }
}

fn client_size(window: HWND) -> Option<(u32, u32)> {
    let mut area = RECT::default();
    // SAFETY: a plain query writing one rectangle.
    unsafe { GetClientRect(window, &raw mut area) }.ok()?;
    let width = u32::try_from(area.right - area.left).ok()?;
    let height = u32::try_from(area.bottom - area.top).ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

fn rect_size(area: RECT) -> Option<(u32, u32)> {
    let width = u32::try_from(area.right - area.left).ok()?;
    let height = u32::try_from(area.bottom - area.top).ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

/// `(width, height)` shrunk so the longest side is at most the thumbnail's.
fn thumbnail_size(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= THUMBNAIL_LONGEST_SIDE {
        return (width.max(1), height.max(1));
    }
    let shrink = |length: u32| {
        u32::try_from(u64::from(length) * u64::from(THUMBNAIL_LONGEST_SIDE) / u64::from(longest))
            .unwrap_or(1)
            .max(1)
    };
    (shrink(width), shrink(height))
}

/// A memory context drawing into a bitmap; both freed, in order, when dropped.
struct GdiPicture {
    context: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}

impl Drop for GdiPicture {
    fn drop(&mut self) {
        // SAFETY: created by `GdiPicture::new`, each released once, the bitmap deselected first.
        unsafe {
            SelectObject(self.context, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.context);
        }
    }
}

impl GdiPicture {
    /// Takes `bitmap` (freed on failure too).
    fn new(bitmap: HBITMAP) -> Option<Self> {
        if bitmap.is_invalid() {
            return None;
        }
        // SAFETY: a fresh memory context; the bitmap is selected into it and restored on drop.
        unsafe {
            let context = CreateCompatibleDC(None);
            if context.is_invalid() {
                let _ = DeleteObject(bitmap.into());
                return None;
            }
            let previous = SelectObject(context, bitmap.into());
            Some(Self {
                context,
                bitmap,
                previous,
            })
        }
    }
}

/// The `area` of `source` shrunk into a thumbnail, as opaque BGRA.
fn shrink_into_thumbnail(source: HDC, area: RECT) -> Option<CapturedFrame> {
    let (width, height) = rect_size(area)?;
    let (thumbnail_width, thumbnail_height) = thumbnail_size(width, height);
    let thumbnail_width_pixels = i32::try_from(thumbnail_width).ok()?;
    let thumbnail_height_pixels = i32::try_from(thumbnail_height).ok()?;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_as_u32::<BITMAPINFOHEADER>(),
            biWidth: thumbnail_width_pixels,
            // Negative: rows top-down, as `CapturedFrame` wants them.
            biHeight: -thumbnail_height_pixels,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let row_bytes = usize::try_from(thumbnail_width).ok()? * 4;
    let length = row_bytes.checked_mul(usize::try_from(thumbnail_height).ok()?)?;
    // SAFETY: the DIB section's bits hold `length` bytes (32-bit pixels, rows unpadded) for as
    // long as the section lives, which is until `thumbnail` drops, after the copy.
    unsafe {
        let mut bits = std::ptr::null_mut();
        let section = CreateDIBSection(
            None,
            &raw const info,
            DIB_RGB_COLORS,
            &raw mut bits,
            None,
            0,
        )
        .ok()?;
        let thumbnail = GdiPicture::new(section)?;
        SetStretchBltMode(thumbnail.context, HALFTONE);
        let is_drawn = StretchBlt(
            thumbnail.context,
            0,
            0,
            thumbnail_width_pixels,
            thumbnail_height_pixels,
            Some(source),
            area.left,
            area.top,
            area.right - area.left,
            area.bottom - area.top,
            SRCCOPY,
        )
        .as_bool();
        if !is_drawn || bits.is_null() {
            return None;
        }
        let pixels = std::slice::from_raw_parts(bits.cast::<u8>(), length);
        let mut picture = pack_bgra_rows(
            Vec::new(),
            thumbnail_width,
            thumbnail_height,
            row_bytes,
            pixels,
            false,
        )?;
        // GDI leaves alpha at zero.
        for pixel in picture.bytes.as_chunks_mut::<4>().0 {
            pixel[3] = u8::MAX;
        }
        Some(picture)
    }
}

/// A display's thumbnail, read from its area of the virtual screen.
fn display_thumbnail(area: RECT) -> Option<CapturedFrame> {
    // SAFETY: the screen's context is released once, after the copy that reads it.
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let thumbnail = shrink_into_thumbnail(screen, area);
        ReleaseDC(None, screen);
        thumbnail
    }
}

/// A window's thumbnail, painted by the window itself (so covered windows show whole).
fn window_thumbnail(window: HWND) -> Option<CapturedFrame> {
    let (width, height) = client_size(window)?;
    let area = RECT {
        left: 0,
        top: 0,
        right: i32::try_from(width).ok()?,
        bottom: i32::try_from(height).ok()?,
    };
    // SAFETY: a compatible bitmap the window paints into, freed by `GdiPicture`'s drop after
    // the thumbnail is copied out of it; the screen's context is released once.
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let bitmap = CreateCompatibleBitmap(screen, area.right, area.bottom);
        ReleaseDC(None, screen);
        let full = GdiPicture::new(bitmap)?;
        if !PrintWindow(window, full.context, PRINT_FULL_CONTENT).as_bool() {
            return None;
        }
        shrink_into_thumbnail(full.context, area)
    }
}

fn list_sources(with_thumbnails: bool) -> Vec<ScreenSourceDescription> {
    let own_process = own_process_id();
    let mut sources: Vec<ScreenSourceDescription> = displays()
        .into_iter()
        .filter_map(|(monitor, area)| {
            let (width, height) = rect_size(area)?;
            Some(ScreenSourceDescription {
                id: source_id(Source::Display(monitor)),
                kind: ScreenSourceKind::Display,
                title: String::new(),
                application_name: None,
                width,
                height,
                thumbnail: with_thumbnails.then(|| display_thumbnail(area)).flatten(),
            })
        })
        .collect();
    for window in top_level_windows() {
        if !is_shareable_window(window, own_process) {
            continue;
        }
        let Some((width, height)) = client_size(window) else {
            continue;
        };
        // SAFETY: a plain query; a minimised window paints nothing.
        let is_minimised = unsafe { IsIconic(window) }.as_bool();
        sources.push(ScreenSourceDescription {
            id: source_id(Source::Window(window)),
            kind: ScreenSourceKind::Window,
            title: window_title(window),
            application_name: process_name(window_process(window)),
            width,
            height,
            thumbnail: (with_thumbnails && !is_minimised)
                .then(|| window_thumbnail(window))
                .flatten(),
        });
    }
    sources
}

/// Excludes Mellow's top-level windows from every capture (Windows 10 2004 and later) while a
/// display is shared, remembering which, so the picker or a new window never shows in it.
fn exclude_own_windows(own_process: u32, excluded: &mut Vec<HWND>) {
    // SAFETY: plain queries.
    excluded.retain(|window| unsafe { IsWindow(Some(*window)) }.as_bool());
    for window in top_level_windows() {
        if window_process(window) != own_process || excluded.contains(&window) {
            continue;
        }
        // SAFETY: a window of this process, as the call requires.
        match unsafe { SetWindowDisplayAffinity(window, WDA_EXCLUDEFROMCAPTURE) } {
            Ok(()) => excluded.push(window),
            Err(error) => {
                tracing::debug!(%error, "a Mellow window could not be kept out of the share");
            }
        }
    }
}

fn restore_own_windows(excluded: &[HWND]) {
    for window in excluded {
        // SAFETY: a window of this process; a closed one fails harmlessly.
        let _ = unsafe { SetWindowDisplayAffinity(*window, WDA_NONE) };
    }
}

/// The GPU objects a capture copies its pictures with.
struct Direct3D {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    capture_device: IDirect3DDevice,
}

fn create_direct3d() -> windows::core::Result<Direct3D> {
    let mut last_error = windows::core::Error::from(E_FAIL);
    // WARP, Windows' software renderer, where no GPU driver is usable (virtual machines).
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device = None;
        let mut context = None;
        // SAFETY: plain device creation into owned out values.
        let created = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&raw mut device),
                None,
                Some(&raw mut context),
            )
        };
        match (created, device, context) {
            (Ok(()), Some(device), Some(context)) => {
                let dxgi_device: IDXGIDevice = device.cast()?;
                // SAFETY: a valid DXGI device of the device just made.
                let capture_device: IDirect3DDevice =
                    unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }?.cast()?;
                return Ok(Direct3D {
                    device,
                    context,
                    capture_device,
                });
            }
            (Err(error), _, _) => last_error = error,
            _ => {}
        }
    }
    Err(last_error)
}

fn capture_item(source: Source) -> windows::core::Result<GraphicsCaptureItem> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    // SAFETY: a monitor or window handle checked to exist just before; a stale one fails.
    unsafe {
        match source {
            Source::Display(monitor) => interop.CreateForMonitor(monitor),
            Source::Window(window) => interop.CreateForWindow(window),
        }
    }
}

/// Copies the frame's texture into `staging` (re-made when its size changes), then its
/// content area into a packed BGRA picture in `bytes`.
fn read_frame(
    direct3d: &Direct3D,
    frame: &Direct3D11CaptureFrame,
    staging: &mut Option<(ID3D11Texture2D, u32, u32)>,
    bytes: Vec<u8>,
) -> windows::core::Result<Option<CapturedFrame>> {
    let content = frame.ContentSize()?;
    let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
    // SAFETY: the texture is the frame's, valid until the frame closes after this call; the
    // staging texture is mapped for reading, read within its row pitch and rows, then unmapped.
    unsafe {
        let texture: ID3D11Texture2D = access.GetInterface()?;
        let mut description = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&raw mut description);
        let size = (description.Width, description.Height);
        if staging
            .as_ref()
            .is_none_or(|(_, width, height)| (*width, *height) != size)
        {
            let staging_description = D3D11_TEXTURE2D_DESC {
                MipLevels: 1,
                ArraySize: 1,
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0.cast_unsigned(),
                MiscFlags: 0,
                ..description
            };
            let mut created = None;
            direct3d.device.CreateTexture2D(
                &raw const staging_description,
                None,
                Some(&raw mut created),
            )?;
            *staging = created.map(|texture| (texture, size.0, size.1));
        }
        let Some((staging_texture, _, _)) = staging.as_ref() else {
            return Ok(None);
        };
        direct3d.context.CopyResource(staging_texture, &texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        direct3d
            .context
            .Map(staging_texture, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped))?;
        let width = u32::try_from(content.Width).unwrap_or(0).min(size.0);
        let height = u32::try_from(content.Height).unwrap_or(0).min(size.1);
        let stride = mapped.RowPitch as usize;
        let picture = (!mapped.pData.is_null() && width > 0 && height > 0)
            .then(|| {
                let length = stride * (height as usize - 1) + width as usize * 4;
                std::slice::from_raw_parts(mapped.pData.cast::<u8>(), length)
            })
            .and_then(|pixels| pack_bgra_rows(bytes, width, height, stride, pixels, false));
        direct3d.context.Unmap(staging_texture, 0);
        Ok(picture)
    }
}

/// What a running capture's thread shares with it.
struct CaptureState {
    should_stop: AtomicBool,
    /// Set by the item's `Closed` event: the window closed or the display went away.
    is_closed: Arc<AtomicBool>,
}

/// A started `Windows.Graphics.Capture` session and what reading it needs.
struct CaptureSession {
    direct3d: Direct3D,
    item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    closed_token: i64,
    size: SizeInt32,
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

fn start_session(
    source: Source,
    is_closed: &Arc<AtomicBool>,
) -> windows::core::Result<CaptureSession> {
    let direct3d = create_direct3d()?;
    let item = capture_item(source)?;
    let size = item.Size()?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &direct3d.capture_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        FRAME_POOL_BUFFERS,
        size,
    )?;
    let session = pool.CreateCaptureSession(&item)?;
    let closed = Arc::clone(is_closed);
    let closed_token = item.Closed(
        &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
            closed.store(true, Ordering::Relaxed);
            Ok(())
        }),
    )?;
    session.StartCapture()?;
    Ok(CaptureSession {
        direct3d,
        item,
        pool,
        session,
        closed_token,
        size,
    })
}

/// Reads the newest picture 30 times a second until the share or the capture stops.
fn capture_loop(
    mut capture: CaptureSession,
    source: Source,
    pictures: &VideoInput,
    state: &CaptureState,
) -> Result<(), ScreenShareEnd> {
    let interval = Duration::from_secs(1) / FRAMES_PER_SECOND;
    let own_process = own_process_id();
    let mut excluded = Vec::new();
    let mut staging = None;
    let mut next_picture_at = Instant::now();
    let result = loop {
        if state.should_stop.load(Ordering::Relaxed) || !pictures.is_open() {
            break Ok(());
        }
        if state.is_closed.load(Ordering::Relaxed) {
            break Err(ScreenShareEnd::SourceClosed);
        }
        if matches!(source, Source::Display(_)) {
            exclude_own_windows(own_process, &mut excluded);
        }
        // Only the newest frame is read; older ones go back to the pool at once.
        let mut newest: Option<Direct3D11CaptureFrame> = None;
        while let Ok(frame) = capture.pool.TryGetNextFrame() {
            if let Some(older) = newest.replace(frame) {
                let _ = older.Close();
            }
        }
        if let Some(frame) = newest {
            let read = read_frame(
                &capture.direct3d,
                &frame,
                &mut staging,
                pictures.recycled_bytes(),
            );
            let content = frame.ContentSize();
            let _ = frame.Close();
            match read {
                Ok(Some(picture)) => pictures.push(picture),
                Ok(None) => {}
                Err(error) => break Err(ScreenShareEnd::Failed(error.message())),
            }
            // A resized window or display: the next frames come at the new size.
            if let Ok(content) = content
                && content != capture.size
            {
                capture.size = content;
                if let Err(error) = capture.pool.Recreate(
                    &capture.direct3d.capture_device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    FRAME_POOL_BUFFERS,
                    content,
                ) {
                    break Err(ScreenShareEnd::Failed(error.message()));
                }
            }
        }
        next_picture_at += interval;
        let now = Instant::now();
        if next_picture_at < now {
            // A slow read: no catching up with a burst.
            next_picture_at = now;
        }
        std::thread::sleep(next_picture_at.saturating_duration_since(now));
    };
    restore_own_windows(&excluded);
    result
}

struct WindowsCapture {
    state: Arc<CaptureState>,
    thread: Option<JoinHandle<()>>,
    sound_capture: Option<RunningShareSound>,
    sound: ShareSoundState,
}

impl RunningScreenCapture for WindowsCapture {
    fn sound(&self) -> ShareSoundState {
        self.sound
    }
}

impl Drop for WindowsCapture {
    fn drop(&mut self) {
        self.sound_capture = None;
        self.state.should_stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the Windows screen capture's thread panicked");
        }
    }
}

/// Whether `source` is still there to capture.
fn source_exists(source: Source) -> bool {
    match source {
        Source::Display(monitor) => displays().iter().any(|(listed, _)| *listed == monitor),
        // SAFETY: a plain query; a stale handle answers false.
        Source::Window(window) => unsafe { IsWindow(Some(window)) }.as_bool(),
    }
}

fn start(
    choice: &ScreenChoice,
    sound: &ShareSound,
    share: &ScreenShareContext,
) -> Result<Box<dyn RunningScreenCapture>, ScreenCaptureError> {
    let ScreenChoice::Source(id) = choice else {
        return Err(ScreenCaptureError::Unavailable);
    };
    let source = parse_source(id)
        .filter(|source| source_exists(*source))
        .ok_or(ScreenCaptureError::SourceGone)?;
    let state = Arc::new(CaptureState {
        should_stop: AtomicBool::new(false),
        is_closed: Arc::new(AtomicBool::new(false)),
    });
    let (started, has_started) = mpsc::sync_channel(1);
    let pictures = share.start_pictures();
    let thread = {
        let state = Arc::clone(&state);
        let capture_share = share.clone();
        // Handles are not `Send`: the thread reads the source again from its id.
        let id = id.clone();
        // The capture's objects are made, used and released on this thread.
        std::thread::Builder::new()
            .name("mellow-screen-windows".into())
            .spawn(move || {
                ensure_com();
                let Some(source) = parse_source(&id) else {
                    let _ = started.send(Err("the source is gone".into()));
                    return;
                };
                match start_session(source, &state.is_closed) {
                    Ok(capture) => {
                        let _ = started.send(Ok(()));
                        if let Err(reason) = capture_loop(capture, source, &pictures, &state) {
                            capture_share.end(reason);
                        }
                    }
                    Err(error) => {
                        let _ = started.send(Err(error.message()));
                    }
                }
            })
            .map_err(|error| {
                share.stop();
                system_error(error)
            })?
    };
    let mut capture = WindowsCapture {
        state,
        thread: Some(thread),
        sound_capture: None,
        sound: ShareSoundState::Off,
    };
    let capture_started = has_started
        .recv_timeout(START_TIMEOUT)
        .unwrap_or_else(|_| Err("the capture did not start".into()));
    if let Err(message) = capture_started {
        share.stop();
        drop(capture);
        return Err(ScreenCaptureError::System(message));
    }
    let window_process = match source {
        Source::Display(_) => None,
        Source::Window(window) => Some(window_process(window)),
    };
    let (target, sound_state) = loopback_target(sound, window_process, own_process_id());
    capture.sound = sound_state;
    if let Some(target) = target {
        match start_share_sound(target, share) {
            Ok(running) => capture.sound_capture = Some(running),
            Err(error) => {
                tracing::warn!(%error, "the share goes without sound");
                capture.sound = ShareSoundState::NotAvailable;
            }
        }
    }
    Ok(Box::new(capture))
}

impl ScreenSources for SystemScreenSources {
    fn picker(&self) -> ScreenPicker {
        ensure_com();
        // Windows 10 1903 and later.
        if GraphicsCaptureSession::IsSupported().unwrap_or(false) {
            ScreenPicker::InApp
        } else {
            ScreenPicker::Unavailable
        }
    }

    fn sources(
        &self,
        with_thumbnails: bool,
    ) -> Result<Vec<ScreenSourceDescription>, ScreenCaptureError> {
        Ok(list_sources(with_thumbnails))
    }

    fn sound_applications(&self) -> Vec<SoundApplication> {
        crate::screen_sound_windows::sound_applications()
    }

    fn start(
        &self,
        choice: &ScreenChoice,
        sound: &ShareSound,
        share: ScreenShareContext,
    ) -> Result<Box<dyn RunningScreenCapture>, ScreenCaptureError> {
        start(choice, sound, &share)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_ids_name_a_display_or_a_window_and_nothing_else() {
        let window = Source::Window(HWND(std::ptr::without_provenance_mut(0x1_0042)));
        assert_eq!(parse_source(&source_id(window)), Some(window));
        let display = Source::Display(HMONITOR(std::ptr::without_provenance_mut(65_539)));
        assert_eq!(parse_source(&source_id(display)), Some(display));
        assert_eq!(parse_source("window:0"), None);
        assert_eq!(parse_source("window:abc"), None);
        assert_eq!(parse_source("monitor:1"), None);
    }

    #[test]
    fn thumbnails_keep_the_shape_within_the_longest_side() {
        assert_eq!(thumbnail_size(1_920, 1_080), (320, 180));
        assert_eq!(thumbnail_size(1_080, 1_920), (180, 320));
        assert_eq!(thumbnail_size(200, 100), (200, 100));
    }

    #[test]
    fn the_listing_leaves_mellow_s_own_windows_out() {
        let own = own_process_id();
        for source in list_sources(true) {
            assert_ne!(source.id, "");
            if let Some(Source::Window(window)) = parse_source(&source.id) {
                assert_ne!(window_process(window), own);
            }
            if let Some(thumbnail) = &source.thumbnail {
                assert!(thumbnail.width <= THUMBNAIL_LONGEST_SIDE);
                assert!(thumbnail.height <= THUMBNAIL_LONGEST_SIDE);
            }
        }
    }

    #[test]
    fn a_gone_window_is_refused_before_the_share_turns_on() {
        let gone = Source::Window(HWND(std::ptr::without_provenance_mut(0x7fff_fff0)));
        assert!(!source_exists(gone));
    }
}

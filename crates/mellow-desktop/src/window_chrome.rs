//! The main window's options and chrome (DESIGN.md §4.2, Window chrome).
//!
//! macOS: the title bar is transparent, so the dusk ground runs under it and the traffic lights sit
//! in a ground band above the cards; `AppKit` still drags the window by that band.
//! Linux: client-side decorations. Mellow draws a title band of ground with its own buttons, and,
//! while the window floats, a transparent margin holding the shadow and the resize grips. When the
//! system insists on drawing the frame (`Decorations::Server`: a compositor without client-side
//! decorations, X11 without a compositor), Mellow draws none of it.
//! Windows: the system title bar is hidden and the system keeps its frame, shadow and corners;
//! the band and the buttons are marked as window control areas, so the system drags, snaps and
//! opens its menu from them.

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Corners, CursorStyle, Decorations, ElementId,
    HitboxBehavior, Hsla, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement as _, Pixels, Point, RenderOnce, ResizeEdge, Role, SharedString,
    Size, StatefulInteractiveElement as _, Styled as _, Tiling, TitlebarOptions, Window,
    WindowBounds, WindowControlArea, WindowControls, WindowDecorations, WindowOptions, canvas, div,
    point, prelude::FluentBuilder as _, px, size,
};

use crate::components::{
    FocusNextControl, FocusPreviousControl, child_id, colour_transition, focus_ring, icon,
    is_focus_visible, remembered_focus_handle, remembered_hover, shadow,
};
use crate::i18n::strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_WINDOW_CHROME_BAND_HEIGHT, COMPONENT_WINDOW_CHROME_BAND_PADDING_END,
    COMPONENT_WINDOW_CHROME_BUTTON_PILL_HEIGHT, COMPONENT_WINDOW_CHROME_BUTTON_PILL_INSET_INLINE,
    COMPONENT_WINDOW_CHROME_BUTTON_RADIUS, COMPONENT_WINDOW_CHROME_BUTTON_WIDTH,
    COMPONENT_WINDOW_CHROME_FRAME_BORDER_WIDTH, COMPONENT_WINDOW_CHROME_FRAME_INSET,
    COMPONENT_WINDOW_CHROME_FRAME_RADIUS, COMPONENT_WINDOW_CHROME_ICON_SIZE, SIZE_SHEET_INSET,
    SIZE_WINDOW_DEFAULT_HEIGHT, SIZE_WINDOW_DEFAULT_WIDTH, SIZE_WINDOW_MIN_HEIGHT,
    SIZE_WINDOW_MIN_WIDTH, SPACE_MD, ThemeTokens,
};
use crate::theme::{color, length, theme};

pub const WINDOW_TITLE: &str = "Mellow";
/// The Linux desktop entry's name, so docks group the window with it.
const APP_ID: &str = "mellow";

/// Height of the ground band the macOS traffic lights sit in; nothing elsewhere, where the band
/// Mellow draws itself depends on the window's decorations (`client_title_band_height`).
#[cfg(target_os = "macos")]
pub const TITLE_BAR_BAND_HEIGHT: f32 = crate::theme::tokens::SPACE_XL;
#[cfg(not(target_os = "macos"))]
pub const TITLE_BAR_BAND_HEIGHT: f32 = 0.0;

/// The ground above the cards: twice the inset, as on the web, or the title bar band if taller.
pub const TOP_BAND_HEIGHT: f32 = if TITLE_BAR_BAND_HEIGHT > 2.0 * SIZE_SHEET_INSET {
    TITLE_BAR_BAND_HEIGHT
} else {
    2.0 * SIZE_SHEET_INSET
};

/// The close, minimise and zoom buttons of macOS are this tall; centring them in the band needs it.
const MACOS_WINDOW_BUTTON_HEIGHT: f32 = 14.0;
/// How far the pointer travels with the button down on the band before the window moves, so a
/// slightly shaky click or double click stays a click.
const WINDOW_MOVE_DRAG_THRESHOLD: Pixels = px(3.0);
/// The default window covers at most this share of the screen (DESIGN.md §4.2).
const DEFAULT_WINDOW_SHARE_OF_SCREEN: f32 = 0.8;
/// A corner grip reaches this far along both edges, a little further than the edge grips, so a
/// corner is easy to catch (Zed's ratio).
const CORNER_GRIP_SPAN_IN_FRAME_INSETS: f32 = 1.5;

#[must_use]
pub fn main_window_options(cx: &App) -> WindowOptions {
    let screen_size = cx.primary_display().map(|display| display.bounds().size);
    let minimum_size = size(px(SIZE_WINDOW_MIN_WIDTH), px(SIZE_WINDOW_MIN_HEIGHT));
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            default_window_size(screen_size),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(WINDOW_TITLE.into()),
            // macOS: the ground runs under the title bar. Windows: the system title bar is hidden
            // and Mellow's band takes its place. Linux ignores it.
            appears_transparent: cfg!(any(target_os = "macos", target_os = "windows")),
            traffic_light_position: cfg!(target_os = "macos").then(|| {
                point(
                    px(SPACE_MD),
                    px((TITLE_BAR_BAND_HEIGHT - MACOS_WINDOW_BUTTON_HEIGHT) / 2.0),
                )
            }),
        }),
        app_id: Some(APP_ID.to_owned()),
        window_min_size: Some(minimum_size),
        // Linux: a request; the compositor may still decide (`Decorations::Server`).
        window_decorations: Some(if cfg!(target_os = "linux") {
            WindowDecorations::Client
        } else {
            WindowDecorations::Server
        }),
        ..WindowOptions::default()
    }
}

/// 1280 × 800, or 80 % of a smaller screen, never below the minimum window size.
fn default_window_size(screen_size: Option<Size<Pixels>>) -> Size<Pixels> {
    let fit = |default: f32, minimum: f32, screen: Option<Pixels>| {
        let available = screen.map_or(default, |screen| {
            f32::from(screen) * DEFAULT_WINDOW_SHARE_OF_SCREEN
        });
        px(default.min(available).max(minimum))
    };
    size(
        fit(
            SIZE_WINDOW_DEFAULT_WIDTH,
            SIZE_WINDOW_MIN_WIDTH,
            screen_size.map(|screen| screen.width),
        ),
        fit(
            SIZE_WINDOW_DEFAULT_HEIGHT,
            SIZE_WINDOW_MIN_HEIGHT,
            screen_size.map(|screen| screen.height),
        ),
    )
}

/// The system the window's chrome is drawn for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowSystem {
    MacOs,
    Windows,
    /// Linux and the other Unix desktops GPUI runs on (Wayland, X11).
    Linux,
}

const CURRENT_WINDOW_SYSTEM: WindowSystem = if cfg!(target_os = "macos") {
    WindowSystem::MacOs
} else if cfg!(target_os = "windows") {
    WindowSystem::Windows
} else {
    WindowSystem::Linux
};

/// What Mellow draws of the window's chrome itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClientChrome {
    /// The title band with the minimise, maximise and close buttons.
    pub is_title_band_drawn: bool,
    /// Linux client-side decorations: the edges the window is tiled on, which get no margin,
    /// grip, outline or rounded corner. `None` when the system draws the frame.
    pub client_frame_tiling: Option<Tiling>,
}

impl ClientChrome {
    /// The height of the title band Mellow draws, 0 when it draws none.
    #[must_use]
    pub fn title_band_height(self) -> f32 {
        if self.is_title_band_drawn {
            COMPONENT_WINDOW_CHROME_BAND_HEIGHT
        } else {
            0.0
        }
    }
}

fn client_chrome_for(
    system: WindowSystem,
    decorations: Decorations,
    is_fullscreen: bool,
) -> ClientChrome {
    match (system, decorations) {
        (WindowSystem::MacOs, _) | (WindowSystem::Linux, Decorations::Server) => {
            ClientChrome::default()
        }
        (WindowSystem::Windows, _) => ClientChrome {
            is_title_band_drawn: !is_fullscreen,
            client_frame_tiling: None,
        },
        (WindowSystem::Linux, Decorations::Client { tiling }) => ClientChrome {
            is_title_band_drawn: !is_fullscreen,
            // GPUI reports a maximised or full-screen window as tiled on every edge.
            client_frame_tiling: Some(if is_fullscreen {
                Tiling::tiled()
            } else {
                tiling
            }),
        },
    }
}

/// What Mellow draws of this window's chrome, as its decorations stand now.
#[must_use]
pub fn client_chrome(window: &Window) -> ClientChrome {
    client_chrome_for(
        CURRENT_WINDOW_SYSTEM,
        window.window_decorations(),
        window.is_fullscreen(),
    )
}

/// The height of the title band Mellow draws above the window's content, 0 when it draws none.
#[must_use]
pub fn client_title_band_height(window: &Window) -> f32 {
    client_chrome(window).title_band_height()
}

/// The transparent frame margins on the left and right of the window's content, together.
#[must_use]
pub fn client_frame_horizontal_inset(window: &Window) -> Pixels {
    client_chrome(window)
        .client_frame_tiling
        .map_or(px(0.0), |tiling| {
            let margins = frame_margins(tiling, px(COMPONENT_WINDOW_CHROME_FRAME_INSET));
            margins.left + margins.right
        })
}

/// The window's content area in window coordinates: inside a floating Linux window's frame
/// margins and outline, and under the title band Mellow draws. Full-window layers (scrims, the
/// enlarged video) and menus stay inside it, so the band keeps moving, minimising and closing the
/// window while they are open, and nothing spills into the transparent margins.
#[must_use]
pub fn client_content_bounds(window: &Window) -> Bounds<Pixels> {
    let band_height = length(COMPONENT_WINDOW_CHROME_BAND_HEIGHT).to_pixels(window.rem_size());
    content_bounds_for(client_chrome(window), window.viewport_size(), band_height)
}

fn content_bounds_for(
    chrome: ClientChrome,
    viewport: Size<Pixels>,
    band_height: Pixels,
) -> Bounds<Pixels> {
    let mut edges = chrome
        .client_frame_tiling
        .map_or_else(gpui::Edges::default, |tiling| {
            let outline = frame_margins(tiling, px(COMPONENT_WINDOW_CHROME_FRAME_BORDER_WIDTH));
            let margins = frame_margins(tiling, px(COMPONENT_WINDOW_CHROME_FRAME_INSET));
            gpui::Edges {
                top: margins.top + outline.top,
                right: margins.right + outline.right,
                bottom: margins.bottom + outline.bottom,
                left: margins.left + outline.left,
            }
        });
    if chrome.is_title_band_drawn {
        edges.top += band_height;
    }
    Bounds::new(
        point(edges.left, edges.top),
        size(
            (viewport.width - edges.left - edges.right).max(px(0.0)),
            (viewport.height - edges.top - edges.bottom).max(px(0.0)),
        ),
    )
}

/// Sizes a full-window layer drawn at `client_content_bounds`' origin to the content area, its
/// bottom corners following a floating window's rounding.
pub fn sized_to_window_content<E: gpui::Styled>(layer: E, window: &Window) -> E {
    let bounds = client_content_bounds(window);
    let corners = client_frame_inner_corner_radii(window);
    layer
        .w(bounds.size.width)
        .h(bounds.size.height)
        .rounded_bl(length(corners.bottom_left))
        .rounded_br(length(corners.bottom_right))
}

/// The rounding of the content's corners inside a floating Linux window's outline, so a fill that
/// reaches a corner (the full-bleed sheet below 816 px) follows the frame instead of squaring it:
/// GPUI does not clip children to rounded corners. All zero when the frame is not Mellow's.
#[must_use]
pub fn client_frame_inner_corner_radii(window: &Window) -> Corners<f32> {
    client_chrome(window)
        .client_frame_tiling
        .map_or_else(Corners::default, |tiling| {
            frame_corner_radii(
                tiling,
                COMPONENT_WINDOW_CHROME_FRAME_RADIUS - COMPONENT_WINDOW_CHROME_FRAME_BORDER_WIDTH,
            )
        })
}

/// A floating frame's corner radii: a corner is round only where both its edges are free.
fn frame_corner_radii(tiling: Tiling, radius: f32) -> Corners<f32> {
    let radius_unless =
        |is_either_edge_tiled: bool| if is_either_edge_tiled { 0.0 } else { radius };
    Corners {
        top_left: radius_unless(tiling.top || tiling.left),
        top_right: radius_unless(tiling.top || tiling.right),
        bottom_right: radius_unless(tiling.bottom || tiling.right),
        bottom_left: radius_unless(tiling.bottom || tiling.left),
    }
}

/// The transparent margin on each edge of a client-side frame: none on a tiled edge.
fn frame_margins(tiling: Tiling, inset: Pixels) -> gpui::Edges<Pixels> {
    let margin_unless = |is_tiled: bool| if is_tiled { px(0.0) } else { inset };
    gpui::Edges {
        top: margin_unless(tiling.top),
        right: margin_unless(tiling.right),
        bottom: margin_unless(tiling.bottom),
        left: margin_unless(tiling.left),
    }
}

/// The resize grip under `position`, if any: the margin of every edge that is not tiled, and a
/// slightly larger square at each corner whose two edges are free.
fn resize_edge_at(
    position: Point<Pixels>,
    window_size: Size<Pixels>,
    inset: Pixels,
    tiling: Tiling,
) -> Option<ResizeEdge> {
    let corner_span = inset * CORNER_GRIP_SPAN_IN_FRAME_INSETS;
    let is_near_top = position.y < corner_span;
    let is_near_bottom = position.y > window_size.height - corner_span;
    let is_near_left = position.x < corner_span;
    let is_near_right = position.x > window_size.width - corner_span;
    let corner = match (is_near_top, is_near_bottom, is_near_left, is_near_right) {
        (true, _, true, _) if !tiling.top && !tiling.left => Some(ResizeEdge::TopLeft),
        (true, _, _, true) if !tiling.top && !tiling.right => Some(ResizeEdge::TopRight),
        (_, true, true, _) if !tiling.bottom && !tiling.left => Some(ResizeEdge::BottomLeft),
        (_, true, _, true) if !tiling.bottom && !tiling.right => Some(ResizeEdge::BottomRight),
        _ => None,
    };
    corner.or_else(|| {
        if !tiling.top && position.y < inset {
            Some(ResizeEdge::Top)
        } else if !tiling.bottom && position.y > window_size.height - inset {
            Some(ResizeEdge::Bottom)
        } else if !tiling.left && position.x < inset {
            Some(ResizeEdge::Left)
        } else if !tiling.right && position.x > window_size.width - inset {
            Some(ResizeEdge::Right)
        } else {
            None
        }
    })
}

fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// Wraps the window's content (already holding its title band) in the chrome Mellow draws: on
/// Linux with client-side decorations, the frame margin, resize grips, outline, rounded corners
/// and shadow of a floating window; elsewhere, the ground alone.
pub fn framed_window_content(
    content: impl IntoElement,
    chrome: ClientChrome,
    ground: Hsla,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(tiling) = chrome.client_frame_tiling else {
        return div()
            .size_full()
            .bg(ground)
            .child(content)
            .into_any_element();
    };
    let tokens = theme(cx);
    let inset = px(COMPONENT_WINDOW_CHROME_FRAME_INSET);
    window.set_client_inset(inset);
    let margins = frame_margins(tiling, inset);
    let radii = frame_corner_radii(tiling, COMPONENT_WINDOW_CHROME_FRAME_RADIUS);
    let border_width = px(COMPONENT_WINDOW_CHROME_FRAME_BORDER_WIDTH);
    let grip_under_pointer = window.use_keyed_state(
        ElementId::Name("window-frame-grip-under-pointer".into()),
        cx,
        |_, _| None::<ResizeEdge>,
    );

    div()
        .id("window-frame")
        .size_full()
        .pt(margins.top)
        .pr(margins.right)
        .pb(margins.bottom)
        .pl(margins.left)
        .child(
            canvas(
                // The whole window, from its corner: the frame's padding would offset a box
                // laid out here, leaving the top and left grips without their cursor.
                |_, window, _| {
                    window.insert_hitbox(
                        Bounds::new(point(px(0.0), px(0.0)), window.viewport_size()),
                        HitboxBehavior::Normal,
                    )
                },
                move |_, hitbox, window, _| {
                    let edge = resize_edge_at(
                        window.mouse_position(),
                        window.viewport_size(),
                        inset,
                        tiling,
                    );
                    if let Some(edge) = edge {
                        window.set_cursor_style(resize_cursor(edge), &hitbox);
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        // Redraws only when the pointer crosses into or out of a grip, for its cursor.
        .on_mouse_move(move |event: &MouseMoveEvent, window, cx| {
            let edge = resize_edge_at(event.position, window.viewport_size(), inset, tiling);
            if *grip_under_pointer.read(cx) != edge {
                grip_under_pointer.update(cx, |remembered, _| *remembered = edge);
                window.refresh();
            }
        })
        .on_mouse_down(
            MouseButton::Left,
            move |event: &MouseDownEvent, window, _| {
                if let Some(edge) =
                    resize_edge_at(event.position, window.viewport_size(), inset, tiling)
                {
                    window.start_window_resize(edge);
                }
            },
        )
        .child(
            div()
                .relative()
                .size_full()
                .bg(ground)
                .border_color(color(tokens.component_window_chrome_frame_border))
                .when(!tiling.top, |frame| frame.border_t(border_width))
                .when(!tiling.right, |frame| frame.border_r(border_width))
                .when(!tiling.bottom, |frame| frame.border_b(border_width))
                .when(!tiling.left, |frame| frame.border_l(border_width))
                .rounded_tl(length(radii.top_left))
                .rounded_tr(length(radii.top_right))
                .rounded_br(length(radii.bottom_right))
                .rounded_bl(length(radii.bottom_left))
                .when(!tiling.is_tiled(), |frame| {
                    frame.shadow(shadow(tokens.component_window_chrome_frame_shadow))
                })
                .child(content),
        )
        .into_any_element()
}

/// A button of the title band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowButton {
    Minimise,
    Maximise,
    /// The maximise button while the window is maximised.
    Restore,
    Close,
}

/// The buttons the band shows, in order: those the system supports, and Close always.
fn window_buttons(controls: WindowControls, is_maximized: bool) -> Vec<WindowButton> {
    let mut buttons = Vec::with_capacity(3);
    if controls.minimize {
        buttons.push(WindowButton::Minimise);
    }
    if controls.maximize {
        buttons.push(if is_maximized {
            WindowButton::Restore
        } else {
            WindowButton::Maximise
        });
    }
    buttons.push(WindowButton::Close);
    buttons
}

impl WindowButton {
    fn element_id(self) -> &'static str {
        match self {
            Self::Minimise => "window-button-minimise",
            Self::Maximise | Self::Restore => "window-button-maximise",
            Self::Close => "window-button-close",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Minimise => IconName::Minus,
            Self::Maximise => IconName::Square,
            Self::Restore => IconName::CopyOutline,
            Self::Close => IconName::X,
        }
    }

    fn label(self, cx: &App) -> &'static str {
        let desktop = strings(cx).desktop();
        match self {
            Self::Minimise => desktop.minimise_window(),
            Self::Maximise => desktop.maximise_window(),
            Self::Restore => desktop.restore_window(),
            Self::Close => desktop.close_window(),
        }
    }

    /// Where the Windows hit test sends this button, so the system runs it (and shows Snap
    /// Layouts over Maximise).
    fn control_area(self) -> WindowControlArea {
        match self {
            Self::Minimise => WindowControlArea::Min,
            Self::Maximise | Self::Restore => WindowControlArea::Max,
            Self::Close => WindowControlArea::Close,
        }
    }

    fn run(self, window: &mut Window) {
        match self {
            Self::Minimise => window.minimize_window(),
            Self::Maximise | Self::Restore => window.zoom_window(),
            // As the system's close button: Mellow may keep running in the background (ADR-0056).
            Self::Close => window.remove_window(),
        }
    }
}

/// The band at the window's top: drags the window, maximises it on a double click, opens the
/// system's window menu on a right click, and holds the buttons at its trailing end. Drawn over
/// the content's top, which leaves `client_title_band_height` of room for it.
pub fn title_band(window: &mut Window, cx: &mut App) -> impl IntoElement {
    let controls = window.window_controls();
    let buttons = window_buttons(controls, window.is_maximized());
    // Where the band was pressed: the move starts once the pointer has travelled from there, so a
    // double click can maximise.
    let press_position = window.use_keyed_state(
        ElementId::Name("title-band-press-position".into()),
        cx,
        |_, _| None::<Point<Pixels>>,
    );
    let forget_press = {
        let press_position = press_position.clone();
        move |cx: &mut App| press_position.update(cx, |pressed_at, _| *pressed_at = None)
    };
    let is_trailing_edge_flush = is_trailing_edge_flush_with_the_screen(window);

    div()
        .id("title-band")
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(length(COMPONENT_WINDOW_CHROME_BAND_HEIGHT))
        .flex()
        .flex_row()
        .justify_end()
        .when(!is_trailing_edge_flush, |band| {
            band.pr(length(COMPONENT_WINDOW_CHROME_BAND_PADDING_END))
        })
        // Tab from a button reaches the next control, the content's or the band's: the content's
        // own roots handle Tab only for what is inside them.
        .on_action(|_: &FocusNextControl, window, cx| window.focus_next(cx))
        .on_action(|_: &FocusPreviousControl, window, cx| window.focus_prev(cx))
        .map(|band| match CURRENT_WINDOW_SYSTEM {
            // The system drags, maximises and opens its menu from the caption area itself.
            WindowSystem::Windows => band.window_control_area(WindowControlArea::Drag),
            WindowSystem::MacOs | WindowSystem::Linux => band
                .on_mouse_down(MouseButton::Left, {
                    let press_position = press_position.clone();
                    move |event: &MouseDownEvent, window, cx| {
                        let pressed_at = (event.click_count < 2).then_some(event.position);
                        press_position.update(cx, |remembered, _| *remembered = pressed_at);
                        if event.click_count >= 2 && controls.maximize {
                            window.zoom_window();
                        }
                    }
                })
                .on_mouse_up(MouseButton::Left, {
                    let forget_press = forget_press.clone();
                    move |_, _, cx| forget_press(cx)
                })
                .on_mouse_down_out(move |_, _, cx| forget_press(cx))
                .on_mouse_move(move |event: &MouseMoveEvent, window, cx| {
                    let Some(pressed_at) = *press_position.read(cx) else {
                        return;
                    };
                    if event.pressed_button == Some(MouseButton::Left)
                        && has_travelled_far_enough_to_move(pressed_at, event.position)
                    {
                        press_position.update(cx, |remembered, _| *remembered = None);
                        window.start_window_move();
                    }
                })
                .when(controls.window_menu, |band| {
                    band.on_mouse_down(MouseButton::Right, |event: &MouseDownEvent, window, _| {
                        window.show_window_menu(event.position);
                    })
                }),
        })
        .children(buttons.into_iter().map(|button| WindowButtonElement {
            button,
            // Fitts: pushed to the screen's corner, the pointer still closes the window.
            reaches_the_window_edge: button == WindowButton::Close && is_trailing_edge_flush,
        }))
}

/// Whether the window's trailing edge touches the screen's (maximised, tiled to the right).
fn is_trailing_edge_flush_with_the_screen(window: &Window) -> bool {
    client_chrome(window)
        .client_frame_tiling
        .map_or_else(|| window.is_maximized(), |tiling| tiling.right)
}

fn has_travelled_far_enough_to_move(pressed_at: Point<Pixels>, now: Point<Pixels>) -> bool {
    let travel = now - pressed_at;
    travel.x.abs() >= WINDOW_MOVE_DRAG_THRESHOLD || travel.y.abs() >= WINDOW_MOVE_DRAG_THRESHOLD
}

/// A button's fill under the pointer, fill while pressed and icon under the pointer: Close turns
/// red, as on every desktop.
fn button_state_colours(button: WindowButton, tokens: &ThemeTokens) -> (u32, u32, u32) {
    if button == WindowButton::Close {
        (
            tokens.component_window_chrome_close_background_hover,
            tokens.component_window_chrome_close_background_pressed,
            tokens.component_window_chrome_close_icon_hover,
        )
    } else {
        (
            tokens.component_window_chrome_background_hover,
            tokens.component_window_chrome_background_pressed,
            tokens.component_window_chrome_icon_hover,
        )
    }
}

#[derive(IntoElement)]
struct WindowButtonElement {
    button: WindowButton,
    /// Takes the band's trailing padding into its target, up to the window's edge.
    reaches_the_window_edge: bool,
}

impl RenderOnce for WindowButtonElement {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let button = self.button;
        let tokens = theme(cx);
        let id = ElementId::Name(button.element_id().into());
        let group_name = SharedString::new_static(button.element_id());
        let focus_handle = remembered_focus_handle(&id, window, cx);
        let hover = remembered_hover(&id, window, cx);
        let is_hovered = *hover.read(cx);
        let (hover_fill, pressed_fill, hover_icon) = button_state_colours(button, tokens);
        let hover_fill = color(hover_fill);
        let target_fill = if is_hovered {
            hover_fill
        } else {
            // The hover colour at no opacity, so the fade does not pass through black.
            Hsla {
                a: 0.0,
                ..hover_fill
            }
        };
        let target_icon = color(if is_hovered {
            hover_icon
        } else {
            tokens.component_window_chrome_icon
        });
        let fill = colour_transition(child_id(&id, "background"), target_fill, window, cx);
        let icon_tint = colour_transition(child_id(&id, "icon"), target_icon, window, cx);
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        let is_run_by_the_system = CURRENT_WINDOW_SYSTEM == WindowSystem::Windows;
        let pill_id = child_id(&id, "pill");

        div()
            .id(id)
            .group(group_name.clone())
            .role(Role::Button)
            .aria_label(button.label(cx))
            .track_focus(&focus_handle)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(length(COMPONENT_WINDOW_CHROME_BUTTON_WIDTH))
            .when(self.reaches_the_window_edge, |target| {
                target
                    .w(length(
                        COMPONENT_WINDOW_CHROME_BUTTON_WIDTH
                            + COMPONENT_WINDOW_CHROME_BAND_PADDING_END,
                    ))
                    .pr(length(COMPONENT_WINDOW_CHROME_BAND_PADDING_END))
            })
            .h_full()
            // Above the band: the Windows hit test and the pointer find the button, not the
            // band's drag area behind it.
            .occlude()
            .when(is_run_by_the_system, |target| {
                target
                    .window_control_area(button.control_area())
                    // Keeps focus where it was, as the system's own buttons do; the press must
                    // still reach the system, so it is not stopped.
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            })
            .when(!is_run_by_the_system, |target| {
                // The band would start a move or a double-click maximise from this press.
                target.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            })
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .on_click(move |event: &ClickEvent, window, _| {
                // On Windows the system runs a pointer's click from the hit test; only keys come here.
                if !is_run_by_the_system || matches!(event, ClickEvent::Keyboard(_)) {
                    button.run(window);
                }
            })
            .child(
                div()
                    .id(pill_id)
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(length(
                        COMPONENT_WINDOW_CHROME_BUTTON_WIDTH
                            - 2.0 * COMPONENT_WINDOW_CHROME_BUTTON_PILL_INSET_INLINE,
                    ))
                    .h(length(COMPONENT_WINDOW_CHROME_BUTTON_PILL_HEIGHT))
                    .rounded(length(COMPONENT_WINDOW_CHROME_BUTTON_RADIUS))
                    .bg(fill)
                    .group_active(group_name, move |style| style.bg(color(pressed_fill)))
                    .child(icon(
                        button.icon(),
                        COMPONENT_WINDOW_CHROME_ICON_SIZE,
                        icon_tint,
                    ))
                    .when(is_ring_shown, |pill| {
                        pill.child(focus_ring(COMPONENT_WINDOW_CHROME_BUTTON_RADIUS, cx))
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::{DARK_THEME, LIGHT_THEME};

    /// Snapped to the left half of the screen.
    const LEFT_HALF: Tiling = Tiling {
        top: true,
        left: true,
        right: false,
        bottom: true,
    };

    const FLOATING: Decorations = Decorations::Client {
        tiling: Tiling {
            top: false,
            left: false,
            right: false,
            bottom: false,
        },
    };

    #[test]
    fn the_default_window_fits_small_screens_but_never_below_the_minimum() {
        assert_eq!(
            default_window_size(None),
            size(
                px(SIZE_WINDOW_DEFAULT_WIDTH),
                px(SIZE_WINDOW_DEFAULT_HEIGHT)
            )
        );
        assert_eq!(
            default_window_size(Some(size(px(2560.0), px(1440.0)))),
            size(px(1280.0), px(800.0))
        );
        let small_screen = default_window_size(Some(size(px(1366.0), px(768.0))));
        assert!((f32::from(small_screen.width) - 1092.8).abs() < 0.01);
        assert!((f32::from(small_screen.height) - 614.4).abs() < 0.01);
        assert_eq!(
            default_window_size(Some(size(px(400.0), px(400.0)))),
            size(px(SIZE_WINDOW_MIN_WIDTH), px(SIZE_WINDOW_MIN_HEIGHT))
        );
    }

    #[test]
    fn linux_draws_its_band_and_frame_only_with_client_side_decorations() {
        let floating = client_chrome_for(WindowSystem::Linux, FLOATING, false);
        assert!(floating.is_title_band_drawn);
        assert_eq!(floating.client_frame_tiling, Some(Tiling::default()));
        assert_eq!(
            floating.title_band_height(),
            COMPONENT_WINDOW_CHROME_BAND_HEIGHT
        );

        let system_frame = client_chrome_for(WindowSystem::Linux, Decorations::Server, false);
        assert_eq!(
            system_frame,
            ClientChrome::default(),
            "never two title bars"
        );
        assert!(system_frame.title_band_height().abs() < f32::EPSILON);
    }

    #[test]
    fn full_screen_hides_the_band_and_the_frame_margins() {
        let linux = client_chrome_for(WindowSystem::Linux, FLOATING, true);
        assert!(!linux.is_title_band_drawn);
        assert_eq!(linux.client_frame_tiling, Some(Tiling::tiled()));
        let windows = client_chrome_for(WindowSystem::Windows, Decorations::Server, true);
        assert!(!windows.is_title_band_drawn);
    }

    #[test]
    fn windows_draws_the_band_but_leaves_the_frame_to_the_system() {
        let windows = client_chrome_for(WindowSystem::Windows, Decorations::Server, false);
        assert!(windows.is_title_band_drawn);
        assert_eq!(windows.client_frame_tiling, None);
    }

    #[test]
    fn macos_keeps_its_own_title_bar() {
        for decorations in [Decorations::Server, FLOATING] {
            assert_eq!(
                client_chrome_for(WindowSystem::MacOs, decorations, false),
                ClientChrome::default()
            );
        }
    }

    #[test]
    fn the_band_keeps_maximised_windows_and_their_tiled_edges() {
        let maximised = Decorations::Client {
            tiling: Tiling::tiled(),
        };
        let chrome = client_chrome_for(WindowSystem::Linux, maximised, false);
        assert!(
            chrome.is_title_band_drawn,
            "the buttons stay while maximised"
        );
        assert_eq!(chrome.client_frame_tiling, Some(Tiling::tiled()));
    }

    #[test]
    fn buttons_follow_what_the_system_supports_and_close_always_shows() {
        let everything = WindowControls::default();
        assert_eq!(
            window_buttons(everything, false),
            [
                WindowButton::Minimise,
                WindowButton::Maximise,
                WindowButton::Close
            ]
        );
        assert_eq!(
            window_buttons(everything, true),
            [
                WindowButton::Minimise,
                WindowButton::Restore,
                WindowButton::Close
            ]
        );
        let close_only = WindowControls {
            fullscreen: false,
            maximize: false,
            minimize: false,
            window_menu: false,
        };
        assert_eq!(window_buttons(close_only, false), [WindowButton::Close]);
    }

    #[test]
    fn a_tiled_edge_loses_its_margin() {
        let inset = px(12.0);
        let free = frame_margins(Tiling::default(), inset);
        assert_eq!(
            (free.top, free.right, free.bottom, free.left),
            (inset, inset, inset, inset)
        );
        let left_half = frame_margins(LEFT_HALF, inset);
        assert_eq!(
            (
                left_half.top,
                left_half.right,
                left_half.bottom,
                left_half.left
            ),
            (px(0.0), inset, px(0.0), px(0.0))
        );
        let maximised = frame_margins(Tiling::tiled(), inset);
        assert_eq!(maximised.left + maximised.right, px(0.0));
    }

    #[test]
    fn a_corner_is_round_only_where_both_its_edges_are_free() {
        let free = frame_corner_radii(Tiling::default(), 16.0);
        assert_eq!(free, Corners::all(16.0));
        assert_eq!(
            frame_corner_radii(LEFT_HALF, 16.0),
            Corners::default(),
            "every corner of a left half touches a tiled edge"
        );
        let snapped_to_the_top = Tiling {
            top: true,
            ..Tiling::default()
        };
        let top_only = frame_corner_radii(snapped_to_the_top, 16.0);
        assert_eq!((top_only.top_left, top_only.bottom_left), (0.0, 16.0));
        assert_eq!(
            frame_corner_radii(Tiling::tiled(), 16.0),
            Corners::default()
        );
    }

    #[test]
    fn grips_cover_the_margin_of_free_edges_and_a_larger_square_at_corners() {
        let window = size(px(1000.0), px(800.0));
        let inset = px(12.0);
        let free = Tiling::default();
        let at = |x: f32, y: f32, tiling: Tiling| {
            resize_edge_at(point(px(x), px(y)), window, inset, tiling)
        };

        assert_eq!(at(500.0, 5.0, free), Some(ResizeEdge::Top));
        assert_eq!(at(500.0, 795.0, free), Some(ResizeEdge::Bottom));
        assert_eq!(at(5.0, 400.0, free), Some(ResizeEdge::Left));
        assert_eq!(at(995.0, 400.0, free), Some(ResizeEdge::Right));
        assert_eq!(at(3.0, 3.0, free), Some(ResizeEdge::TopLeft));
        // The corner square reaches past the margin: 15 px is inside the window yet a corner.
        assert_eq!(at(985.0, 15.0, free), Some(ResizeEdge::TopRight));
        assert_eq!(at(15.0, 785.0, free), Some(ResizeEdge::BottomLeft));
        assert_eq!(at(995.0, 795.0, free), Some(ResizeEdge::BottomRight));
        // Inside the window, away from the corners: the band and the content, never a grip.
        assert_eq!(at(500.0, 15.0, free), None);
        assert_eq!(at(500.0, 400.0, free), None);
    }

    #[test]
    fn tiled_edges_have_no_grip() {
        let window = size(px(1000.0), px(800.0));
        let inset = px(12.0);
        let at = |x: f32, y: f32, tiling: Tiling| {
            resize_edge_at(point(px(x), px(y)), window, inset, tiling)
        };

        assert_eq!(at(500.0, 5.0, Tiling::tiled()), None);
        assert_eq!(at(3.0, 3.0, Tiling::tiled()), None);
        let left_half = LEFT_HALF;
        assert_eq!(at(995.0, 400.0, left_half), Some(ResizeEdge::Right));
        assert_eq!(
            at(995.0, 3.0, left_half),
            Some(ResizeEdge::Right),
            "no corner on a tiled top"
        );
        assert_eq!(at(5.0, 400.0, left_half), None);
        assert_eq!(at(500.0, 3.0, left_half), None);
    }

    #[test]
    fn the_content_area_leaves_out_the_frame_margins_the_outline_and_the_band() {
        let viewport = size(px(1000.0), px(800.0));
        let band = px(32.0);
        let floating = client_chrome_for(WindowSystem::Linux, FLOATING, false);
        let content = content_bounds_for(floating, viewport, band);
        // 12 px of margin and 1 px of outline on each side, then the band on top.
        assert_eq!(content.origin, point(px(13.0), px(45.0)));
        assert_eq!(content.size, size(px(974.0), px(742.0)));

        let maximised = client_chrome_for(
            WindowSystem::Linux,
            Decorations::Client {
                tiling: Tiling::tiled(),
            },
            false,
        );
        let content = content_bounds_for(maximised, viewport, band);
        assert_eq!(content.origin, point(px(0.0), px(32.0)));
        assert_eq!(content.size, size(px(1000.0), px(768.0)));

        let system_frame = client_chrome_for(WindowSystem::Linux, Decorations::Server, false);
        let content = content_bounds_for(system_frame, viewport, band);
        assert_eq!(content, Bounds::new(point(px(0.0), px(0.0)), viewport));

        let windows = client_chrome_for(WindowSystem::Windows, Decorations::Server, false);
        assert_eq!(
            content_bounds_for(windows, viewport, band).origin,
            point(px(0.0), px(32.0))
        );
    }

    #[test]
    fn a_shaky_click_on_the_band_does_not_move_the_window() {
        let pressed_at = point(px(100.0), px(10.0));
        assert!(!has_travelled_far_enough_to_move(
            pressed_at,
            point(px(102.0), px(12.0))
        ));
        assert!(has_travelled_far_enough_to_move(
            pressed_at,
            point(px(103.0), px(10.0))
        ));
        assert!(has_travelled_far_enough_to_move(
            pressed_at,
            point(px(100.0), px(7.0))
        ));
    }

    #[test]
    fn close_turns_red_under_the_pointer_and_the_others_stay_neutral() {
        for tokens in [&DARK_THEME, &LIGHT_THEME] {
            assert_eq!(
                button_state_colours(WindowButton::Close, tokens).0,
                tokens.color_danger_fill
            );
            assert_eq!(
                button_state_colours(WindowButton::Minimise, tokens).0,
                tokens.color_state_hover
            );
        }
    }

    struct TitleBandHarness;

    impl gpui::Render for TitleBandHarness {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div().size_full().child(title_band(window, cx))
        }
    }

    #[gpui::test]
    fn tab_moves_through_the_window_buttons_and_never_stops_on_one(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme::for_appearance(
                gpui::WindowAppearance::Dark,
            ));
            crate::components::bind_tab_navigation(cx);
        });
        let (_, cx) = cx.add_window_view(|_, _| TitleBandHarness);
        cx.update(|window, cx| {
            window.activate_window();
            window.focus_next(cx);
        });
        cx.run_until_parked();
        let focused = |cx: &mut gpui::VisualTestContext| cx.update(|window, cx| window.focused(cx));
        let minimise = focused(cx);
        assert!(minimise.is_some(), "the first button takes focus");
        cx.simulate_keystrokes("tab");
        let maximise = focused(cx);
        assert_ne!(maximise, minimise, "Tab leaves Minimise");
        cx.simulate_keystrokes("tab");
        cx.simulate_keystrokes("tab");
        assert_eq!(focused(cx), minimise, "past Close, Tab goes round");
        cx.simulate_keystrokes("shift-tab");
        assert_ne!(focused(cx), minimise, "Shift+Tab goes back");
    }
}

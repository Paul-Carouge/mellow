//! A line of message text (`MessageMarkdown.svelte`'s `inlines` snippet): Markdown's inline nodes
//! as one GPUI text with highlight runs, so it wraps as one paragraph. Mentions, code, spoilers and
//! search matches are coloured runs; links and spoilers are click ranges; a server emoji keeps
//! room with no-break spaces and its picture is painted over them, since GPUI text holds no
//! inline boxes.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Bounds, ElementId, Entity, FontStyle, FontWeight, GlobalElementId, HighlightStyle, Hitbox,
    Hsla, ImageSource, InspectorElementId, InteractiveText, IntoElement, LayoutId, Pixels,
    SharedString, StrikethroughStyle, StyledText, TextLayout, UnderlineStyle, Window, point, px,
    size,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::markdown::{InlineNode, safe_http_url};
use mellow_client_core::shared_logic::role_colors::{
    RoleMentionChipColours, role_mention_chip_colours,
};
use mellow_client_core::shared_logic::role_mentions::role_mention_label;
use mellow_protocol::CustomEmoji;

use super::picture_for_box;
use crate::components::Tooltip;
use crate::i18n::Strings;
use crate::pictures::Animation;
use crate::theme::tokens::{
    COMPONENT_CUSTOM_EMOJI_MISSING_BORDER_WIDTH, COMPONENT_CUSTOM_EMOJI_MISSING_RADIUS,
    FONT_WEIGHT_BOLD, FONT_WEIGHT_MEDIUM, FONT_WEIGHT_SEMIBOLD, ThemeTokens,
};
use crate::theme::{MONOSPACE_FONT_FAMILY, color, length};

/// What the room of a server emoji is made of: no-break spaces never wrap apart.
const PICTURE_ROOM_CHARACTER: char = '\u{00A0}';
/// Source Code Pro's advance is 600 of its 1000 units per em; the room is set in it so its width
/// is known before layout.
const MONOSPACE_ADVANCE_SHARE_OF_EM: f32 = 0.6;
/// A thin space each side of a mention chip, standing in for its inline padding (GPUI text runs
/// have no padding).
const CHIP_PADDING: &str = "\u{2009}";
/// The colour of a run that keeps no colour of its own.
const NO_COLOUR_BYTES: u32 = 0;

/// What clicking a range of the text does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InlineAction {
    /// Only http and https addresses, checked again on click (`safe_http_url`).
    OpenLink(String),
    RevealSpoiler(u32),
}

/// A server emoji's picture over its room in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InlinePicture {
    pub room: Range<usize>,
    pub emoji: CustomEmoji,
}

/// One paragraph's text and how its ranges look and behave.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct RichText {
    pub text: String,
    pub highlights: Vec<(Range<usize>, HighlightStyle)>,
    pub monospace: Vec<Range<usize>>,
    pub actions: Vec<(Range<usize>, InlineAction)>,
    pub tooltips: Vec<(Range<usize>, SharedString)>,
    pub pictures: Vec<InlinePicture>,
}

/// What the text is drawn with.
#[derive(Clone, Copy)]
pub(crate) struct RichTextContext<'a> {
    pub tokens: &'static ThemeTokens,
    pub strings: Strings,
    pub revealed_spoilers: &'a HashSet<u32>,
    /// A server emoji's side as a share of the text's em: wider in jumbo messages' smaller ratio.
    pub picture_em: f32,
}

/// The style of the nodes being walked, inherited by their children.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent marks, as CSS sets them: any mix is valid"
)]
#[derive(Clone, Copy, Default)]
struct InlineStyle {
    weight: Option<u16>,
    is_italic: bool,
    is_underlined: bool,
    is_struck: bool,
    colour: Option<Hsla>,
    background: Option<Hsla>,
    is_monospace: bool,
    /// Inside a spoiler not revealed yet: invisible, and its links inactive.
    is_hidden: bool,
}

impl InlineStyle {
    fn highlight(&self) -> Option<HighlightStyle> {
        let highlight = HighlightStyle {
            color: self.colour,
            font_weight: self.weight.map(|weight| FontWeight(f32::from(weight))),
            font_style: self.is_italic.then_some(FontStyle::Italic),
            background_color: self.background,
            underline: self.is_underlined.then(|| UnderlineStyle {
                thickness: px(1.0),
                color: None,
                wavy: false,
            }),
            strikethrough: self.is_struck.then(|| StrikethroughStyle {
                thickness: px(1.0),
                color: None,
            }),
            fade_out: None,
        };
        (highlight != HighlightStyle::default()).then_some(highlight)
    }
}

thread_local! {
    /// Worked out once per colour and theme: every chip of a role in every row shares it.
    static ROLE_CHIP_COLOURS: RefCell<HashMap<(u32, u32), RoleMentionChipColours>> =
        RefCell::new(HashMap::new());
}

fn role_chip_colours(role_colour: u32, sheet: u32) -> RoleMentionChipColours {
    ROLE_CHIP_COLOURS.with(|cache| {
        *cache
            .borrow_mut()
            .entry((role_colour, sheet))
            .or_insert_with(|| role_mention_chip_colours(role_colour, sheet))
    })
}

/// `0xRRGGBB` as an opaque GPUI colour.
fn opaque(rgb: u32) -> Hsla {
    color((rgb << 8) | 0xFF)
}

impl RichText {
    pub fn from_inlines(nodes: &[InlineNode], context: RichTextContext<'_>) -> Self {
        let mut rich_text = Self::default();
        rich_text.push_nodes(nodes, InlineStyle::default(), context);
        rich_text
    }

    /// "(edited)" after the last word, with its tooltip.
    pub fn push_trailing(&mut self, text: &str, colour: Hsla, tooltip: SharedString) {
        self.text.push(' ');
        let range = self.push_text(
            text,
            InlineStyle {
                colour: Some(colour),
                ..InlineStyle::default()
            },
        );
        self.tooltips.push((range, tooltip));
    }

    fn push_text(&mut self, text: &str, style: InlineStyle) -> Range<usize> {
        let start = self.text.len();
        self.text.push_str(text);
        let range = start..self.text.len();
        if range.is_empty() {
            return range;
        }
        if let Some(highlight) = style.highlight() {
            self.highlights.push((range.clone(), highlight));
        }
        if style.is_monospace {
            self.monospace.push(range.clone());
        }
        range
    }

    fn push_chip(&mut self, text: &str, background: Hsla, colour: Hsla, style: InlineStyle) {
        let chip = InlineStyle {
            weight: Some(FONT_WEIGHT_MEDIUM),
            ..style
        };
        let chip = if style.is_hidden {
            chip
        } else {
            InlineStyle {
                background: Some(background),
                colour: Some(colour),
                ..chip
            }
        };
        self.push_text(&format!("{CHIP_PADDING}{text}{CHIP_PADDING}"), chip);
    }

    #[allow(clippy::too_many_lines)] // One arm per node kind, as the web's snippet.
    fn push_nodes(
        &mut self,
        nodes: &[InlineNode],
        style: InlineStyle,
        context: RichTextContext<'_>,
    ) {
        let tokens = context.tokens;
        for node in nodes {
            match node {
                InlineNode::Text(text) => {
                    self.push_text(text, style);
                }
                InlineNode::Match(text) => {
                    let matched = if style.is_hidden {
                        style
                    } else {
                        InlineStyle {
                            background: Some(color(tokens.component_search_match_background)),
                            colour: Some(color(tokens.component_search_match_text)),
                            weight: Some(FONT_WEIGHT_SEMIBOLD),
                            ..style
                        }
                    };
                    self.push_text(text, matched);
                }
                InlineNode::Mention { text, .. } | InlineNode::EveryoneMention(text) => self
                    .push_chip(
                        text,
                        color(tokens.component_mention_chip_background),
                        color(tokens.component_mention_chip_text),
                        style,
                    ),
                InlineNode::RoleMention(role) => {
                    let label = role_mention_label(
                        role.as_ref(),
                        context.strings.messages().deleted_role_name(),
                    );
                    match role {
                        None => {
                            self.push_text(&label, style);
                        }
                        Some(role) if role.color == NO_COLOUR_BYTES => self.push_chip(
                            &label,
                            color(tokens.component_mention_chip_background),
                            color(tokens.component_mention_chip_text),
                            style,
                        ),
                        Some(role) => {
                            let chip =
                                role_chip_colours(role.color, tokens.color_surface_chat >> 8);
                            self.push_chip(
                                &label,
                                opaque(chip.background),
                                opaque(chip.text),
                                style,
                            );
                        }
                    }
                }
                InlineNode::Code(text) => {
                    let code = InlineStyle {
                        is_monospace: true,
                        background: (!style.is_hidden)
                            .then(|| color(tokens.component_message_code_background))
                            .or(style.background),
                        ..style
                    };
                    self.push_text(text, code);
                }
                InlineNode::CustomEmoji(emoji) => {
                    let room_length = picture_room_length(context.picture_em);
                    let room_text: String =
                        std::iter::repeat_n(PICTURE_ROOM_CHARACTER, room_length).collect();
                    let room = self.push_text(
                        &room_text,
                        InlineStyle {
                            is_monospace: true,
                            ..style
                        },
                    );
                    if !style.is_hidden {
                        self.tooltips
                            .push((room.clone(), format!(":{}:", emoji.name).into()));
                        self.pictures.push(InlinePicture {
                            room,
                            emoji: emoji.clone(),
                        });
                    }
                }
                InlineNode::Strong(children) => self.push_nodes(
                    children,
                    InlineStyle {
                        weight: Some(FONT_WEIGHT_BOLD),
                        ..style
                    },
                    context,
                ),
                InlineNode::Emphasis(children) => self.push_nodes(
                    children,
                    InlineStyle {
                        is_italic: true,
                        ..style
                    },
                    context,
                ),
                InlineNode::Underline(children) => self.push_nodes(
                    children,
                    InlineStyle {
                        is_underlined: true,
                        ..style
                    },
                    context,
                ),
                InlineNode::Strikethrough(children) => self.push_nodes(
                    children,
                    InlineStyle {
                        is_struck: true,
                        ..style
                    },
                    context,
                ),
                InlineNode::Spoiler { id, children } => {
                    let start = self.text.len();
                    if context.revealed_spoilers.contains(id) || style.is_hidden {
                        let revealed = if style.is_hidden {
                            style
                        } else {
                            InlineStyle {
                                background: Some(color(
                                    tokens.component_message_spoiler_revealed_background,
                                )),
                                ..style
                            }
                        };
                        self.push_nodes(children, revealed, context);
                    } else {
                        let hidden_colour = color(tokens.component_message_spoiler_background);
                        self.push_nodes(
                            children,
                            InlineStyle {
                                background: Some(hidden_colour),
                                // The text keeps its room but cannot be read: drawn in the bar's colour.
                                colour: Some(hidden_colour),
                                is_hidden: true,
                                ..style
                            },
                            context,
                        );
                        let range = start..self.text.len();
                        self.actions
                            .push((range.clone(), InlineAction::RevealSpoiler(*id)));
                        self.tooltips
                            .push((range, context.strings.messages().spoiler_hidden().into()));
                    }
                }
                InlineNode::Link {
                    url,
                    is_masked,
                    children,
                } => {
                    let start = self.text.len();
                    let link = if style.is_hidden {
                        style
                    } else {
                        InlineStyle {
                            colour: Some(color(tokens.color_text_link)),
                            ..style
                        }
                    };
                    self.push_nodes(children, link, context);
                    let range = start..self.text.len();
                    if !style.is_hidden && !range.is_empty() {
                        self.actions
                            .push((range.clone(), InlineAction::OpenLink(url.clone())));
                        // The destination is shown wherever the visible text is not it.
                        if *is_masked {
                            self.tooltips.push((range, url.clone().into()));
                        }
                    }
                }
            }
        }
    }
}

/// How many monospace no-break spaces hold a picture `picture_em` ems wide.
fn picture_room_length(picture_em: f32) -> usize {
    // A small positive count of characters: the cast cannot truncate.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let length = (picture_em / MONOSPACE_ADVANCE_SHARE_OF_EM).ceil().max(1.0) as usize;
    length
}

/// Opens an address from a message in the OS browser, only if it is http or https.
pub(crate) fn open_message_link(url: &str, cx: &App) {
    if let Some(url) = safe_http_url(url) {
        cx.open_url(&url);
    }
}

/// The text as an element: GPUI's interactive text, plus the server emoji painted over their room.
pub(crate) struct RichTextElement {
    text: InteractiveText,
    layout: TextLayout,
    pictures: Vec<(Range<usize>, ImageSource)>,
    picture_side: f32,
    missing_border: Hsla,
}

impl RichTextElement {
    /// `picture_side` is the token size of a server emoji here (inline or jumbo).
    pub fn new(
        id: ElementId,
        rich_text: RichText,
        picture_side: f32,
        revealed_spoilers: Option<Entity<HashSet<u32>>>,
        tokens: &ThemeTokens,
        window: &Window,
    ) -> Self {
        let RichText {
            text,
            highlights,
            monospace,
            actions,
            tooltips,
            pictures,
        } = rich_text;
        let styled = StyledText::new(text)
            .with_highlights(highlights)
            .with_font_family_overrides(
                monospace
                    .into_iter()
                    .map(|range| (range, SharedString::new_static(MONOSPACE_FONT_FAMILY))),
            );
        let layout = styled.layout().clone();
        let (ranges, actions): (Vec<_>, Vec<_>) = actions.into_iter().unzip();
        let tooltips: Rc<[(Range<usize>, SharedString)]> = tooltips.into();
        let mut interactive = InteractiveText::new(id, styled);
        if !ranges.is_empty() {
            let actions: Arc<[InlineAction]> = actions.into();
            interactive =
                interactive.on_click(ranges, move |index, _, cx| match actions.get(index) {
                    Some(InlineAction::OpenLink(url)) => open_message_link(url, cx),
                    Some(InlineAction::RevealSpoiler(id)) => {
                        if let Some(revealed) = &revealed_spoilers {
                            revealed.update(cx, |revealed, cx| {
                                revealed.insert(*id);
                                cx.notify();
                            });
                        }
                    }
                    None => {}
                });
        }
        if !tooltips.is_empty() {
            interactive = interactive.tooltip(move |index, _, cx| {
                tooltips
                    .iter()
                    .find(|(range, _)| range.contains(&index))
                    .map(|(_, label)| Tooltip::view(label.clone(), cx))
            });
        }
        // Painted from frame 0 only: an animated emoji shows its still frame in text.
        let pictures = pictures
            .into_iter()
            .map(|picture| {
                let source = picture_for_box(
                    DownloadableFile::Icon(picture.emoji.id),
                    picture_side,
                    picture_side,
                    Animation::StillFrame,
                    window,
                );
                (picture.room, source)
            })
            .collect();
        Self {
            text: interactive,
            layout,
            pictures,
            picture_side,
            missing_border: color(tokens.component_custom_emoji_missing_border),
        }
    }

    fn paint_pictures(&self, window: &mut Window, cx: &mut App) {
        let side = length(self.picture_side).to_pixels(window.rem_size());
        let line_height = self.layout.line_height();
        for (room, source) in &self.pictures {
            let (Some(start), Some(end)) = (
                self.layout.position_for_index(room.start),
                self.layout.position_for_index(room.end),
            ) else {
                continue;
            };
            // The room never wraps; at a line's end its end may read as the next line's start.
            let room_width = if end.y == start.y {
                end.x - start.x
            } else {
                side
            };
            let bounds = Bounds::new(
                point(
                    start.x + (room_width - side) / 2.0,
                    start.y + (line_height - side) / 2.0,
                ),
                size(side, side),
            );
            let picture = match source {
                ImageSource::Custom(load) => load(window, cx),
                _ => None,
            };
            match picture {
                Some(Ok(picture)) => {
                    // Only fails for a window without an atlas, which draws nothing anyway.
                    let _ = window.paint_image(
                        bounds,
                        bounds,
                        gpui::Corners::default(),
                        picture,
                        0,
                        false,
                    );
                }
                Some(Err(_)) => window.paint_quad(
                    gpui::outline(bounds, self.missing_border, gpui::BorderStyle::Solid)
                        .corner_radii(
                            length(COMPONENT_CUSTOM_EMOJI_MISSING_RADIUS)
                                .to_pixels(window.rem_size()),
                        )
                        .border_widths(
                            length(COMPONENT_CUSTOM_EMOJI_MISSING_BORDER_WIDTH)
                                .to_pixels(window.rem_size()),
                        ),
                ),
                // Still loading: the room stays empty, as an image without its bytes yet.
                None => {}
            }
        }
    }
}

impl IntoElement for RichTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for RichTextElement {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        self.text.id()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn a11y_role(&self) -> Option<gpui::Role> {
        self.text.a11y_role()
    }

    fn write_a11y_info(&self, node: &mut gpui::accesskit::Node) {
        self.text.write_a11y_info(node);
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.text
            .prepaint(id, inspector_id, bounds, request_layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text
            .paint(id, inspector_id, bounds, request_layout, hitbox, window, cx);
        self.paint_pictures(window, cx);
    }
}

impl std::fmt::Debug for RichTextElement {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RichTextElement")
            .field("pictures", &self.pictures.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{Permissions, Role, Snowflake};

    use super::*;
    use crate::i18n::Language;
    use crate::theme::tokens::{
        COMPONENT_CUSTOM_EMOJI_INLINE_SIZE, COMPONENT_CUSTOM_EMOJI_JUMBO_SIZE, DARK_THEME,
        TYPE_MESSAGE,
    };

    fn context(revealed: &HashSet<u32>) -> RichTextContext<'_> {
        RichTextContext {
            tokens: &DARK_THEME,
            strings: Strings::new(Language::English),
            revealed_spoilers: revealed,
            picture_em: COMPONENT_CUSTOM_EMOJI_INLINE_SIZE / TYPE_MESSAGE.font_size,
        }
    }

    fn text(value: &str) -> InlineNode {
        InlineNode::Text(value.to_owned())
    }

    #[test]
    fn formatting_becomes_ordered_runs_over_one_text() {
        let revealed = HashSet::new();
        let rich = RichText::from_inlines(
            &[
                text("a "),
                InlineNode::Strong(vec![
                    text("bold "),
                    InlineNode::Emphasis(vec![text("both")]),
                ]),
                text(" "),
                InlineNode::Code("x()".into()),
            ],
            context(&revealed),
        );
        assert_eq!(rich.text, "a bold both x()");
        let ranges: Vec<_> = rich
            .highlights
            .iter()
            .map(|(range, _)| range.clone())
            .collect();
        assert_eq!(ranges, vec![2..7, 7..11, 12..15]);
        assert!(ranges.windows(2).all(|pair| pair[0].end <= pair[1].start));
        assert_eq!(rich.highlights[1].1.font_style, Some(FontStyle::Italic));
        assert_eq!(
            rich.highlights[1].1.font_weight,
            Some(FontWeight(f32::from(FONT_WEIGHT_BOLD)))
        );
        assert_eq!(rich.monospace, vec![12..15]);
    }

    #[test]
    fn a_hidden_spoiler_hides_its_text_and_deactivates_its_links_until_revealed() {
        let spoiler = [InlineNode::Spoiler {
            id: 0,
            children: vec![InlineNode::Link {
                url: "https://example.com/".into(),
                is_masked: false,
                children: vec![text("example.com")],
            }],
        }];
        let hidden = RichText::from_inlines(&spoiler, context(&HashSet::new()));
        assert_eq!(
            hidden.actions,
            vec![(0..11, InlineAction::RevealSpoiler(0))]
        );
        let bar = color(DARK_THEME.component_message_spoiler_background);
        assert_eq!(hidden.highlights[0].1.color, Some(bar));

        let revealed = HashSet::from([0]);
        let shown = RichText::from_inlines(&spoiler, context(&revealed));
        assert_eq!(
            shown.actions,
            vec![(0..11, InlineAction::OpenLink("https://example.com/".into()))]
        );
        assert_eq!(
            shown.highlights[0].1.color,
            Some(color(DARK_THEME.color_text_link))
        );
    }

    #[test]
    fn a_masked_link_shows_its_real_destination() {
        let rich = RichText::from_inlines(
            &[InlineNode::Link {
                url: "https://example.com/".into(),
                is_masked: true,
                children: vec![text("docs")],
            }],
            context(&HashSet::new()),
        );
        assert_eq!(rich.tooltips, vec![(0..4, "https://example.com/".into())]);
    }

    #[test]
    fn a_server_emoji_keeps_room_for_its_picture_and_is_named_by_its_tooltip() {
        let emoji = CustomEmoji {
            id: Snowflake::from_raw(500),
            name: "party".into(),
            is_animated: false,
        };
        let rich = RichText::from_inlines(
            &[text("hi "), InlineNode::CustomEmoji(emoji.clone())],
            context(&HashSet::new()),
        );
        let room = 3..rich.text.len();
        assert!(
            rich.text[room.clone()]
                .chars()
                .all(|character| character == PICTURE_ROOM_CHARACTER)
        );
        // The room is at least as wide as the picture, in the monospace font's advances.
        let room_characters =
            u16::try_from(rich.text[room.clone()].chars().count()).expect("a short room in a test");
        let room_em = f32::from(room_characters) * MONOSPACE_ADVANCE_SHARE_OF_EM;
        assert!(room_em >= COMPONENT_CUSTOM_EMOJI_INLINE_SIZE / TYPE_MESSAGE.font_size);
        assert_eq!(
            rich.pictures,
            vec![InlinePicture {
                room: room.clone(),
                emoji
            }]
        );
        assert_eq!(rich.monospace, vec![room.clone()]);
        assert_eq!(rich.tooltips, vec![(room, ":party:".into())]);
        assert!(
            picture_room_length(
                COMPONENT_CUSTOM_EMOJI_JUMBO_SIZE / COMPONENT_CUSTOM_EMOJI_JUMBO_SIZE
            ) >= 2
        );
    }

    #[test]
    fn role_mentions_are_chips_in_the_role_colour_or_plain_when_deleted() {
        let role = Role {
            id: Snowflake::from_raw(7),
            name: "Mods".into(),
            color: 0x004F_B39A,
            position: 1,
            permissions: Permissions::empty(),
            is_displayed_separately: false,
            is_mentionable: true,
            icon: None,
            unicode_emoji: None,
        };
        let rich = RichText::from_inlines(
            &[
                InlineNode::RoleMention(Some(role)),
                text(" "),
                InlineNode::RoleMention(None),
            ],
            context(&HashSet::new()),
        );
        assert!(
            rich.text
                .starts_with(&format!("{CHIP_PADDING}@Mods{CHIP_PADDING}"))
        );
        assert!(rich.text.ends_with("@deleted-role"));
        let chip = role_chip_colours(0x004F_B39A, DARK_THEME.color_surface_chat >> 8);
        assert_eq!(rich.highlights.len(), 1, "the deleted role is plain text");
        assert_eq!(
            rich.highlights[0].1.background_color,
            Some(opaque(chip.background))
        );
    }

    #[test]
    fn only_http_and_https_links_open() {
        assert_eq!(safe_http_url("javascript:alert(1)"), None);
        assert_eq!(safe_http_url("file:///etc/passwd"), None);
        assert!(safe_http_url("https://example.com").is_some());
    }
}

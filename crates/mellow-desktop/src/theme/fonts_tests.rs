//! GPUI's Linux text system as patched by ADR-0059 (`vendor/gpui-pre-wgpu`): system fonts read on a
//! background thread, emoji fonts in a fallback chain. Figtree is the app's font; stand-ins play
//! the system fonts, read only once the test sends `()`.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use cosmic_text::FontSystem;
use cosmic_text::fontdb::Database;
use gpui::{
    Font, FontId, FontRun, IndentAdjustment, LineFragment, LineLayout, PlatformTextSystem as _,
    font, px,
};
use gpui_wgpu::CosmicTextSystem;

use super::{EMBEDDED_FONT_FILES, MONOSPACE_FONT_FAMILY, SANS_FONT_FAMILY, interface_font};

const FIGTREE_REGULAR: &[u8] = EMBEDDED_FONT_FILES[0];
const FIGTREE_BOLD: &[u8] = EMBEDDED_FONT_FILES[3];
const SOURCE_CODE_PRO_REGULAR: &[u8] = EMBEDDED_FONT_FILES[8];
const EMOJI_FONT_SUBSET: &[u8] =
    include_bytes!("../../tests/fixtures/fonts/NotoColorEmoji-subset.ttf");
const MONOCHROME_TEXT_FONT_SUBSETS: [&[u8]; 2] = [
    include_bytes!("../../tests/fixtures/fonts/DejaVuSans-subset.ttf"),
    include_bytes!("../../tests/fixtures/fonts/DejaVuSans-Bold-subset.ttf"),
];
/// A box-drawing character: in Source Code Pro, not in Figtree.
const CHARACTER_ONLY_THE_SYSTEM_HAS: char = '\u{2500}';
const FONT_SIZE: f32 = 14.0;

/// The text system, with Figtree added, and what lets its system fonts be read.
fn text_system_whose_system_fonts_are(
    database: impl FnOnce() -> Database + Send + 'static,
) -> (Arc<CosmicTextSystem>, Sender<()>) {
    let (read_system_fonts, may_read) = channel::<()>();
    let text_system =
        CosmicTextSystem::new_reading_fonts_in_background(SANS_FONT_FAMILY, move || {
            may_read.recv().ok();
            FontSystem::new_with_locale_and_db("en-US".to_owned(), database())
        });
    text_system
        .add_fonts(vec![
            Cow::Borrowed(FIGTREE_REGULAR),
            Cow::Borrowed(FIGTREE_BOLD),
        ])
        .expect("Figtree loads");
    (Arc::new(text_system), read_system_fonts)
}

fn source_code_pro() -> Database {
    let mut database = Database::new();
    database.load_font_data(SOURCE_CODE_PRO_REGULAR.to_vec());
    database
}

/// The colour emoji font, and `DejaVu Sans` (regular and bold) with its monochrome 😀, ☺ and ©:
/// cosmic-text's own fallback tries `DejaVu Sans` first.
fn emoji_font_and_a_monochrome_text_font() -> Database {
    let mut database = Database::new();
    database.load_font_data(EMOJI_FONT_SUBSET.to_vec());
    for text_font in MONOCHROME_TEXT_FONT_SUBSETS {
        database.load_font_data(text_font.to_vec());
    }
    database
}

fn layout(text_system: &CosmicTextSystem, text: &str, font_id: FontId) -> LineLayout {
    text_system.layout_line(
        text,
        px(FONT_SIZE),
        &[FontRun {
            len: text.len(),
            font_id,
        }],
    )
}

/// The font of each glyph drawn.
fn glyph_fonts(text_system: &CosmicTextSystem, text: &str, font_id: FontId) -> Vec<FontId> {
    layout(text_system, text, font_id)
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter().map(|_| run.font_id))
        .collect()
}

/// Whether each glyph drawn comes from the colour emoji font.
fn emoji_glyphs(text_system: &CosmicTextSystem, text: &str, font: &Font) -> Vec<bool> {
    let font_id = text_system.font_id(font).expect("the font resolves");
    layout(text_system, text, font_id)
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.is_emoji))
        .collect()
}

#[test]
fn text_is_shaped_with_the_app_fonts_before_the_system_fonts_are_read() {
    let (text_system, _read_system_fonts) = text_system_whose_system_fonts_are(source_code_pro);
    // Would wait for ever if Figtree waited for the system fonts.
    let figtree = text_system
        .font_id(&font(SANS_FONT_FAMILY))
        .expect("Figtree");
    assert_eq!(text_system.all_font_names(), [SANS_FONT_FAMILY]);
    // The box-drawing glyph is left out, not drawn as a box.
    let text = format!("a{CHARACTER_ONLY_THE_SYSTEM_HAS}b");
    assert_eq!(
        glyph_fonts(&text_system, &text, figtree),
        [figtree, figtree]
    );
}

#[test]
fn once_the_system_fonts_join_old_font_ids_still_draw_and_lookups_give_new_ones() {
    let (text_system, read_system_fonts) = text_system_whose_system_fonts_are(source_code_pro);
    let figtree_before = text_system
        .font_id(&font(SANS_FONT_FAMILY))
        .expect("Figtree");

    read_system_fonts.send(()).expect("the reader waits");
    text_system.prewarm_fonts(&[figtree_before]);

    assert_eq!(
        text_system.all_font_names(),
        [SANS_FONT_FAMILY, MONOSPACE_FONT_FAMILY]
    );
    let figtree = text_system
        .font_id(&font(SANS_FONT_FAMILY))
        .expect("Figtree");
    assert_ne!(
        figtree, figtree_before,
        "GPUI's caches per font id must start again"
    );
    let source_code_pro = text_system
        .font_id(&font(MONOSPACE_FONT_FAMILY))
        .expect("the system font");
    let text = format!("a{CHARACTER_ONLY_THE_SYSTEM_HAS}b");
    assert_eq!(
        glyph_fonts(&text_system, &text, figtree),
        [figtree, source_code_pro, figtree]
    );
    assert_eq!(
        glyph_fonts(&text_system, &text, figtree_before),
        [figtree_before, source_code_pro, figtree_before],
        "a font id given before still draws"
    );
}

#[test]
fn character_widths_measured_before_the_system_fonts_are_measured_again() {
    let (platform, read_system_fonts) = text_system_whose_system_fonts_are(source_code_pro);
    let text_system = Arc::new(gpui::TextSystem::new(platform));
    let font_size = px(FONT_SIZE);
    let text = CHARACTER_ONLY_THE_SYSTEM_HAS.to_string().repeat(10);
    let wrap = |wrap_width| {
        let mut wrapper = text_system.line_wrapper(font(SANS_FONT_FAMILY), font_size);
        wrapper
            .wrap_line(
                &[LineFragment::text(&text)],
                wrap_width,
                IndentAdjustment::NoIndent,
            )
            .map(|boundary| boundary.ix)
            .collect::<Vec<_>>()
    };
    let width_before = text_system.layout_width(
        text_system.resolve_font(&font(SANS_FONT_FAMILY)),
        font_size,
        CHARACTER_ONLY_THE_SYSTEM_HAS,
    );
    // Remembers the width of the missing glyph in the wrapper's cache.
    wrap(px(1000.0));

    read_system_fonts.send(()).expect("the reader waits");
    text_system.prewarm_fonts(&[font(SANS_FONT_FAMILY)]);
    // What Mellow does once the system fonts are in (`redraw_with_system_fonts_once_read`).
    text_system.add_fonts(Vec::new()).expect("no font to add");

    let width = text_system.layout_width(
        text_system.resolve_font(&font(MONOSPACE_FONT_FAMILY)),
        font_size,
        CHARACTER_ONLY_THE_SYSTEM_HAS,
    );
    assert_ne!(width_before, width, "the test needs two different widths");
    let four_and_a_half = width * 4.5;
    let fourth_character_end = 4 * CHARACTER_ONLY_THE_SYSTEM_HAS.len_utf8();
    assert_eq!(wrap(four_and_a_half).first(), Some(&fourth_character_end));
}

#[test]
fn a_family_only_the_system_has_waits_for_the_system_fonts() {
    let (text_system, read_system_fonts) = text_system_whose_system_fonts_are(source_code_pro);
    // Read before the lookup, but joined only when a lookup misses, so it misses and waits.
    read_system_fonts.send(()).expect("the reader waits");
    assert!(text_system.font_id(&font(MONOSPACE_FONT_FAMILY)).is_ok());
}

#[test]
fn emoji_come_from_the_colour_emoji_font_of_the_fallback_chain() {
    let (text_system, read_system_fonts) =
        text_system_whose_system_fonts_are(emoji_font_and_a_monochrome_text_font);
    read_system_fonts.send(()).expect("the reader waits");
    text_system.prewarm_fonts(&[]);
    let interface = interface_font();

    assert_eq!(
        emoji_glyphs(&text_system, "a\u{1F600}b", &interface),
        [false, true, false]
    );
    assert_eq!(
        emoji_glyphs(&text_system, "a\u{1F600}b", &interface.clone().bold()),
        [false, true, false],
        "bold text too, though the emoji font has one weight"
    );
    assert!(
        emoji_glyphs(&text_system, "a\u{263A}b", &interface)
            .iter()
            .all(|is_emoji| !is_emoji),
        "☺ without U+FE0F is text, as in browsers"
    );
    let keycap = emoji_glyphs(&text_system, "a1\u{FE0F}\u{20E3}b", &interface);
    assert!(
        keycap.len() > 2 && keycap[1..keycap.len() - 1].iter().all(|is_emoji| *is_emoji),
        "a keycap starts with an ASCII digit Figtree has: {keycap:?}"
    );
    assert!(
        emoji_glyphs(&text_system, "a\u{A9}\u{FE0F}b", &interface)[1],
        "Figtree has ©, but ©️ asks for the emoji"
    );
}

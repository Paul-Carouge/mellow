//! The installed web app (DESIGN.md §4.5): its manifest, and the `theme-color` metas of
//! `web/index.html`, coloured from the tokens so the window's title bar is the ground it frames.

use std::fmt::Write as _;

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::json;

use crate::app_icons;
use crate::design_tokens::{DesignTokens, Theme};

/// The window's ground: the title bar and the splash screen continue it.
const GROUND_TOKEN: &str = "color.surface.rail";
const THEME_COLOR_START_MARKER: &str =
    "<!-- theme-color: generated from design/tokens.json by `cargo xtask tokens` -->";
const THEME_COLOR_END_MARKER: &str = "<!-- /theme-color -->";
const ICONS_URL_PATH: &str = "/icons";

/// The manifest accepts one colour per field; dark is Mellow's default theme (DESIGN.md §2), and
/// the page's `theme-color` metas take over once it loads, light included.
pub fn generate_manifest(tokens: &DesignTokens) -> Result<String> {
    let ground = ground_colour(tokens, Theme::Dark)?;
    let icon = |file_name: String, size: u32, purpose: &str| {
        json!({
            "src": format!("{ICONS_URL_PATH}/{file_name}"),
            "sizes": format!("{size}x{size}"),
            "type": "image/png",
            "purpose": purpose,
        })
    };
    let manifest = json!({
        "id": "/",
        "name": "Mellow",
        "short_name": "Mellow",
        "start_url": "/",
        "scope": "/",
        "display": "standalone",
        "background_color": ground,
        "theme_color": ground,
        "icons": [
            icon(app_icons::png_file_name(192), 192, "any"),
            icon(app_icons::png_file_name(512), 512, "any"),
            icon(
                app_icons::maskable_png_file_name(app_icons::WEB_MASKABLE_ICON_SIZE),
                app_icons::WEB_MASKABLE_ICON_SIZE,
                "maskable",
            ),
        ],
    });
    let mut text = serde_json::to_string_pretty(&manifest).context("cannot write the manifest")?;
    text.push('\n');
    Ok(text)
}

/// `index_html` with the lines between the theme-color markers replaced by one meta per scheme.
pub fn with_theme_color_metas(index_html: &str, tokens: &DesignTokens) -> Result<String> {
    let start = index_html
        .find(THEME_COLOR_START_MARKER)
        .ok_or_else(|| anyhow!("web/index.html has no `{THEME_COLOR_START_MARKER}` line"))?;
    let after_start = start + THEME_COLOR_START_MARKER.len();
    let Some(end) = index_html[after_start..]
        .find(THEME_COLOR_END_MARKER)
        .map(|offset| after_start + offset)
    else {
        bail!("web/index.html has no `{THEME_COLOR_END_MARKER}` after the theme-color marker");
    };
    // The generated lines take the indentation of the end marker's line.
    let indentation: String = index_html[..end]
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .take_while(|character| character.is_whitespace())
        .collect();
    let mut block = String::from("\n");
    for (scheme, theme) in [("dark", Theme::Dark), ("light", Theme::Light)] {
        writeln!(
            block,
            "{indentation}<meta name=\"theme-color\" media=\"(prefers-color-scheme: {scheme})\" content=\"{}\" />",
            ground_colour(tokens, theme)?
        )?;
    }
    block.push_str(&indentation);
    Ok(format!(
        "{}{block}{}",
        &index_html[..after_start],
        &index_html[end..]
    ))
}

fn ground_colour(tokens: &DesignTokens, theme: Theme) -> Result<String> {
    let value = tokens.resolved_value(GROUND_TOKEN, theme)?;
    let colour = value
        .as_str()
        .ok_or_else(|| anyhow!("`{GROUND_TOKEN}` is not a colour"))?;
    // A title bar cannot be translucent: `#RRGGBB` only.
    if colour.len() != "#RRGGBB".len() {
        bail!("`{GROUND_TOKEN}` must be an opaque `#RRGGBB` colour, found `{colour}`");
    }
    Ok(colour.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> DesignTokens {
        DesignTokens::parse(
            r##"{ "color": { "surface": { "$type": "color",
                "rail": { "$value": "#110B11", "$extensions": { "org.mellow.theme": { "light": "#F5F1F6" } } } } } }"##,
        )
        .unwrap()
    }

    #[test]
    fn the_manifest_is_installable_and_coloured_from_the_ground() {
        let manifest: serde_json::Value =
            serde_json::from_str(&generate_manifest(&tokens()).unwrap()).unwrap();
        assert_eq!(manifest["display"], "standalone");
        assert_eq!(manifest["start_url"], "/");
        assert_eq!(manifest["theme_color"], "#110B11");
        assert_eq!(manifest["background_color"], "#110B11");
        let icons = manifest["icons"].as_array().unwrap();
        assert!(icons.iter().any(|icon| icon["sizes"] == "192x192"));
        assert!(
            icons
                .iter()
                .any(|icon| icon["sizes"] == "512x512" && icon["purpose"] == "maskable")
        );
    }

    #[test]
    fn theme_color_metas_replace_only_the_marked_block() {
        let page = format!(
            "<head>\n    <title>Mellow</title>\n    {THEME_COLOR_START_MARKER}\n    old\n    {THEME_COLOR_END_MARKER}\n  </head>\n"
        );
        let updated = with_theme_color_metas(&page, &tokens()).unwrap();
        assert!(updated.contains("<title>Mellow</title>"));
        assert!(!updated.contains("old"));
        assert!(updated.contains(
            r##"    <meta name="theme-color" media="(prefers-color-scheme: dark)" content="#110B11" />"##
        ));
        assert!(updated.contains(r##"media="(prefers-color-scheme: light)" content="#F5F1F6""##));
        // Generating again changes nothing, so `--check` passes on a fresh output.
        assert_eq!(
            with_theme_color_metas(&updated, &tokens()).unwrap(),
            updated
        );
        assert!(with_theme_color_metas("<head></head>", &tokens()).is_err());
    }
}

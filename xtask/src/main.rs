//! `cargo xtask tokens [--check]`: generates platform token files, app icons and the web app
//! manifest from `design/tokens.json` and enforces the declared WCAG contrast pairs (DESIGN.md §3.5).

mod app_icons;
mod css_tokens;
mod design_tokens;
mod favicon;
mod slint_tokens;
mod web_manifest;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, anyhow, bail};

use crate::design_tokens::{DesignTokens, Rgba, Theme, contrast_ratio};

const USAGE: &str = "usage: cargo xtask tokens [--check]";
const TOKENS_SOURCE_PATH: &str = "design/tokens.json";
const SLINT_TOKENS_OUTPUT_PATH: &str = "crates/mellow-desktop/ui/tokens.slint";
const CSS_TOKENS_OUTPUT_PATH: &str = "web/src/lib/tokens.css";
const FAVICON_OUTPUT_PATH: &str = "web/public/favicon.svg";
const WEB_MANIFEST_OUTPUT_PATH: &str = "web/public/manifest.webmanifest";
/// Hand-written, except for its marked `theme-color` block.
const WEB_INDEX_HTML_PATH: &str = "web/index.html";

fn main() -> ExitCode {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: &[String]) -> Result<()> {
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match arguments.as_slice() {
        ["tokens"] => generate_tokens(false),
        ["tokens", "--check"] => generate_tokens(true),
        _ => bail!("{USAGE}"),
    }
}

fn workspace_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("xtask must live one level below the workspace root"))
}

fn generate_tokens(is_check_only: bool) -> Result<()> {
    let root = workspace_root()?;
    let source_path = root.join(TOKENS_SOURCE_PATH);
    let json_text = std::fs::read_to_string(&source_path)
        .with_context(|| format!("cannot read {}", source_path.display()))?;
    let tokens = DesignTokens::parse(&json_text)?;

    check_contrast_pairs(&tokens)?;

    let favicon_svg = favicon::generate_favicon(&tokens)?;
    let mut outputs = vec![
        (
            SLINT_TOKENS_OUTPUT_PATH.to_owned(),
            slint_tokens::generate_slint_tokens(&tokens)?.into_bytes(),
        ),
        (
            CSS_TOKENS_OUTPUT_PATH.to_owned(),
            css_tokens::generate_css_tokens(&tokens)?.into_bytes(),
        ),
        (
            FAVICON_OUTPUT_PATH.to_owned(),
            favicon_svg.clone().into_bytes(),
        ),
    ];
    outputs.push((
        WEB_MANIFEST_OUTPUT_PATH.to_owned(),
        web_manifest::generate_manifest(&tokens)?.into_bytes(),
    ));
    let index_html_path = root.join(WEB_INDEX_HTML_PATH);
    let index_html = std::fs::read_to_string(&index_html_path)
        .with_context(|| format!("cannot read {}", index_html_path.display()))?;
    outputs.push((
        WEB_INDEX_HTML_PATH.to_owned(),
        web_manifest::with_theme_color_metas(&index_html, &tokens)?.into_bytes(),
    ));
    let maskable_svg = favicon::generate_maskable_icon(&tokens)?;
    let icons_by_directory = [
        (
            app_icons::ICONS_DIRECTORY,
            app_icons::generate_app_icons(&favicon_svg)?,
        ),
        (
            app_icons::WEB_ICONS_DIRECTORY,
            app_icons::generate_web_app_icons(&favicon_svg, &maskable_svg)?,
        ),
    ];
    // (path, file name) of every icon, compared by pixels rather than bytes below.
    let mut icon_paths = Vec::new();
    for (directory, icons) in icons_by_directory {
        if !is_check_only {
            std::fs::create_dir_all(root.join(directory))?;
        }
        for (name, generated) in icons {
            let relative_path = format!("{directory}/{name}");
            icon_paths.push((relative_path.clone(), name));
            outputs.push((relative_path, generated));
        }
    }
    for (relative_path, generated) in outputs {
        let output_path = root.join(&relative_path);
        let current = std::fs::read(&output_path).unwrap_or_default();
        // A rasteriser's rounding differences alone never rewrite an icon (app_icons.rs).
        let is_up_to_date = match icon_paths.iter().find(|(path, _)| *path == relative_path) {
            Some((_, icon_file_name)) => {
                app_icons::is_icon_up_to_date(icon_file_name, &current, &generated)?
            }
            None => current == generated,
        };
        if is_up_to_date {
            continue;
        }
        if is_check_only {
            bail!(
                "{} is out of date; run `cargo xtask tokens`",
                output_path.display()
            );
        }
        std::fs::write(&output_path, generated)
            .with_context(|| format!("cannot write {}", output_path.display()))?;
        eprintln!("wrote {}", output_path.display());
    }
    if is_check_only {
        eprintln!("design tokens are up to date");
    }
    Ok(())
}

fn check_contrast_pairs(tokens: &DesignTokens) -> Result<()> {
    let mut failures = Vec::new();
    for pair in &tokens.contrast_pairs {
        for theme in [Theme::Dark, Theme::Light] {
            let colour = |path: &str| -> Result<Rgba> {
                let value = tokens.resolved_value(path, theme)?;
                Rgba::parse_hex(
                    value
                        .as_str()
                        .ok_or_else(|| anyhow!("`{path}` is not a colour"))?,
                )
            };
            let background = colour(&pair.background_path)?;
            if background.alpha != u8::MAX {
                bail!(
                    "contrast pair background `{}` is translucent in {theme:?}; pair it with an opaque surface",
                    pair.background_path
                );
            }
            let ratio = contrast_ratio(colour(&pair.foreground_path)?, background);
            if ratio < pair.minimum_ratio {
                failures.push(format!(
                    "{theme:?}: {} on {} is {ratio:.2}:1, needs {}:1",
                    pair.foreground_path, pair.background_path, pair.minimum_ratio
                ));
            }
        }
    }
    if !failures.is_empty() {
        bail!(
            "contrast pairs below WCAG minimum:\n{}",
            failures.join("\n")
        );
    }
    eprintln!(
        "{} contrast pairs pass in both themes",
        tokens.contrast_pairs.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_with_pair(foreground: &str, background: &str, minimum: f64) -> DesignTokens {
        DesignTokens::parse(&format!(
            r#"{{
                "color": {{ "$type": "color",
                    "foreground": {{ "$value": "{foreground}" }},
                    "background": {{ "$value": "{background}" }} }},
                "$extensions": {{ "org.mellow.contrast-pairs": [
                    {{ "foreground": "color.foreground", "background": "color.background", "minimum": {minimum} }}
                ] }}
            }}"#
        ))
        .unwrap()
    }

    #[test]
    fn passing_pairs_are_accepted() {
        assert!(check_contrast_pairs(&tokens_with_pair("#FFFFFF", "#000000", 4.5)).is_ok());
    }

    #[test]
    fn pairs_below_their_minimum_fail_with_the_measured_ratio() {
        let error = check_contrast_pairs(&tokens_with_pair("#777777", "#888888", 4.5))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("color.foreground on color.background"),
            "{error}"
        );
    }

    #[test]
    fn translucent_backgrounds_are_rejected() {
        let error = check_contrast_pairs(&tokens_with_pair("#FFFFFF", "#00000080", 4.5))
            .unwrap_err()
            .to_string();
        assert!(error.contains("translucent"), "{error}");
    }
}

//! Reads `design/tokens.json` (W3C Design Tokens format with Mellow extensions) and resolves
//! aliases per theme. See DESIGN.md §3.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use serde_json::Value;

const THEME_EXTENSION: &str = "org.mellow.theme";
const CONTRAST_PAIRS_EXTENSION: &str = "org.mellow.contrast-pairs";
/// Guards against alias cycles such as `{a}` → `{b}` → `{a}`.
const MAXIMUM_ALIAS_DEPTH: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DesignToken {
    pub token_type: String,
    pub dark_value: Value,
    /// Present only when the light theme differs from the dark one.
    pub light_value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContrastPair {
    pub foreground_path: String,
    pub background_path: String,
    pub minimum_ratio: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DesignTokens {
    /// Keyed by dotted path, e.g. `color.surface.chat`; sorted so generated output is stable.
    pub tokens_by_path: BTreeMap<String, DesignToken>,
    pub contrast_pairs: Vec<ContrastPair>,
}

impl DesignTokens {
    pub fn parse(json_text: &str) -> Result<Self> {
        let root: Value =
            serde_json::from_str(json_text).context("design tokens are not valid JSON")?;
        let mut tokens_by_path = BTreeMap::new();
        collect_tokens(&root, &mut Vec::new(), None, &mut tokens_by_path)?;
        let contrast_pairs = root
            .pointer(&format!("/$extensions/{CONTRAST_PAIRS_EXTENSION}"))
            .and_then(Value::as_array)
            .map(|pairs| {
                pairs
                    .iter()
                    .map(parse_contrast_pair)
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            tokens_by_path,
            contrast_pairs,
        })
    }

    /// Value of `path` in `theme`, with every `{alias}` replaced by its resolved value.
    pub fn resolved_value(&self, path: &str, theme: Theme) -> Result<Value> {
        self.resolve_path(path, theme, 0)
    }

    fn resolve_path(&self, path: &str, theme: Theme, depth: usize) -> Result<Value> {
        ensure!(
            depth < MAXIMUM_ALIAS_DEPTH,
            "alias chain too deep (cycle?) at `{path}`"
        );
        let token = self
            .tokens_by_path
            .get(path)
            .ok_or_else(|| anyhow!("unknown token `{path}`"))?;
        let value = match (theme, &token.light_value) {
            (Theme::Light, Some(light_value)) => light_value,
            _ => &token.dark_value,
        };
        self.resolve_value(value, theme, depth + 1)
            .with_context(|| format!("while resolving `{path}`"))
    }

    fn resolve_value(&self, value: &Value, theme: Theme, depth: usize) -> Result<Value> {
        match value {
            Value::String(text) => match alias_target(text) {
                Some(target_path) => self.resolve_path(target_path, theme, depth),
                None => Ok(value.clone()),
            },
            Value::Object(fields) => fields
                .iter()
                .map(|(name, field_value)| {
                    Ok((name.clone(), self.resolve_value(field_value, theme, depth)?))
                })
                .collect::<Result<serde_json::Map<_, _>>>()
                .map(Value::Object),
            _ => Ok(value.clone()),
        }
    }
}

fn alias_target(text: &str) -> Option<&str> {
    text.strip_prefix('{')?.strip_suffix('}')
}

fn collect_tokens(
    node: &Value,
    path: &mut Vec<String>,
    inherited_type: Option<&str>,
    tokens_by_path: &mut BTreeMap<String, DesignToken>,
) -> Result<()> {
    let Value::Object(fields) = node else {
        return Ok(());
    };
    let token_type = fields
        .get("$type")
        .and_then(Value::as_str)
        .or(inherited_type);
    if let Some(dark_value) = fields.get("$value") {
        let dotted_path = path.join(".");
        let token_type = token_type.ok_or_else(|| anyhow!("token `{dotted_path}` has no $type"))?;
        let light_value = fields
            .get("$extensions")
            .and_then(|extensions| extensions.get(THEME_EXTENSION))
            .and_then(|theme| theme.get("light"))
            .cloned();
        tokens_by_path.insert(
            dotted_path,
            DesignToken {
                token_type: token_type.to_owned(),
                dark_value: dark_value.clone(),
                light_value,
            },
        );
        return Ok(());
    }
    for (name, child) in fields.iter().filter(|(name, _)| !name.starts_with('$')) {
        path.push(name.clone());
        collect_tokens(child, path, token_type, tokens_by_path)?;
        path.pop();
    }
    Ok(())
}

fn parse_contrast_pair(pair: &Value) -> Result<ContrastPair> {
    let field = |name: &str| {
        pair.get(name)
            .ok_or_else(|| anyhow!("contrast pair is missing `{name}`: {pair}"))
    };
    Ok(ContrastPair {
        foreground_path: field("foreground")?
            .as_str()
            .ok_or_else(|| anyhow!("`foreground` must be a token path"))?
            .to_owned(),
        background_path: field("background")?
            .as_str()
            .ok_or_else(|| anyhow!("`background` must be a token path"))?
            .to_owned(),
        minimum_ratio: field("minimum")?
            .as_f64()
            .ok_or_else(|| anyhow!("`minimum` must be a number"))?,
    })
}

/// sRGB colour with alpha, each channel in 0..=255.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl Rgba {
    pub fn parse_hex(text: &str) -> Result<Self> {
        let digits = text
            .strip_prefix('#')
            .ok_or_else(|| anyhow!("colour `{text}` must start with #"))?;
        let channel = |index: usize| {
            digits
                .get(index * 2..index * 2 + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| anyhow!("colour `{text}` is not #RRGGBB or #RRGGBBAA"))
        };
        match digits.len() {
            6 => Ok(Self {
                red: channel(0)?,
                green: channel(1)?,
                blue: channel(2)?,
                alpha: u8::MAX,
            }),
            8 => Ok(Self {
                red: channel(0)?,
                green: channel(1)?,
                blue: channel(2)?,
                alpha: channel(3)?,
            }),
            _ => bail!("colour `{text}` is not #RRGGBB or #RRGGBBAA"),
        }
    }

    /// Source-over compositing of `self` on an opaque `background`.
    fn composited_over(self, background: Self) -> Self {
        let alpha = f64::from(self.alpha) / f64::from(u8::MAX);
        let blend = |foreground: u8, background: u8| {
            // Rounded blend of two 0..=255 values stays within 0..=255.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let blended = (f64::from(foreground) * alpha + f64::from(background) * (1.0 - alpha))
                .round() as u8;
            blended
        };
        Self {
            red: blend(self.red, background.red),
            green: blend(self.green, background.green),
            blue: blend(self.blue, background.blue),
            alpha: u8::MAX,
        }
    }

    fn relative_luminance(self) -> f64 {
        let linear = |channel: u8| {
            let srgb = f64::from(channel) / f64::from(u8::MAX);
            if srgb <= 0.040_45 {
                srgb / 12.92
            } else {
                ((srgb + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(self.red) + 0.7152 * linear(self.green) + 0.0722 * linear(self.blue)
    }
}

/// WCAG 2.x contrast ratio; a translucent foreground is first composited over the background.
pub fn contrast_ratio(foreground: Rgba, opaque_background: Rgba) -> f64 {
    let foreground_luminance = foreground
        .composited_over(opaque_background)
        .relative_luminance();
    let background_luminance = opaque_background.relative_luminance();
    let (lighter, darker) = if foreground_luminance > background_luminance {
        (foreground_luminance, background_luminance)
    } else {
        (background_luminance, foreground_luminance)
    };
    (lighter + 0.05) / (darker + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TOKENS: &str = r##"{
        "ref": { "$type": "color", "black": { "$value": "#000000" }, "white": { "$value": "#FFFFFF" } },
        "color": {
            "$type": "color",
            "text": { "$value": "{ref.white}", "$extensions": { "org.mellow.theme": { "light": "{ref.black}" } } },
            "surface": { "$value": "{ref.black}", "$extensions": { "org.mellow.theme": { "light": "{ref.white}" } } }
        },
        "component": { "label": { "color": { "$type": "color", "$value": "{color.text}" } } },
        "$extensions": { "org.mellow.contrast-pairs": [
            { "foreground": "color.text", "background": "color.surface", "minimum": 4.5 }
        ] }
    }"##;

    #[test]
    fn aliases_resolve_through_semantic_tokens_in_each_theme() {
        let tokens = DesignTokens::parse(SAMPLE_TOKENS).unwrap();
        assert_eq!(
            tokens
                .resolved_value("component.label.color", Theme::Dark)
                .unwrap(),
            "#FFFFFF"
        );
        assert_eq!(
            tokens
                .resolved_value("component.label.color", Theme::Light)
                .unwrap(),
            "#000000"
        );
        assert_eq!(tokens.contrast_pairs.len(), 1);
    }

    #[test]
    fn alias_cycles_and_unknown_aliases_are_errors() {
        let cyclic = r#"{ "a": { "$type": "color", "$value": "{b}" }, "b": { "$type": "color", "$value": "{a}" } }"#;
        assert!(
            DesignTokens::parse(cyclic)
                .unwrap()
                .resolved_value("a", Theme::Dark)
                .is_err()
        );
        let dangling = r#"{ "a": { "$type": "color", "$value": "{missing}" } }"#;
        assert!(
            DesignTokens::parse(dangling)
                .unwrap()
                .resolved_value("a", Theme::Dark)
                .is_err()
        );
    }

    #[test]
    fn contrast_ratio_matches_wcag_reference_values() {
        let black = Rgba::parse_hex("#000000").unwrap();
        let white = Rgba::parse_hex("#FFFFFF").unwrap();
        assert!((contrast_ratio(white, black) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(black, black) - 1.0).abs() < 1e-9);
        let transparent_white = Rgba::parse_hex("#FFFFFF00").unwrap();
        assert!((contrast_ratio(transparent_white, black) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn malformed_colours_are_rejected() {
        for text in ["FFFFFF", "#FFF", "#GGGGGG", "#FFFFFFF"] {
            assert!(Rgba::parse_hex(text).is_err(), "{text}");
        }
    }
}

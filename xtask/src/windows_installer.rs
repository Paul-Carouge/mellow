//! The Windows installer's folder (ADR-0060): what Inno Setup compiles into
//! `mellow-desktop-<version>-x86_64-pc-windows-msvc-setup.exe`. The script is hand-written
//! (`packaging/windows/mellow.iss`); the version, the colours and the wizard pictures are generated
//! here from the workspace version and `design/tokens.json`, so the installer follows the tokens
//! like every other icon and no picture is made by hand.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, anyhow, bail};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

use crate::app_icons::render_png;
use crate::design_tokens::{DesignTokens, Rgba, Theme};
use crate::desktop_package::{PACKAGING_DIRECTORY, VERSION, copy_file, read_favicon, write_file};
use crate::favicon::{MELLI_DRAWING, fill_mascot_colours, rgb_hex};

const SCRIPT_FILE_NAME: &str = "mellow.iss";
const GENERATED_DEFINITIONS_FILE_NAME: &str = "generated.iss";
const ICON_FILE_NAME: &str = "mellow.ico";
const IMAGES_DIRECTORY: &str = "images";
/// The wizard's background in the light and dark appearances: the conversation's surface.
const BACKGROUND_TOKEN: &str = "color.surface.chat";
/// Inno Setup's welcome picture area at 100 % to 250 % scaling, for its modern style and default
/// font (its `WizardImageFile` help topic); it picks the closest one.
const WIZARD_IMAGE_SIZES: [(u32, u32); 7] = [
    (202, 386),
    (269, 515),
    (336, 643),
    (403, 772),
    (430, 824),
    (498, 953),
    (534, 1022),
];
/// The top-right square on the progress page, at the same scalings.
const WIZARD_SMALL_IMAGE_SIZES: [u32; 7] = [58, 77, 97, 116, 124, 143, 159];

/// The welcome page's picture, in Inno Setup's 164 × 314 proportions: Melli under a dusk sky
/// with a few stars, the apricot glow of the favicon rising from the bottom.
const WIZARD_PICTURE_TEMPLATE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 164 314">
  <defs>
    <linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{tile-top}"/><stop offset="1" stop-color="{tile-bottom}"/>
    </linearGradient>
    <radialGradient id="glow" cx="0.5" cy="0.62" r="0.7">
      <stop offset="0" stop-color="{glow}" stop-opacity="{glow-opacity}"/><stop offset="1" stop-color="{glow}" stop-opacity="0"/>
    </radialGradient>
  </defs>
  <rect width="164" height="314" fill="url(#sky)"/>
  <rect width="164" height="314" fill="url(#glow)"/>
  <g fill="{body}" fill-opacity="0.6">
    <circle cx="24" cy="34" r="1.2"/><circle cx="58" cy="20" r="0.8"/><circle cx="121" cy="42" r="1.4"/><circle cx="143" cy="80" r="0.9"/><circle cx="36" cy="94" r="0.8"/><circle cx="97" cy="66" r="1"/><circle cx="18" cy="142" r="1.1"/><circle cx="146" cy="134" r="0.8"/>
  </g>
  <g transform="translate(82 190) scale(0.56) translate(-128 -124)">
{melli}  </g>
</svg>
"#;
const WIZARD_PICTURE_WIDTH: f32 = 164.0;
const WIZARD_PICTURE_HEIGHT: f32 = 314.0;

/// The installer's file name without `.exe`, as the release publishes it.
#[must_use]
pub fn installer_base_name(version: &str) -> String {
    format!("mellow-desktop-{version}-x86_64-pc-windows-msvc-setup")
}

fn wizard_image_name(width: u32, height: u32) -> String {
    format!("{IMAGES_DIRECTORY}\\wizard-{width}x{height}.png")
}

fn wizard_small_image_name(size: u32) -> String {
    format!("{IMAGES_DIRECTORY}\\small-{size}.png")
}

/// The welcome picture's SVG, coloured from the mascot tokens.
pub fn wizard_picture_svg(tokens: &DesignTokens) -> Result<String> {
    fill_mascot_colours(
        &WIZARD_PICTURE_TEMPLATE.replace("{melli}", MELLI_DRAWING),
        tokens,
    )
}

/// `picture` drawn to fill `width` × `height` (the sizes keep its proportions within a pixel).
fn render_wizard_picture(picture: &usvg::Tree, width: u32, height: u32) -> Result<Vec<u8>> {
    let mut pixmap =
        Pixmap::new(width, height).ok_or_else(|| anyhow!("empty wizard picture size"))?;
    // Picture sizes are small integers: the f32 conversions are exact.
    #[allow(clippy::cast_precision_loss)]
    let (horizontal, vertical) = (
        width as f32 / WIZARD_PICTURE_WIDTH,
        height as f32 / WIZARD_PICTURE_HEIGHT,
    );
    resvg::render(
        picture,
        Transform::from_scale(horizontal, vertical),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .with_context(|| format!("cannot encode the {width} × {height} wizard picture"))
}

fn theme_colour(tokens: &DesignTokens, path: &str, theme: Theme) -> Result<String> {
    let value = tokens.resolved_value(path, theme)?;
    let colour = Rgba::parse_hex(
        value
            .as_str()
            .ok_or_else(|| anyhow!("`{path}` is not a colour"))?,
    )?;
    if colour.alpha != u8::MAX {
        bail!("`{path}` must be opaque to be the installer's background");
    }
    Ok(rgb_hex(colour))
}

/// `generated.iss`: Inno Setup preprocessor definitions the script includes.
pub fn generated_definitions(tokens: &DesignTokens, version: &str) -> Result<String> {
    let wizard_images: Vec<String> = WIZARD_IMAGE_SIZES
        .iter()
        .map(|(width, height)| wizard_image_name(*width, *height))
        .collect();
    let small_images: Vec<String> = WIZARD_SMALL_IMAGE_SIZES
        .iter()
        .map(|size| wizard_small_image_name(*size))
        .collect();
    let mut definitions = String::from(
        "; Generated by `cargo xtask windows-package` from the workspace version and design/tokens.json.\n",
    );
    for (name, value) in [
        ("MellowVersion", version.to_owned()),
        ("OutputBaseName", installer_base_name(version)),
        (
            "BackColorLight",
            theme_colour(tokens, BACKGROUND_TOKEN, Theme::Light)?,
        ),
        (
            "BackColorDark",
            theme_colour(tokens, BACKGROUND_TOKEN, Theme::Dark)?,
        ),
        ("WizardImages", wizard_images.join(",")),
        ("WizardSmallImages", small_images.join(",")),
    ] {
        if value.contains('"') {
            bail!("{name} cannot hold a quote");
        }
        writeln!(definitions, "#define {name} \"{value}\"")?;
    }
    Ok(definitions)
}

/// Lays out in `output_directory` what Inno Setup compiles: `mellow.exe` (from `binary`), the
/// licence, the icon, the script, its generated definitions and the wizard pictures.
pub fn build_windows_installer_folder(
    root: &Path,
    binary: &Path,
    output_directory: &Path,
) -> Result<()> {
    if output_directory.exists() {
        bail!(
            "{} already exists; the installer folder starts empty",
            output_directory.display()
        );
    }
    let packaging = root.join(PACKAGING_DIRECTORY).join("windows");
    let tokens_path = root.join(crate::TOKENS_SOURCE_PATH);
    let tokens = DesignTokens::parse(
        &std::fs::read_to_string(&tokens_path)
            .with_context(|| format!("cannot read {}", tokens_path.display()))?,
    )?;
    copy_file(binary, &output_directory.join("mellow.exe"))?;
    copy_file(&root.join("LICENSE"), &output_directory.join("LICENSE.txt"))?;
    copy_file(
        &packaging.join(ICON_FILE_NAME),
        &output_directory.join(ICON_FILE_NAME),
    )?;
    copy_file(
        &packaging.join(SCRIPT_FILE_NAME),
        &output_directory.join(SCRIPT_FILE_NAME),
    )?;
    write_file(
        &output_directory.join(GENERATED_DEFINITIONS_FILE_NAME),
        generated_definitions(&tokens, VERSION)?,
    )?;
    let picture = usvg::Tree::from_str(&wizard_picture_svg(&tokens)?, &usvg::Options::default())
        .context("the wizard picture is not valid SVG")?;
    // Inno Setup names them with backslashes; on disk they are files of `images/`.
    let on_disk = |name: String| output_directory.join(name.replace('\\', "/"));
    for (width, height) in WIZARD_IMAGE_SIZES {
        write_file(
            &on_disk(wizard_image_name(width, height)),
            render_wizard_picture(&picture, width, height)?,
        )?;
    }
    let (_, favicon) = read_favicon(root)?;
    for size in WIZARD_SMALL_IMAGE_SIZES {
        write_file(
            &on_disk(wizard_small_image_name(size)),
            render_png(&favicon, size)?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> std::path::PathBuf {
        crate::workspace_root().unwrap()
    }

    fn workspace_tokens() -> DesignTokens {
        DesignTokens::parse(
            &std::fs::read_to_string(workspace_root().join(crate::TOKENS_SOURCE_PATH)).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn the_definitions_carry_the_version_the_background_and_every_picture() {
        let definitions = generated_definitions(&workspace_tokens(), "1.2.3").unwrap();
        assert!(definitions.contains("#define MellowVersion \"1.2.3\"\n"));
        assert!(definitions.contains(
            "#define OutputBaseName \"mellow-desktop-1.2.3-x86_64-pc-windows-msvc-setup\"\n"
        ));
        for name in ["BackColorLight", "BackColorDark"] {
            let line = definitions
                .lines()
                .find(|line| line.starts_with(&format!("#define {name} ")))
                .unwrap();
            assert!(line.ends_with('"') && line.contains("\"#"), "{line}");
        }
        assert!(definitions.contains("images\\wizard-202x386.png,images\\wizard-269x515.png"));
        assert!(definitions.contains("images\\small-58.png,"));
    }

    #[test]
    fn the_script_uses_every_definition_and_installs_per_user_without_questions() {
        let script = std::fs::read_to_string(
            workspace_root()
                .join(PACKAGING_DIRECTORY)
                .join("windows")
                .join(SCRIPT_FILE_NAME),
        )
        .unwrap();
        // Inno Setup reads a UTF-8 script by its byte order mark (the French strings).
        assert!(script.starts_with('\u{feff}'));
        assert!(script.contains(&format!("#include \"{GENERATED_DEFINITIONS_FILE_NAME}\"")));
        for name in [
            "MellowVersion",
            "OutputBaseName",
            "BackColorLight",
            "BackColorDark",
            "WizardImages",
            "WizardSmallImages",
        ] {
            assert!(script.contains(&format!("{{#{name}}}")), "{name} is unused");
        }
        for line in [
            "PrivilegesRequired=lowest",
            "DefaultDirName={autopf}\\Mellow",
            "DisableDirPage=yes",
            "DisableProgramGroupPage=yes",
            "DisableReadyPage=yes",
            "DisableFinishedPage=yes",
            "ShowLanguageDialog=no",
            "CloseApplications=no",
            "SetupIconFile=mellow.ico",
        ] {
            assert!(script.lines().any(|candidate| candidate == line), "{line}");
        }
        // The notifications' identifier, which the Start menu entry must carry too.
        let desktop_lib =
            std::fs::read_to_string(workspace_root().join("crates/mellow-desktop/src/lib.rs"))
                .unwrap();
        assert!(desktop_lib.contains(r#"APP_IDENTIFIER: &str = "org.mellow.desktop";"#));
        assert!(script.contains("AppUserModelID: \"org.mellow.desktop\""));
        // The self-update's switches (crates/mellow-desktop/src/self_update/package.rs).
        assert!(script.contains("HasSwitch('/UPDATE')"));
        assert!(script.contains("HasSwitch('/LAUNCH')"));
    }

    #[test]
    fn the_wizard_picture_is_opaque_with_melli_in_the_middle() {
        let tokens = workspace_tokens();
        let picture = usvg::Tree::from_str(
            &wizard_picture_svg(&tokens).unwrap(),
            &usvg::Options::default(),
        )
        .unwrap();
        let png = render_wizard_picture(&picture, 202, 386).unwrap();
        let pixmap = Pixmap::decode_png(&png).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (202, 386));
        assert!(pixmap.pixels().iter().all(|pixel| pixel.alpha() == u8::MAX));
        let body = Rgba::parse_hex(
            tokens
                .resolved_value("color.mascot.body", Theme::Light)
                .unwrap()
                .as_str()
                .unwrap(),
        )
        .unwrap();
        // Between Melli's eyes, at (82, 190) of the 164 × 314 drawing.
        let centre = pixmap.pixel(101, 233).unwrap().demultiply();
        assert_eq!(
            (centre.red(), centre.green(), centre.blue()),
            (body.red, body.green, body.blue)
        );
    }

    #[test]
    fn the_installer_folder_holds_what_the_script_names() {
        let scratch = tempfile::tempdir().unwrap();
        let binary = scratch.path().join("mellow.exe");
        std::fs::write(&binary, b"MZ").unwrap();
        let folder = scratch.path().join("installer");
        build_windows_installer_folder(&workspace_root(), &binary, &folder).unwrap();
        for relative in [
            "mellow.exe",
            "LICENSE.txt",
            "mellow.ico",
            "mellow.iss",
            "generated.iss",
            "images/wizard-202x386.png",
            "images/wizard-534x1022.png",
            "images/small-58.png",
            "images/small-159.png",
        ] {
            assert!(folder.join(relative).is_file(), "{relative} is missing");
        }
        let small = Pixmap::decode_png(&std::fs::read(folder.join("images/small-97.png")).unwrap())
            .unwrap();
        assert_eq!(small.width(), 97);
        assert!(build_windows_installer_folder(&workspace_root(), &binary, &folder).is_err());
    }
}

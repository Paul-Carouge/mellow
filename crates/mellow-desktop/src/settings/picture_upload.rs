//! A picture picked for an avatar, a banner or an icon: read off the UI thread, never more than
//! the instance takes, and its refusals said in the settings' own words.

use std::io::Read as _;
use std::path::Path;

use gpui::App;
use mellow_client_core::ClientError;
use mellow_client_core::shared_logic::user_settings::{
    MAXIMUM_PROFILE_IMAGE_BYTES, ProfileImageFileProblem, profile_image_file_problem,
};
use mellow_protocol::ApiErrorCode;

use crate::i18n::strings;
use crate::messages::file_size_label;
use crate::settings::picture_crop::{MAXIMUM_PICTURE_MEGAPIXELS, MAXIMUM_PICTURE_SIDE_PIXELS};
use crate::sign_in_form::describe_error;
use crate::strings_pending_i18n::strings as pending_strings;

/// Why a picked file was not sent.
#[derive(Debug)]
pub enum PictureReadProblem {
    File(ProfileImageFileProblem),
    Unreadable,
    /// More pixels than the instance takes.
    TooManyPixels,
}

/// Reads a picked file, never more than the instance takes. Blocking: call it off the UI thread.
pub fn read_picture(path: &Path) -> Result<Vec<u8>, PictureReadProblem> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let size = std::fs::metadata(path)
        .map_err(|_| PictureReadProblem::Unreadable)?
        .len();
    if let Some(problem) = profile_image_file_problem(&file_name, size) {
        return Err(PictureReadProblem::File(problem));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| {
            file.take(MAXIMUM_PROFILE_IMAGE_BYTES + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| PictureReadProblem::Unreadable)?;
    // The file grew since its size was read.
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAXIMUM_PROFILE_IMAGE_BYTES {
        return Err(PictureReadProblem::File(ProfileImageFileProblem::TooLarge));
    }
    Ok(bytes)
}

/// "Up to 8 MB.", the reason a file too large gives.
pub fn too_large_reason(cx: &App) -> String {
    let catalogue = strings(cx);
    catalogue
        .profile()
        .too_large(&file_size_label(MAXIMUM_PROFILE_IMAGE_BYTES, catalogue))
}

/// The sentence for a picked file that was not sent: `summary`, then why.
pub fn describe_read_problem(problem: &PictureReadProblem, summary: &str, cx: &App) -> String {
    let catalogue = strings(cx);
    let reason = match problem {
        PictureReadProblem::File(ProfileImageFileProblem::NotAPicture) => {
            catalogue.profile().not_a_picture().to_owned()
        }
        PictureReadProblem::File(ProfileImageFileProblem::TooLarge) => too_large_reason(cx),
        PictureReadProblem::Unreadable => catalogue.profile().unreadable().to_owned(),
        PictureReadProblem::TooManyPixels => (pending_strings(cx).picture_too_many_pixels)(
            MAXIMUM_PICTURE_MEGAPIXELS,
            MAXIMUM_PICTURE_SIDE_PIXELS,
        ),
    };
    (pending_strings(cx).error_with_reason)(summary, &reason)
}

/// Says what to do about each refusal, in the settings' words rather than the generic ones.
pub fn describe_image_failure(error: &ClientError, summary: &str, cx: &App) -> String {
    let pending = pending_strings(cx);
    match error {
        ClientError::Api {
            code: ApiErrorCode::ValidationFailed,
            ..
        } => (pending.error_with_reason)(summary, strings(cx).profile().unreadable()),
        ClientError::FileTooLarge => (pending.error_with_reason)(summary, &too_large_reason(cx)),
        _ => describe_error(error, summary, pending),
    }
}

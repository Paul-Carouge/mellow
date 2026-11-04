//! GIFs from Klipy, searched by members' browsers (ADR-0042).

use serde::{Deserialize, Serialize};

/// A member keeps at most this many starred GIFs.
pub const MAXIMUM_GIF_FAVOURITES: u32 = 200;

/// Answer of `GET /api/v1/gifs/settings` (`404` when the instance has no GIFs): what the browser
/// needs to call `https://api.klipy.com/api/v1/{klipy_app_key}/gifs/…` itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GifSettings {
    pub klipy_app_key: String,
    /// Opaque and stable per member and instance; never the member's id.
    pub customer_id: String,
}

use mellow_protocol::ApiErrorCode;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("invalid instance address: {0}")]
    InvalidInstanceAddress(String),
    /// The server answered with an error; `code` is stable, `message` is for humans.
    #[error("{message}")]
    Api {
        status: u16,
        code: ApiErrorCode,
        message: String,
    },
    #[error("cannot reach the instance")]
    Network(#[source] reqwest::Error),
    #[error("unexpected answer from the instance")]
    UnexpectedResponse(String),
    #[error("not signed in")]
    NotSignedIn,
}

impl ClientError {
    /// True when signing in again is the only way forward (expired or revoked session).
    #[must_use]
    pub fn needs_new_login(&self) -> bool {
        matches!(
            self,
            Self::NotSignedIn
                | Self::Api {
                    code: ApiErrorCode::Unauthorized | ApiErrorCode::AccountDisabled,
                    ..
                }
        )
    }
}

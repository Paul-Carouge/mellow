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
    #[error("the system random generator is unavailable")]
    RandomGeneratorFailed(#[source] getrandom::Error),
}

impl ClientError {
    /// True when the instance answered a send by refusing to store the message: sending it again
    /// unchanged cannot succeed. Other failures may hide a send that did reach the server.
    #[must_use]
    pub fn refuses_message_for_good(&self) -> bool {
        matches!(
            self,
            Self::Api {
                code: ApiErrorCode::ValidationFailed
                    | ApiErrorCode::NotFound
                    | ApiErrorCode::MissingPermissions
                    | ApiErrorCode::LimitReached,
                ..
            }
        )
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_refusal_by_the_instance_drops_a_message_from_the_outbox() {
        let api = |code| ClientError::Api {
            status: 400,
            code,
            message: String::new(),
        };
        assert!(api(ApiErrorCode::ValidationFailed).refuses_message_for_good());
        assert!(api(ApiErrorCode::NotFound).refuses_message_for_good());
        for retryable in [
            ApiErrorCode::RateLimited,
            ApiErrorCode::ServerBusy,
            ApiErrorCode::Internal,
            ApiErrorCode::Unauthorized,
        ] {
            assert!(!api(retryable).refuses_message_for_good());
        }
        assert!(!ClientError::UnexpectedResponse("html".into()).refuses_message_for_good());
    }
}

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
    /// An upload the instance refused for its size (`413`), or a download larger than the cap
    /// the caller gave.
    #[error("the file is too large")]
    FileTooLarge,
    /// A download whose answer or body stopped arriving for longer than the stall timeout.
    #[error("the transfer stalled")]
    TransferStalled,
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
            } | Self::FileTooLarge
        )
    }

    /// True when the instance answered and refused the change: a guess shown before the answer
    /// is undone (`state::ChangeOutcome::Undone`). Otherwise the answer may have been lost after
    /// the instance kept the change, and its copy settles it.
    #[must_use]
    pub fn is_refused_by_instance(&self) -> bool {
        matches!(self, Self::Api { .. } | Self::FileTooLarge)
    }

    /// True when the same request may succeed later: the instance was out of reach, too busy or
    /// failing, never a refusal of the request itself.
    #[must_use]
    pub fn is_worth_retrying(&self) -> bool {
        match self {
            Self::Api { status, .. } => *status == 429 || *status >= 500,
            Self::Network(_) | Self::UnexpectedResponse(_) | Self::TransferStalled => true,
            Self::InvalidInstanceAddress(_)
            | Self::NotSignedIn
            | Self::RandomGeneratorFailed(_)
            | Self::FileTooLarge => false,
        }
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
        assert!(ClientError::FileTooLarge.refuses_message_for_good());
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

    #[test]
    fn only_a_busy_or_failing_instance_is_worth_retrying_and_any_answer_is_a_refusal() {
        let api = |status, code| ClientError::Api {
            status,
            code,
            message: String::new(),
        };
        assert!(api(429, ApiErrorCode::RateLimited).is_worth_retrying());
        assert!(api(503, ApiErrorCode::ServerBusy).is_worth_retrying());
        assert!(!api(404, ApiErrorCode::NotFound).is_worth_retrying());
        assert!(!api(403, ApiErrorCode::MissingPermissions).is_worth_retrying());
        assert!(ClientError::UnexpectedResponse("html".into()).is_worth_retrying());
        assert!(api(403, ApiErrorCode::MissingPermissions).is_refused_by_instance());
        assert!(!ClientError::UnexpectedResponse("html".into()).is_refused_by_instance());
    }
}

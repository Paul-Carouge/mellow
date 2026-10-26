//! Friends, friend requests and blocks (ADR-0025). Each user sees only their own relationships.

use serde::{Deserialize, Serialize};

use crate::PublicUser;

/// At most this many friends and outgoing requests per user; blocks never count.
pub const MAXIMUM_RELATIONSHIPS_PER_USER: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipKind {
    Friend,
    /// They asked; accept with `PUT …/relationships/{user_id}`, refuse with `DELETE`.
    IncomingRequest,
    /// You asked; cancel with `DELETE`.
    OutgoingRequest,
    /// You blocked them; they are never told.
    Blocked,
}

/// One relationship as its owner sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relationship {
    pub user: PublicUser,
    pub kind: RelationshipKind,
    /// When it took this kind.
    pub since_unix_milliseconds: u64,
}

/// Body of `POST /api/v1/users/@me/relationships`: asks `username` to be friends, or accepts
/// their request if they already asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendFriendRequest {
    pub username: String,
}

/// Body of `PUT /api/v1/users/@me/relationships/{user_id}`: `friend` accepts their request,
/// `blocked` blocks them (whatever the relationship was).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetRelationshipRequest {
    pub kind: SetRelationshipKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetRelationshipKind {
    Friend,
    Blocked,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_use_snake_case_on_the_wire() {
        assert_eq!(
            serde_json::to_string(&RelationshipKind::IncomingRequest).unwrap(),
            r#""incoming_request""#
        );
        let request: SetRelationshipRequest =
            serde_json::from_str(r#"{"kind":"blocked"}"#).unwrap();
        assert_eq!(request.kind, SetRelationshipKind::Blocked);
        assert!(
            serde_json::from_str::<SetRelationshipRequest>(r#"{"kind":"incoming_request"}"#)
                .is_err()
        );
    }
}

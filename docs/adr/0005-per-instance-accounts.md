# 0005. Keep accounts local to each instance

- Status: Accepted
- Date: 2026-10-19

## Context
Mellow is self-hosted, not federated. Users may belong to several communities hosted on different instances.

## Decision
Each instance owns its accounts (username, email, Argon2id password hash). Clients support several instances side by side, each with its own session token stored in the OS keychain. Registration mode is chosen by the admin: open, invite-only or closed.

## Consequences
- No dependency on any external identity provider; simplest possible setup.
- A user on three instances has three accounts. The client hides most of this friction by listing instances in the server rail.
- SSO (OIDC) can be added later as an additional login method without changing the account model.

## Alternatives considered
- **Federated identity:** large protocol and trust complexity, out of scope.
- **Mandatory SSO:** forces small communities to run an identity provider.

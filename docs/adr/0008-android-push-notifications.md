# 0008. Deliver Android push through UnifiedPush with FCM fallback

- Status: Accepted
- Date: 2026-10-19

## Context
Android kills background connections, so the app needs push notifications. FCM is the default on Google devices but requires a Firebase project, and server credentials that a self-hoster cannot easily get for an app they did not publish. Many privacy-minded users run devices without Google services.

## Decision
- The app registers with UnifiedPush when a distributor (for example ntfy) is installed, and with FCM otherwise.
- UnifiedPush: the instance sends Web Push (RFC 8030/8291, encrypted) directly to the distributor endpoint.
- FCM: instances send encrypted notifications to a small, stateless **Mellow push relay** operated by the project, which holds the FCM credentials of the published app and forwards to Google. Self-hosters can disable FCM or point to their own relay.
- Push payloads carry only identifiers (instance, channel, message id) and are encrypted end to end between instance and device; the app fetches content over its authenticated connection.

## Consequences
- Works on de-Googled devices and on standard devices.
- The project must run and secure the push relay; it sees only encrypted, content-free payloads and device tokens.
- The relay is a new component; it lives in the workspace as a later crate (`mellow-push-relay`) when Android push is implemented.

## Alternatives considered
- **FCM only:** excludes de-Googled devices and forces each instance to hold Firebase credentials.
- **UnifiedPush only:** users must install a distributor; too much friction for most.
- **Persistent foreground service:** drains battery and shows a permanent notification.

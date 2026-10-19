# 0004. Use REST for commands and a WebSocket gateway for events

- Status: Accepted
- Date: 2026-10-19

## Context
Clients need request/response operations (send message, fetch history, change settings) and a stream of real-time events (new messages, presence, typing).

## Decision
- REST over HTTPS under `/api/v1` with JSON bodies for all commands and queries.
- One WebSocket per instance at `/gateway?v=1` for server-to-client events, with `Identify`, `Ready`, heartbeats, sequence numbers and `Resume`.
- 64-bit time-ordered Snowflake-style identifiers, serialized as JSON strings.
- Errors carry a stable machine-readable `code` and a human `message`.

## Consequences
- REST is easy to debug, cache, rate-limit and test; the gateway stays a one-way event stream, which keeps its state machine small.
- JSON costs more bandwidth than a binary format. Acceptable at our scale; per-message compression (`permessage-deflate`) can be enabled later.
- Time-ordered ids give cursor pagination and history ordering without extra indexes.

## Alternatives considered
- **Everything over WebSocket:** fewer connections but reinvents request/response, retries and error mapping.
- **gRPC:** good tooling but heavy on Android/desktop for little gain, and awkward for the admin web UI.
- **UUIDv7:** 128 bits, larger indexes, no benefit over 64-bit ids on a single node.

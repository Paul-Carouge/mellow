// When an id was made: Snowflake-style ids keep their creation time (ADR-0004).

import type { Snowflake } from "./protocol";

/** Unix time of 2026-01-01T00:00:00Z: `MELLOW_EPOCH_UNIX_MILLISECONDS` in mellow-protocol. */
const MELLOW_EPOCH_UNIX_MILLISECONDS = 1_767_225_600_000;
/** Ids keep 42 bits of milliseconds above 22 bits of sequence and reserved bits (ADR-0004). */
const SNOWFLAKE_TIMESTAMP_SHIFT = 22n;

export function unixMillisecondsOfSnowflake(id: Snowflake): number {
  return Number(BigInt(id) >> SNOWFLAKE_TIMESTAMP_SHIFT) + MELLOW_EPOCH_UNIX_MILLISECONDS;
}

// The servers and groups whose docked member sheet was left open on this device (DESIGN.md 5.7):
// remembered across visits, so it does not have to be reopened every time.

import type { Snowflake } from "../lib/protocol";

const MEMBER_SHEET_PLACES_STORAGE_KEY = "mellow.member-sheet-places";
/** The latest places only, so a hostile or corrupted entry cannot grow without bound. */
export const MAXIMUM_REMEMBERED_MEMBER_SHEET_PLACES = 200;
const SNOWFLAKE = /^\d{1,20}$/u;

/** The places stored by this browser, most recently opened last; nothing when unreadable. */
export function readMemberSheetPlaceIds(storage: Pick<Storage, "getItem"> | null): Snowflake[] {
  let stored: unknown;
  try {
    stored = JSON.parse(storage?.getItem(MEMBER_SHEET_PLACES_STORAGE_KEY) ?? "[]");
  } catch (error) {
    console.warn("unreadable member sheet places", error);
    return [];
  }
  if (!Array.isArray(stored)) {
    return [];
  }
  const placeIds = stored.filter((value): value is Snowflake => typeof value === "string" && SNOWFLAKE.test(value));
  return [...new Set(placeIds)].slice(-MAXIMUM_REMEMBERED_MEMBER_SHEET_PLACES);
}

/** Stores the places when the browser lets us; a full or blocked storage remembers them for this tab only. */
export function writeMemberSheetPlaceIds(storage: Pick<Storage, "setItem"> | null, placeIds: Iterable<Snowflake>): void {
  try {
    storage?.setItem(
      MEMBER_SHEET_PLACES_STORAGE_KEY,
      JSON.stringify([...placeIds].slice(-MAXIMUM_REMEMBERED_MEMBER_SHEET_PLACES)),
    );
  } catch (error) {
    console.warn("couldn't store the member sheet places", error);
  }
}

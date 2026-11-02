import { describe, expect, it, vi } from "vitest";

import { memoryStorage } from "../test-memory-storage";
import {
  MAXIMUM_REMEMBERED_MEMBER_SHEET_PLACES,
  readMemberSheetPlaceIds,
  writeMemberSheetPlaceIds,
} from "./member-sheet-places";

describe("member sheet places", () => {
  it("remembers the places across visits", () => {
    const storage = memoryStorage();
    writeMemberSheetPlaceIds(storage, new Set(["101", "202"]));

    expect(readMemberSheetPlaceIds(storage)).toEqual(["101", "202"]);
  });

  it("starts with none when nothing or something unreadable is stored", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const storage = memoryStorage();
    expect(readMemberSheetPlaceIds(storage)).toEqual([]);
    expect(readMemberSheetPlaceIds(null)).toEqual([]);

    storage.setItem("mellow.member-sheet-places", "{not json");
    expect(readMemberSheetPlaceIds(storage)).toEqual([]);
    storage.setItem("mellow.member-sheet-places", '{"101":true}');
    expect(readMemberSheetPlaceIds(storage)).toEqual([]);
  });

  it("keeps only ids, once each", () => {
    const storage = memoryStorage();
    storage.setItem("mellow.member-sheet-places", JSON.stringify(["101", 7, "abc", "101", "", "202"]));

    expect(readMemberSheetPlaceIds(storage)).toEqual(["101", "202"]);
  });

  it("keeps the latest places only", () => {
    const storage = memoryStorage();
    const placeIds = Array.from({ length: MAXIMUM_REMEMBERED_MEMBER_SHEET_PLACES + 5 }, (_, index) => String(index + 1));
    writeMemberSheetPlaceIds(storage, placeIds);

    const remembered = readMemberSheetPlaceIds(storage);
    expect(remembered).toHaveLength(MAXIMUM_REMEMBERED_MEMBER_SHEET_PLACES);
    expect(remembered.at(-1)).toBe(String(MAXIMUM_REMEMBERED_MEMBER_SHEET_PLACES + 5));
  });

  it("forgets nothing already open when the storage refuses to write", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const refusing = {
      setItem: () => {
        throw new Error("quota");
      },
    };
    expect(() => writeMemberSheetPlaceIds(refusing, ["101"])).not.toThrow();
  });
});

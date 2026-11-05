import { describe, expect, it } from "vitest";

import { ApiError } from "../lib/api";
import type { Guild, Member, Role, UserProfile } from "../lib/protocol";
import { parseInlineMarkdown } from "./markdown";
import {
  canSaveProfileDraft,
  characterCount,
  linkProblem,
  previewLinks,
  profileRequestOf,
} from "./profile-details-form";
import { highestRoleWithIcon, roleIconsByUserId } from "./role-icons";
import { createUserProfileLoader } from "./user-profile-loader.svelte";

function role(id: string, position: number, icon: Partial<Pick<Role, "icon" | "unicode_emoji">> = {}): Role {
  return { id, name: id, color: 0, position, permissions: "0", is_displayed_separately: false, ...icon };
}

function guild(roles: Role[]): Guild {
  return { id: "g", name: "Club", owner_id: "o", my_permissions: "0", my_role_ids: [], roles, channels: [] };
}

function member(id: string, roleIds: string[]): Member {
  return { user: { id, username: id, display_name: id }, role_ids: roleIds, joined_at_unix_milliseconds: 0 };
}

describe("role icons", () => {
  const club = guild([
    role("g", 0),
    role("plain", 9),
    role("emoji", 3, { unicode_emoji: "🔥" }),
    role("picture", 5, { icon: { id: "900", is_animated: false } }),
  ]);

  it("shows the member's highest role with an icon, never one they do not hold", () => {
    expect(highestRoleWithIcon(club, ["plain", "emoji", "picture"])?.id).toBe("picture");
    expect(highestRoleWithIcon(club, ["plain", "emoji"])?.id).toBe("emoji");
    expect(highestRoleWithIcon(club, ["plain"])).toBeNull();
  });

  it("maps members to their icon, and nobody when the server has no role icon", () => {
    const icons = roleIconsByUserId(club, [member("a", ["emoji"]), member("b", [])]);
    expect([...icons.entries()].map(([userId, shown]) => [userId, shown.id])).toEqual([["a", "emoji"]]);
    expect(roleIconsByUserId(guild([role("g", 0), role("plain", 1)]), [member("a", ["plain"])]).size).toBe(0);
    expect(roleIconsByUserId(null, undefined).size).toBe(0);
  });
});

describe("about me and links form", () => {
  it("counts characters as the instance does, emoji included", () => {
    expect(characterCount("é🔥")).toBe(2);
    expect(canSaveProfileDraft({ aboutMe: "🔥".repeat(190), links: [] })).toBe(true);
    expect(canSaveProfileDraft({ aboutMe: "🔥".repeat(191), links: [] })).toBe(false);
  });

  it("accepts only https links and leaves blank rows out of the request", () => {
    expect(linkProblem({ url: "https://example.org/a", label: "" })).toBeNull();
    expect(linkProblem({ url: "  ", label: "" })).toBeNull();
    expect(linkProblem({ url: "http://example.org/", label: "" })).toBe("not-https");
    expect(linkProblem({ url: "javascript:alert(1)", label: "" })).toBe("not-https");
    expect(linkProblem({ url: "example.org", label: "" })).toBe("not-https");
    expect(linkProblem({ url: `https://example.org/${"a".repeat(300)}`, label: "" })).toBe("too-long");
    expect(
      profileRequestOf({
        aboutMe: "  hi  ",
        links: [
          { url: " https://example.org/ ", label: " Blog " },
          { url: "", label: "lost" },
          { url: "https://example.com/", label: "  " },
        ],
      }),
    ).toEqual({ about_me: "hi", links: [{ url: "https://example.org/", label: "Blog" }, { url: "https://example.com/" }] });
  });

  it("previews typed links with the icon their host already has", () => {
    const links = previewLinks(
      { aboutMe: "", links: [{ url: "https://Example.org/new", label: "" }, { url: "http://bad/", label: "" }] },
      [{ url: "https://example.org/old", icon_id: "950" }],
    );
    expect(links).toEqual([{ url: "https://Example.org/new", label: null, icon_id: "950" }]);
  });

  it("reads inline Markdown only", () => {
    const [paragraph, ...rest] = parseInlineMarkdown("# Not a heading **bold**\n- not a list");
    expect(rest).toEqual([]);
    expect(paragraph?.kind).toBe("paragraph");
    expect(JSON.stringify(paragraph)).toContain('"strong"');
    expect(JSON.stringify(paragraph)).toContain("# Not a heading");
    expect(parseInlineMarkdown("")).toEqual([]);
  });
});

describe("user profile loader", () => {
  const profile = (userId: string): UserProfile => ({ user_id: userId, about_me: `about ${userId}`, links: [] });

  it("keeps only the answer for the profile shown last", async () => {
    const answers = new Map<string, (profile: UserProfile) => void>();
    const loader = createUserProfileLoader(
      (userId) => new Promise((resolve) => answers.set(userId, resolve)),
    );
    loader.show("a");
    loader.show("b");
    answers.get("b")?.(profile("b"));
    answers.get("a")?.(profile("a"));
    await Promise.resolve();
    await Promise.resolve();
    expect(loader.current).toEqual({ kind: "loaded", userId: "b", profile: profile("b") });
  });

  it("shows no details for someone it may not read, and a failure otherwise", async () => {
    const hidden = createUserProfileLoader(() => Promise.reject(new ApiError(404, "NOT_FOUND")));
    hidden.show("a");
    const broken = createUserProfileLoader(() => Promise.reject(new ApiError(0, "NETWORK")));
    broken.show("a");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(hidden.current).toEqual({ kind: "loaded", userId: "a", profile: { user_id: "a", about_me: "", links: [] } });
    expect(broken.current).toEqual({ kind: "failed", userId: "a" });
    broken.show(null);
    expect(broken.current).toEqual({ kind: "idle" });
  });
});

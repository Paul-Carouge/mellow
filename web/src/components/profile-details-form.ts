// The "About me" and links editor's draft (ADR-0050): what it sends, and what it refuses before
// sending, by the same rules as the instance (`profile_rules.rs`).

import {
  MAXIMUM_ABOUT_ME_LENGTH,
  MAXIMUM_PROFILE_LINK_LABEL_LENGTH,
  MAXIMUM_PROFILE_LINK_URL_LENGTH,
  type ProfileLink,
  type UpdateUserProfileRequest,
  type UserProfile,
} from "../lib/protocol";

export interface ProfileLinkDraft {
  readonly url: string;
  readonly label: string;
}

export interface ProfileDraft {
  readonly aboutMe: string;
  readonly links: readonly ProfileLinkDraft[];
}

export type ProfileLinkProblem = "not-https" | "too-long";

export const EMPTY_PROFILE_DRAFT: ProfileDraft = { aboutMe: "", links: [] };

/** Characters as the instance counts them: Unicode scalar values, not UTF-16 units. */
export function characterCount(text: string): number {
  return [...text].length;
}

export function profileDraftOf(profile: UserProfile): ProfileDraft {
  return {
    aboutMe: profile.about_me,
    links: profile.links.map((link) => ({ url: link.url, label: link.label ?? "" })),
  };
}

/** Blank link rows are left out, so an empty row never blocks a save. */
export function profileRequestOf(draft: ProfileDraft): UpdateUserProfileRequest {
  return {
    about_me: draft.aboutMe.trim(),
    links: draft.links
      .filter((link) => link.url.trim() !== "")
      .map((link) => {
        const label = link.label.trim();
        return label === "" ? { url: link.url.trim() } : { url: link.url.trim(), label };
      }),
  };
}

export function isAboutMeTooLong(draft: ProfileDraft): boolean {
  return characterCount(draft.aboutMe.trim()) > MAXIMUM_ABOUT_ME_LENGTH;
}

/** What is wrong with a link row, or null; a blank row is fine (it is not sent). */
export function linkProblem(link: ProfileLinkDraft): ProfileLinkProblem | null {
  const url = link.url.trim();
  if (url === "") {
    return null;
  }
  if (characterCount(url) > MAXIMUM_PROFILE_LINK_URL_LENGTH || characterCount(link.label.trim()) > MAXIMUM_PROFILE_LINK_LABEL_LENGTH) {
    return "too-long";
  }
  try {
    const parsed = new URL(url);
    return parsed.protocol === "https:" && parsed.hostname !== "" && parsed.username === "" && parsed.password === ""
      ? null
      : "not-https";
  } catch {
    return "not-https";
  }
}

export function canSaveProfileDraft(draft: ProfileDraft): boolean {
  return !isAboutMeTooLong(draft) && draft.links.every((link) => linkProblem(link) === null);
}

/**
 * The links the preview shows: the draft's, each with the icon its host already has among the
 * saved links, so a typed link shows the glyph until it is saved and fetched.
 */
export function previewLinks(draft: ProfileDraft, saved: readonly ProfileLink[]): ProfileLink[] {
  const iconByHost = new Map<string, string>();
  for (const link of saved) {
    const host = hostOf(link.url);
    if (host !== null && link.icon_id) {
      iconByHost.set(host, link.icon_id);
    }
  }
  return profileRequestOf(draft).links.flatMap((link) => {
    const host = hostOf(link.url);
    if (host === null || linkProblem({ url: link.url, label: link.label ?? "" }) !== null) {
      return [];
    }
    return [{ url: link.url, label: link.label ?? null, icon_id: iconByHost.get(host) ?? null }];
  });
}

function hostOf(url: string): string | null {
  try {
    return new URL(url).hostname.replace(/\.$/u, "").toLowerCase();
  } catch {
    return null;
  }
}

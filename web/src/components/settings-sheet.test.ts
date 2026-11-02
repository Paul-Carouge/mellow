import { createRawSnippet } from "svelte";
import { render } from "svelte/server";
import { describe, expect, it } from "vitest";

import SettingsSheet from "./SettingsSheet.svelte";

const emptySnippet = createRawSnippet(() => ({ render: () => "<span></span>" }));
const pageWithScreenReaderOnlyRadio = createRawSnippet(() => ({
  render: () => '<input type="radio" class="sr-only" name="permission">',
}));

function renderedSheet(): string {
  return render(SettingsSheet, {
    props: {
      label: "Channel settings",
      title: "#general",
      backLabel: "Back",
      isWide: true,
      isPageShownOnPhone: false,
      pageTitle: "Permissions",
      initialFocusId: null,
      onrequestclose: () => {},
      onback: () => {},
      navigation: emptySnippet,
      page: pageWithScreenReaderOnlyRadio,
    },
  }).body;
}

describe("SettingsSheet", () => {
  // A `sr-only` control is absolutely positioned: with no positioned ancestor before the dialog it
  // sits outside the scrolling page, overflows the dialog, and focusing it (choosing Deny on a
  // permission low in the page) scrolled the whole dialog up, leaving the bottom of the window empty.
  it("makes the scrolling page the containing block of its screen-reader-only controls", () => {
    const html = renderedSheet();
    const scrollingPageTag = html.match(/<div class="[^"]*overflow-y-auto bg-\(--mw-component-settings-content-background\)[^"]*"/)?.[0];

    expect(scrollingPageTag).toBeDefined();
    expect(scrollingPageTag).toMatch(/\brelative\b/);
  });
});

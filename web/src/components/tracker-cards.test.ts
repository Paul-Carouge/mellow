import { render } from "svelte/server";
import { describe, expect, it } from "vitest";

import { strings } from "../i18n";
import type { Checklist } from "../lib/protocol";
import ChecklistCard from "./ChecklistCard.svelte";
import ProgressBarCard from "./ProgressBarCard.svelte";

const checklist: Checklist = {
  title: "Release",
  items: [
    { id: 1, text: "Tag", checked: { user_id: "2", at_unix_milliseconds: 0 } },
    { id: 2, text: "Build" },
    { id: 3, text: "Deploy" },
  ],
  revision: 3,
};

describe("the checklist card", () => {
  it("shows a 1/3 progress bar and one checkbox per item, with who ticked it", () => {
    const html = render(ChecklistCard, {
      props: { checklist, canTick: true, canEdit: false, nameOfUser: () => "Ada" },
    }).body;
    expect(html).toMatch(/role="progressbar"[^>]*aria-valuemax="3"[^>]*aria-valuenow="1"/);
    expect(html).toContain(`aria-valuetext="${strings.checklists.doneSpoken(1, 3)}"`);
    expect(html.match(/role="checkbox"/g)).toHaveLength(3);
    expect(html.match(/aria-checked="true"/g)).toHaveLength(1);
    expect(html).toContain("Ada");
    expect(html).not.toContain(strings.checklists.edit);
  });

  it("names each checkbox by its text alone, and describes it by who ticked it", () => {
    const html = render(ChecklistCard, {
      props: { checklist, canTick: true, canEdit: false, nameOfUser: () => "Ada" },
    }).body;
    const ticked = html.match(/role="checkbox"[^>]*aria-labelledby="([^"]+)"[^>]*aria-describedby="([^"]+)"/);
    expect(ticked).not.toBeNull();
    const [, labelId, descriptionId] = ticked!;
    expect(html).toMatch(new RegExp(`id="${labelId}"[^>]*>Tag<`));
    expect(html).toMatch(new RegExp(`id="${descriptionId}"[^>]*>Ticked by Ada, `));
  });

  it("draws one segment per item and says when every item is ticked", () => {
    const partly = render(ChecklistCard, {
      props: { checklist, canTick: true, canEdit: false, nameOfUser: () => "Ada" },
    }).body;
    expect(partly.match(/class="segment /g)).toHaveLength(3);
    expect(partly).not.toContain(strings.checklists.allDone);
    const tick = { user_id: "2", at_unix_milliseconds: 0 };
    const finished = render(ChecklistCard, {
      props: {
        checklist: { ...checklist, items: checklist.items.map((item) => ({ ...item, checked: tick })) },
        canTick: true,
        canEdit: false,
        nameOfUser: () => "Ada",
      },
    }).body;
    expect(finished).toContain(strings.checklists.allDone);
  });

  it("says why its boxes are inert: read only where the reader may not write, else neutrally", () => {
    const readOnly = render(ChecklistCard, {
      props: { checklist, canTick: false, canWrite: false, canEdit: false, nameOfUser: () => "Ada" },
    }).body;
    expect(readOnly).toContain(strings.checklists.readOnly);
    expect(readOnly).not.toContain(strings.checklists.cannotTick);
    expect(readOnly.match(/aria-disabled="true"/g)).toHaveLength(3);
    const cannotTick = render(ChecklistCard, {
      props: { checklist, canTick: false, canWrite: true, canEdit: false, nameOfUser: () => "Ada" },
    }).body;
    expect(cannotTick).toContain(strings.checklists.cannotTick);
    expect(cannotTick).not.toContain(strings.checklists.readOnly);
    const ticking = render(ChecklistCard, {
      props: { checklist, canTick: true, canWrite: true, canEdit: false, nameOfUser: () => "Ada" },
    }).body;
    expect(ticking).not.toContain(strings.checklists.cannotTick);
  });

  it("keeps who ticked an item out of the reading order: it is only the checkbox's description", () => {
    const html = render(ChecklistCard, {
      props: { checklist, canTick: true, canEdit: false, nameOfUser: () => "Ada" },
    }).body;
    expect(html).toMatch(/id="[^"]+-by-1" hidden(="")?>Ticked by Ada, /);
  });

  it("offers editing to its author or moderators, and nothing to press while pending", () => {
    const editable = render(ChecklistCard, {
      props: { checklist, canTick: true, canEdit: true, nameOfUser: () => "Ada" },
    }).body;
    expect(editable).toContain(strings.checklists.edit);
    const pending = render(ChecklistCard, {
      props: { checklist, isPending: true, canTick: true, canEdit: true, nameOfUser: () => "Ada" },
    }).body;
    expect(pending).not.toContain(strings.checklists.edit);
    expect(pending.match(/aria-disabled="true"/g)).toHaveLength(3);
  });
});

describe("the progress bar card", () => {
  it("reads as a percentage out of 100, else as n/m", () => {
    const percent = render(ProgressBarCard, {
      props: { bar: { label: "Sync", value: 40, maximum: 100, revision: 0 }, canEdit: false },
    }).body;
    expect(percent).toMatch(/role="progressbar"[^>]*aria-valuemax="100"[^>]*aria-valuenow="40"/);
    expect(percent).toContain(strings.progressBars.figure(40, 100, 40));
    const fraction = render(ProgressBarCard, {
      props: { bar: { label: "Sync", value: 3, maximum: 5, revision: 0 }, canEdit: true },
    }).body;
    expect(fraction).toContain(`aria-valuetext="${strings.progressBars.spoken(3, 5, 60)}"`);
    expect(fraction).toContain(strings.progressBars.update);
  });
});

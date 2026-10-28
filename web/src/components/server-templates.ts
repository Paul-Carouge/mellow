// The starter templates of "Create a server" (DESIGN.md §5.2): each a list of categories and text
// channels, named in the interface's language and sent as `initial_categories`. Blank still gives
// one category with a general channel, as the instance does when a client sends none.

import { type Catalogue, strings } from "../i18n";
import type { InitialCategory } from "../lib/protocol";

export type ServerTemplateId = keyof Catalogue["addServer"]["templates"];

/** In the order the dialog lists them; the first is chosen when it opens. */
export const SERVER_TEMPLATE_IDS: readonly ServerTemplateId[] = ["friends", "community", "studyGroup", "gaming", "blank"];

export interface ServerTemplate {
  readonly id: ServerTemplateId;
  readonly name: string;
  readonly categories: readonly InitialCategory[];
}

export function serverTemplates(catalogue: Catalogue = strings): ServerTemplate[] {
  return SERVER_TEMPLATE_IDS.map((id) => {
    const words = catalogue.addServer.templates[id];
    return { id, name: words.name, categories: words.categories };
  });
}

/** "general, photos, plans": the channels a template brings, as its row describes it. */
export function templateChannelNames(template: Pick<ServerTemplate, "categories">): string {
  return template.categories.flatMap((category) => category.channels.map((channel) => channel.name)).join(", ");
}

# Checklists and progress bars on the web (ROADMAP item 18)

Status: designed and built on `feat/checklists-and-progress-bars`, redesigned on 2026-11-15 (see [Redesign](#redesign-2026-11-15)) (ADR-0047: both are messages; ticks, items and values are shared state on the instance, versioned by a revision), DESIGN.md v0.55. `ChecklistCard.svelte` and `ProgressBarCard.svelte` (rendered by `MessageItem.svelte`) share `ProgressMeter.svelte`; `CreateChecklistDialog.svelte` and `ProgressBarDialog.svelte` are lazy chunks opened by `ChatPane.svelte`. Rules live in `components/tracker-form.ts` (the forms' checks), `components/tracker-view.ts` (one-line summaries, outbox previews) and `lib/checklists.ts` (newest copy, ticks, counts, percentages). Both cards take their frame from `component.poll.*` and everything inside from `component.checklist.*` (2026-11-15). Icons: Phosphor `list-checks` and `gauge` (Fill).

## Brief

Clémence runs the release on Friday and wants the team to see what is left without asking: a list everyone ticks, with who did what. Bob is moving the photo archive and wants the channel to see how far it is without posting "40 %… 55 %…". The job: **share a list in ten seconds, tick in one click, and read progress at a glance.** Discord has neither; the owner asked for both, with their permission (#roadmap).

Assumption: checklists hold three to ten short items and are ticked over hours or days; progress bars move a handful of times.

## Flow and states

1. **Offer.** "Checklist" and "Progression" / "Progress bar" join the composer's "/" menu after "Sondage", found by "/checklist", "/todo", "/progression" or "/progress" in either language, only where the writer holds **Créer des checklists et des barres de progression** (computed channel permissions), and in conversations whenever they may write there. Like the poll, they write nothing: choosing one removes the "/query" and opens its dialog.
2. **Create a checklist.** "Créer une checklist": Titre (300 characters, counter always shown), Éléments (three rows to start, 200 characters each, counter from 180, "Ajouter" up to 25, a cross on each row while there is more than one, "Tu peux en ajouter encore 22." announced politely). Enter goes to the next row and adds one after the last; Enter never sends. Items may repeat. A too long text is said as it is typed; a missing title or no item only on "Envoyer la checklist", which then focuses the first field to fix. Blank rows are dropped.
3. **Create a progress bar.** "Créer une barre de progression": Libellé (300 characters), Valeur and Sur side by side (whole numbers, numeric keyboard on phones), "Sur 100, elle s'affiche en pourcentage." under them. It opens at 0 sur 100. Problems ("Un nombre entier de 0 à 5.") are said on "Envoyer", focus on the first field to fix.
4. **Send.** Through the outbox with a nonce, like a poll: the dialog closes, the list jumps to the present, the card shows as it will arrive with nothing pressable, and a refusal leaves it "Non envoyé." with "Réessayer" and "Supprimer".
5. **Read a checklist.** The poll's well, 600 px at most: the `list-checks` glyph (the boxes' size, over their column, on the title's first line) and the title, then the segmented level (one segment per item, "3/5" beside it, "Tout est coché" once done), then one borderless row per item: a 22 px box and the text, never truncated. A ticked row fills its box, dims and strikes its text, and shows "Ada · Aujourd'hui à 14:32" under it; "Cochée par Ada, jeudi 5 novembre 2026 à 14:32" is the checkbox's description for screen readers (`aria-describedby` on a `hidden` element, so it is not read twice).
6. **Tick.** A click, Space or Enter on a row ticks or unticks it at once, as the reader; the instance's answer then replaces the guess, with the real time. Rows are `role=checkbox` buttons with `aria-checked`, 44 px tall at least on every pointer, each its own Tab stop (a list, not a radio group). A second press while the first is on its way is ignored. A refusal undoes the tick and says why above the composer ("Impossible de cocher l'élément : …"), unless someone else's change arrived meanwhile, which wins. After a lost answer the client reads the instance's copy instead of guessing.
7. **Edit a checklist.** Its author, or a moderator ("Gérer les messages") in a server channel, sees "Modifier la liste" under the items. In editing, each item is a field (Enter or leaving it renames; blank or unchanged keeps it), with a trash button (none on the last item: "Une checklist garde au moins un élément."), and "Nouvel élément" with "Ajouter" at the end (disabled at 25). "Terminé" returns to ticking. Ticks survive renames.
8. **Read and update a progress bar.** The card: the `gauge` glyph and the label, the level and its figure ("40 %" out of 100, else "3/5"). Its author or a moderator sees "Mettre à jour", which opens the same dialog filled with the current values ("Mettre à jour la progression", "Enregistrer"); the card moves when the instance answers.
9. **Motion.** The level's fill eases to its new width over `motion.duration.slow` with the standard easing; under reduced motion the duration token is 0, so it jumps.
10. **Screen readers.** The level is a `progressbar` named by the card's heading, with `aria-valuenow`, `aria-valuemax` and `aria-valuetext` ("3 sur 5 faites", "40 %"). Screen readers do not announce a progress bar's new value on their own, so a polite live region beside it says the new value when it changes (never the first one: a page of history stays quiet).
11. **Elsewhere.** Neither has Edit, Forward or Copy text, and Up in an empty composer skips them. Reply lines, pins, notifications and dialog previews read "Checklist : {titre}" or "Progression : {libellé}"; a search slip shows them with their glyph. Without the right to write, a checklist's marks are inert (`aria-disabled`).

```
┌──────────────────────────────────────────────────────────┐
│  ☷  Release vendredi                                     │
│  ▬▬▬▬▬▬▬▬▬ ▭▭▭▭▭▭▭▭▭ ▭▭▭▭▭▭▭▭▭                     1/3   │
│  ■  T̶a̶g̶                                                   │
│     Ada · Aujourd'hui à 14:32                            │
│  □  Build                                                │
│  □  Deploy                                               │
│  ✎ Modifier la liste                                     │
└──────────────────────────────────────────────────────────┘
```

## Redesign (2026-11-15)

The owner disliked the first cards: "je déteste le style, pas bien adaptés, trop petits en colonnes". Seen on screen, the problems were: a 432 px column leaving most of the sheet empty, every row a bordered box (ticked ones with a lamp edge, so a finished list was a stack of orange frames), an 18 px mark, a 12 px level filled with a brown that read as disabled, the figure and "Modifier la liste" at 12 px, and "Cochée par Chloé Martin, dimanche 15 novembre 2026 à 20:24" wrapping on two lines under each item on a phone.

What changed (web `ChecklistCard.svelte`, `ProgressMeter.svelte`, `ProgressBarCard.svelte`; desktop `messages/cards.rs`, `components/progress.rs`, `conversation/checklist_editor.rs`; tokens `component.checklist.*`):

- **Width.** 600 px at most instead of 432: a sentence-long item stays on one line on a laptop.
- **Rows without borders.** A list, not a stack of cards: 44 px rows that light with `state.hover` under the pointer, like the navigator's. A ticked row says it with its box, its dimmed, struck text and the "who" line, not with a frame.
- **Larger boxes.** 22 px (from 18), a 6 px radius, the check 14 px; the title's glyph has the box's size and sits over the column of boxes.
- **A segmented level.** One segment per item, lit with the same lamp as a ticked box, so "3 of 5" reads as three lit items at a glance; the figure at body size; "Tout est coché" in green once done. The progress bar keeps a continuous fill in the same colours.
- **Who and when, short.** "Chloé · Aujourd'hui à 14:32" (`messageTimestampLabel`) on one line; the full sentence stays for screen readers. On the web the checkbox stretches over its row, so that line ticks too while staying out of the checkbox's name (Lighthouse's label-in-name check flagged the first version).
- **Quiet actions.** "Modifier la liste" and "Mettre à jour" are buttons in `text.secondary` with a row-like hover, not 12 px links.
- **Read-only said.** Inert boxes get a dimmer edge and, under the rows, "Lecture seule : tu ne peux pas écrire ici." where the reader may not write, else "Tu ne peux pas cocher cette liste." (writing allowed but not reading the history).
- **Phones.** Below a 360 px card, the card's padding drops to 4 px and the rows' to 8 px, giving 16 px back to the text on each side.
- **Motion.** The check grows into its box with the settle curve, the box and the segment change colour; nothing moves on first render, nothing under reduced motion.

## Decisions

- **The poll card's frame, the checklist's own insides** (2026-11-15). The three cards share the well; the checklist and the progress bar have their own rows, boxes and level (`component.checklist.*`). The level is the lamp (`accent.indicator`), as a ticked box is: it is the channel's shared progress, the one thing on the card everyone looks at. Read-only is said in words: "Lecture seule : tu ne peux pas écrire ici." where the reader may not write, else "Tu ne peux pas cocher cette liste."
- **Who and when, in words, under the item.** A tooltip hides it from touch and screen readers; the line is short and only on ticked items.
- **No optimistic item edits.** Renames and removals are rare and their failures need the field back: the card waits for the instance's answer (a few tens of milliseconds).
- **One dialog for creating and updating a progress bar.** The same three fields; "n sur m" rather than a percentage mode switch, since "sur 100" says it.

# Checklists and progress bars on the web (ROADMAP item 18)

Status: designed and built on `feat/checklists-and-progress-bars` (ADR-0047: both are messages; ticks, items and values are shared state on the instance, versioned by a revision), DESIGN.md v0.55. `ChecklistCard.svelte` and `ProgressBarCard.svelte` (rendered by `MessageItem.svelte`) share `ProgressMeter.svelte`; `CreateChecklistDialog.svelte` and `ProgressBarDialog.svelte` are lazy chunks opened by `ChatPane.svelte`. Rules live in `components/tracker-form.ts` (the forms' checks), `components/tracker-view.ts` (one-line summaries, outbox previews) and `lib/checklists.ts` (newest copy, ticks, counts, percentages). No new token: both cards reuse `component.poll.*`, so they read as siblings of the poll card. Icons: Phosphor `list-checks` and `gauge` (Fill).

## Brief

Clémence runs the release on Friday and wants the team to see what is left without asking: a list everyone ticks, with who did what. Bob is moving the photo archive and wants the channel to see how far it is without posting "40 %… 55 %…". The job: **share a list in ten seconds, tick in one click, and read progress at a glance.** Discord has neither; the owner asked for both, with their permission (#roadmap).

Assumption: checklists hold three to ten short items and are ticked over hours or days; progress bars move a handful of times.

## Flow and states

1. **Offer.** "Checklist" and "Progression" / "Progress bar" join the composer's "/" menu after "Sondage", found by "/checklist", "/todo", "/progression" or "/progress" in either language, only where the writer holds **Créer des checklists et des barres de progression** (computed channel permissions), and in conversations whenever they may write there. Like the poll, they write nothing: choosing one removes the "/query" and opens its dialog.
2. **Create a checklist.** "Créer une checklist": Titre (300 characters, counter always shown), Éléments (three rows to start, 200 characters each, counter from 180, "Ajouter" up to 25, a cross on each row while there is more than one, "Tu peux en ajouter encore 22." announced politely). Enter goes to the next row and adds one after the last; Enter never sends. Items may repeat. A too long text is said as it is typed; a missing title or no item only on "Envoyer la checklist", which then focuses the first field to fix. Blank rows are dropped.
3. **Create a progress bar.** "Créer une barre de progression": Libellé (300 characters), Valeur and Sur side by side (whole numbers, numeric keyboard on phones), "Sur 100, elle s'affiche en pourcentage." under them. It opens at 0 sur 100. Problems ("Un nombre entier de 0 à 5.") are said on "Envoyer", focus on the first field to fix.
4. **Send.** Through the outbox with a nonce, like a poll: the dialog closes, the list jumps to the present, the card shows as it will arrive with nothing pressable, and a refusal leaves it "Non envoyé." with "Réessayer" and "Supprimer".
5. **Read a checklist.** The poll card's frame (432 px at most): the `list-checks` glyph and the title, then the level ("3/5" beside it), then one row per item: the poll's checkbox mark and the text. A ticked row takes the lamp edge, its mark fills, its text is struck through, and under it "Cochée par Ada, 5 nov. 2026, 14:32" (the full date, so screen readers and phones get it without a tooltip).
6. **Tick.** A click, Space or Enter on a row ticks or unticks it at once, as the reader; the instance's answer then replaces the guess, with the real time. Rows are `role=checkbox` buttons with `aria-checked`, 44 px tall at least on every pointer, each its own Tab stop (a list, not a radio group). A second press while the first is on its way is ignored. A refusal undoes the tick and says why above the composer ("Impossible de cocher l'élément : …"), unless someone else's change arrived meanwhile, which wins. After a lost answer the client reads the instance's copy instead of guessing.
7. **Edit a checklist.** Its author, or a moderator ("Gérer les messages") in a server channel, sees "Modifier la liste" under the items. In editing, each item is a field (Enter or leaving it renames; blank or unchanged keeps it), with a trash button (none on the last item: "Une checklist garde au moins un élément."), and "Nouvel élément" with "Ajouter" at the end (disabled at 25). "Terminé" returns to ticking. Ticks survive renames.
8. **Read and update a progress bar.** The card: the `gauge` glyph and the label, the level and its figure ("40 %" out of 100, else "3/5"). Its author or a moderator sees "Mettre à jour", which opens the same dialog filled with the current values ("Mettre à jour la progression", "Enregistrer"); the card moves when the instance answers.
9. **Motion.** The level's fill eases to its new width over `motion.duration.slow` with the standard easing; under reduced motion the duration token is 0, so it jumps.
10. **Screen readers.** The level is a `progressbar` named by the card's heading, with `aria-valuenow`, `aria-valuemax` and `aria-valuetext` ("3 sur 5 faites", "40 %"). Screen readers do not announce a progress bar's new value on their own, so a polite live region beside it says the new value when it changes (never the first one: a page of history stays quiet).
11. **Elsewhere.** Neither has Edit, Forward or Copy text, and Up in an empty composer skips them. Reply lines, pins, notifications and dialog previews read "Checklist : {titre}" or "Progression : {libellé}"; a search slip shows them with their glyph. Without the right to write, a checklist's marks are inert (`aria-disabled`).

```
┌──────────────────────────────────────────────┐
│ ☑ Release vendredi                           │
│ ███████████████░░░░░░░░░░░░░░░░░░░  2/5      │
│ ┌──────────────────────────────────────────┐ │
│ │ ■ T̶a̶g̶                                     │ │
│ │   Cochée par Ada, 5 nov. 2026, 14:32     │ │
│ ├──────────────────────────────────────────┤ │
│ │ □ Build                                  │ │
│ └──────────────────────────────────────────┘ │
│ ✎ Modifier la liste                          │
└──────────────────────────────────────────────┘
```

## Decisions

- **The poll card's frame and tokens, no new ones.** The three cards are one family: the same well, rows, marks and level colours. The level is apricot (`component.poll.bar-mine`) because it is the channel's shared progress, the one thing on the card everyone looks at; a ticked row takes the lamp edge as your own poll answer does.
- **Who and when, in words, under the item.** A tooltip hides it from touch and screen readers; the line is short and only on ticked items.
- **No optimistic item edits.** Renames and removals are rare and their failures need the field back: the card waits for the instance's answer (a few tens of milliseconds).
- **One dialog for creating and updating a progress bar.** The same three fields; "n sur m" rather than a percentage mode switch, since "sur 100" says it.

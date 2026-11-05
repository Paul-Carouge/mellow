# Polls on the web (ROADMAP item 18)

Status: designed and built on `feat/polls-server` (ADR-0043: a poll is a message; votes are shared state on the instance), DESIGN.md v0.52. `PollCard.svelte` (rendered by `MessageItem.svelte`), `CreatePollDialog.svelte` and `PollVotersDialog.svelte` (lazy chunks opened by `ChatPane.svelte`), rules in `components/poll-view.ts` (results, leaders, time left, summaries), `components/poll-form.ts` (the form's checks) and `lib/polls.ts` (vote arithmetic), the shared minute clock in `components/poll-clock.svelte.ts`, tokens `component.poll.*` and `ref.apricot.bar-light` in `design/tokens.json`, and the Phosphor icon `chart-bar-horizontal` (Fill).

## Brief

Bob wants the group to pick a restaurant for Friday without twenty messages of "moi pizza". The job: **ask in ten seconds, answer in one click, and read the result at a glance**, as on Discord (ADR-0043 follows Discord's rules: up to ten answers, one or several choices, a duration, results hidden until you vote, the author ends it early, a permission to create them).

Assumption: most polls have two to five short answers and last a day; people vote from the list, rarely from pins or search.

## Flow and states

1. **Offer.** "Sondage" / "Poll" is a command of the composer's "/" menu, listed last (after the Markdown commands) and found by "/sond", "/poll" or "/vote" in either language. It shows only where the writer holds **Créer des sondages** in the channel (computed channel permissions), and in conversations whenever they may write there. It writes nothing: choosing it removes the "/query" (undoable) and opens the dialog.
2. **Create.** "Créer un sondage", with who will vote said under the title. Question (300 characters, counter "28/300" always shown, read as "28 caractères sur 300"), Réponses (two rows to start, 55 characters each, counter from 45, "Ajouter une réponse" up to ten, a cross on each row once there are more than two, "Tu peux encore en ajouter 7." announced politely), "Autoriser plusieurs réponses" (switch), "Durée" (1, 4, 8 or 24 heures, 3 jours, 1 or 2 semaines; 24 heures by default). Enter in the question goes to the first answer; Enter in an answer goes to the next and adds one after the last; Enter never sends.
3. **Check.** A repeated answer (case and outer spaces ignored, as the instance compares) is said under its later rows at once: "« samedi » est déjà une réponse."; a text past its limit too: "Au plus 55 caractères.". A missing question or fewer than two answers are only said on "Envoyer le sondage", which then puts focus on the first field to fix. Blank rows are dropped when sent.
4. **Send.** Through the outbox with a nonce (`client.sendPoll`), like any message: the dialog closes at once, the list jumps to the present and the poll shows as it will arrive, nothing pressable, "Envoi…" after the usual delay. A refusal leaves it "Non envoyé." with "Réessayer" and "Supprimer" on the row.
5. **Read, before voting.** The card: question, "Choisis une réponse" (or "une ou plusieurs"), the answers as radio or checkbox rows, and "12 votes · Se termine dans 3 heures" with "Voir les résultats" at the footer's end. Counts per answer stay hidden, as on Discord; the total does not.
6. **Vote.** A click, Space or Enter on an answer votes at once: a single choice moves its mark (one PUT, the instance takes the old vote back), a multiple one toggles. The row takes the lamp's edge and fill, results appear, and "Retirer mon vote" replaces "Voir les résultats". The instance's events for your own vote change nothing (counted once). A refusal puts everything back and says why above the composer: "Impossible de voter : …", or "Ce sondage est terminé, les votes ne comptent plus." when it ended between two clock ticks. A second click while one is on its way is ignored, so changes never cross.
7. **Results.** Each row fills from its leading side to its share of all votes, rounded; "2 votes" over "67 %" sit in a column at its end. When anyone voted for an answer, that column is a button: "Votes pour « Pizzeria du coin »", a dialog of avatars and names, 100 per page with "Afficher plus".
8. **Time.** "Se termine dans 12 minutes", "dans 23 heures", then days from 48 hours ("dans 3 jours"), always rounded down, "dans moins d'une minute" at the end; the exact date is the tooltip and the `<time>`. One clock for the whole page ticks once a minute while at least one open poll is on screen; a poll that ended stops watching it.
9. **End.** At its expiry, or when its author picks **Terminer le sondage** in the message menu (danger section, confirmed by "Terminer ce sondage ?", "Les votes s'arrêtent maintenant et tout le monde voit les résultats définitifs."). Ended: "Terminé", no hint, results for everyone, the leading answers (all of them on a tie, none without votes) semibold with a crown, "Réponse gagnante" for screen readers; the marks no longer respond.
10. **Elsewhere.** A poll has no Edit, Forward or Copy text, and Up in an empty composer skips it. Reply lines, pins, notifications, the delete, pin and end dialogs' previews read "Sondage : {question}"; a search slip shows the question with the chart glyph (search does not look inside polls yet, ADR-0043).
11. **Cannot vote.** Without "Voir les anciens messages" in a server channel, or blocked in a conversation, the card still reads, its marks are inert (`aria-disabled`) and "Retirer mon vote" is not offered.

```
Desktop (in the message column, 432 px at most)          Phone (320 px, full width of the column)
┌──────────────────────────────────────────────┐          ┌───────────────────────────┐
│ On mange où vendredi midi ?                  │          │ On mange où vendredi      │
│ Choisis une réponse                          │          │ midi ?                    │
│ ┌──────────────────────────────────┬───────┐ │          │ Choisis une réponse       │
│ │░░░░░░░░░░░░░░░ ○ Pizzeria du coin│2 votes│ │          │ ┌───────────────────────┐ │
│ │░░░░░░░░░░░░░░░                   │  50 % │ │          │ │░░░ ○ Pizzeria du  50 %│ │
│ ├──────────────────────────────────┼───────┤ │          │ │░░░   coin             │ │
│ │▓▓▓▓▓ ● Le thaï de la rue Paul-Bert│1 vote│ │ yours:   │ └───────────────────────┘ │
│ │▓▓▓▓▓                             │  25 % │ │ apricot  │  …                        │
│ └──────────────────────────────────┴───────┘ │ edge+fill│ 4 votes · Se termine dans │
│ 4 votes · Se termine dans 23 heures  Retirer │          │ 23 heures                 │
│                                    mon vote  │          │ Retirer mon vote          │
└──────────────────────────────────────────────┘          └───────────────────────────┘
```

Below `poll.compact-width` (360 px of card, so every phone up to 430 px), the results column keeps only the percentage, without its 56 px minimum (the voters button keeps the hit target, `size.hit-web-min` under a coarse pointer) and with `space.2xs` of inline padding, the answer's trailing padding also `space.2xs`; the vote count stays in the row's accessible description and the voters button's name (design pass WP6, 2026-11-05). At 320 px this gives an answer 111 to 128 px instead of 86 to 91: "Chacun ramène son plat, on partage en salle de réunion" goes from six lines to four, "Pizzeria du coin" from two to one.

## Design plan

- **Shape.** The card is the well every thing a message brings in uses (file card, link preview, GIF): `surface.sidebar` ground, 1 px `border.divider`, `radius.md`, 432 px at most. Answers are lifted back to the sheet's tone (`surface.chat`) so they read as pressable rows inside the well, 40 px tall (44 px under a coarse pointer), `radius.sm`.
- **Colour.** Dusk for everyone, apricot only for you (ADR-0022): your rows take the `accent.indicator` edge and an apricot fill (`ref.apricot.chip-dark` / the new `ref.apricot.bar-light`, apricot.300 at 50 % over white, since the mention chip is too close to white to read as a level); others' fills are dusk 340 / 890. The crown of the winners is `text.heading`, never apricot: winning is not "you". Marks are ringed in `text.muted`, which stays 3:1 on every fill in both themes (`border.strong` does not on the light fill).
- **Type.** Question `type.heading-md`, answers `type.message` (the size of the message they replace), counts `type.body-sm` `text.secondary` (muted fails on the light fill, 4.36:1), percentages `type.body` semibold `text.heading`, all tabular.
- **Motion.** One moment: when results appear, each fill grows from nothing in `motion.duration.slow` with the standard curve, and follows each vote after; under reduced motion the duration token is 0. Nothing else moves.
- **The signature.** The row *is* the bar. Rejected: a thin progress bar under each answer in the accent (a chart to decode, and apricot everywhere would claim every vote as yours), and Discord's look of a blurple wash on the chosen answer only.
- **Entry point.** The "/" menu over a sixth composer button (the row already has attach, GIF, emoji and, on phones, send; at 320 px the field would lose its room) and over turning "+" into a menu (one click to files is a habit worth keeping). Its cost: the poll is less visible to people who never type "/". If owners ask, "+" can become a menu with "Joindre des fichiers" and "Créer un sondage" later.

## Accessibility

- A single-choice poll is a `radiogroup` named by the question and described by its hint, one Tab stop (your answer, else the first); arrows move between answers **without voting** (a vote is a request, and arrows would send one per press); Space or Enter votes. A multiple-choice poll is a `group` of `role=checkbox` buttons, each a Tab stop. `aria-checked` carries your choice; colour never does alone (the mark fills, the edge changes).
- With results, each answer is described by "2 votes, 67 %"; the count button is named "2 votes, 67 %, voir qui a voté pour « Pizzeria du coin »", its visible text first. The winner is told by "Réponse gagnante", not only by the crown and weight.
- Targets: rows 44 px under a coarse pointer, the footer's text buttons 24 px on fine pointers and 44 px on coarse, the dialog's cross buttons 44 px.
- The dialog's messages live in `role=status` paragraphs referenced by each field's `aria-describedby`; fields with a problem are `aria-invalid` and edged in `component.dialog.error-text`; counters have a spoken form.
- Contrast: seventeen pairs added to the generator (answer text, counts and marks on the row and both fills, the checked mark's glyph, your edge, the hint and the question on the card), all passing in both themes.

## Checked

Rendered in Chrome (Playwright, the debug server built from this branch and Vite on 5173) in dark and light, French and English, at 1280 and 320 px: results hidden then shown on a vote, a move of a single choice by keyboard (ArrowDown moved focus only, Space voted), "Retirer mon vote", the voters dialog, "/sond" in the menu, the dialog empty, with the "send" errors, with a repeated answer, filled (multiple, 3 jours) and sent, the menu's "Terminer le sondage", its confirmation and the ended card.

## Not done

- Search inside polls, checklists and progress bars (ROADMAP item 18's other parts), anonymous polls.
- Showing who voted in a tooltip on hover, as reactions do: the count button opens the list instead.
- An exact end tick: the shared clock moves once a minute, so a poll can look open up to a minute after its expiry; the instance refuses the vote and the card says so.

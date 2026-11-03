# Forwarding messages on the web (ROADMAP item 18, ADR-0039)

Status: designed and built on `feat/forward-messages`, DESIGN.md v0.46. Web client: `ForwardMessageDialog.svelte` (loaded lazily from `ChatPane.svelte`), the forward block in `MessageItem.svelte`, rules in `forward-messages.ts`, the menu item in `context-menus.ts`, `MellowClient.forwardMessage` and `sendMessageTo`, `RestClient.forwardMessage`. The server, protocol and migration are ADR-0039's. No new token; one new icon, Phosphor `arrow-bend-up-right` (Fill), copied into `design/icons`.

## Brief

People in small communities pass things along: Ana posts a screenshot of the new home page in "Rust Lyon" #général, and Paul wants it in #annonces and in his conversation with Bob, with "Regarde ce rendu, c'est propre." under it. Today he copies the text and loses the picture. The job is "send this message somewhere else, as it is, and say where it came from", in a few seconds and without leaving the conversation.

The instance makes a forward a **snapshot** (ADR-0039): a new message by the forwarder holding a copy of the original's words and files, plus the original's channel and message ids. It never mentions anyone and cannot be edited.

## Flow and states

1. **Start**: "Transférer" in the message menu (right-click, long press, More, Shift+F10), after "Répondre", and as an icon in the hover bar after Reply. Every sent message has it, read-only channels included (passing an announcement on is the common case); pending and failed messages have no menu, as before.
2. **Dialog "Transférer le message"**, focus in the search field:
   - The places the reader may post the forward in: conversations first (most recent first, as the navigator lists them), then each server's text channels in the sidebar's order. Left out: voice channels, channels without `SEND_MESSAGES` (and without `ATTACH_FILES` when the message has files), one-to-one conversations with someone you blocked. The instance decides anyway.
   - Search: every word typed, in any order, against the place's name, its server, and a conversation's people and usernames; case and accents ignored (`foldForSearch`); a leading `#` or `@` is ignored. Enter after typing picks the first match and clears the field; Enter with nothing typed forwards.
   - Up to 5 places, native checkboxes; the counter under the field says how many more ("Tu peux encore en choisir 3.", "C'est le maximum d'endroits pour un transfert."), and the other boxes are disabled at 5.
   - The rows keep the order they had when the dialog opened: a forward sent moves its conversation to the top of the navigator, and the rows being picked must not move.
   - Preview: the forward as it will arrive (the bar, "Transféré", the text clamped to 160 px, "1 pièce jointe"), never the original author, since the forwarder sends it.
   - "Ajouter un message (facultatif)": sent as an ordinary message in each place right after the forward.
3. **Sending**: each place on its own, in parallel; the forward, then the note. Each place keeps one nonce for the forward and one for the note for the dialog's life, so trying again after a lost answer returns the message already made instead of a copy; a place whose forward landed only resends its note (no extra write against the instance's rate limit). The selection and the search's Enter are frozen while sending. Your own forward and note leave a destination you had read still read (the instance never moves the sender's read position), while others' unseen messages there stay unread.
4. **Success everywhere**: the dialog closes and the chat's status line above the composer says "Message transféré dans #annonces et Bob." (read out politely); it goes at the next channel or the next message sent. Focus goes to the composer, or back to the message's More button or row in a chat where you cannot write.
5. **Some places refused**: the dialog stays. Places that worked leave the selection, so "Transférer" again only sends what is missing; a quiet line with a check says "Message transféré dans #général."; the refused rows keep their box and replace their second line with the reason (red, warning glyph); the alert holds one sentence per place: "Impossible de transférer dans #entraide : tu ne peux plus y envoyer de messages ou de fichiers." (`MISSING_PERMISSIONS`), "…: le message d'origine a été supprimé, ou tu ne peux plus le lire." (`NOT_FOUND`), or the generic reasons; a note that failed after its forward reads "Le message a été transféré dans Bob, mais le tien n'a pas pu être envoyé : …".
6. **Nowhere to send**: "Tu n'as encore aucun endroit où envoyer ce message. Rejoins un serveur ou commence une conversation, puis transfère-le." with Cancel alone. **No match**: "Aucun salon ni aucune conversation ne correspond à « zed ». "
7. **Loading**: the dialog is its own chunk (`LazyComponent`), with the shared "loading part" notice and the reload offer when the chunk is gone after an update.

## A forward in the list

```
(P) Paul  Aujourd'hui à 21:55
    ┃ ↱ Transféré                         12 px, text.muted, arrow-bend-up-right 16 px
    ┃ Voilà le rendu de la nouvelle page   the copied words, rendered as any message
    ┃ d'accueil, @paul tu en penses quoi ?
    ┃ ┌────────────┐
    ┃ │  picture   │                      the copied files, same previews and viewer
    ┃ └────────────┘
    ┃ #général · Aujourd'hui à 21:54       footer: a button to the original (12 px, text.muted,
    Regarde ce rendu, c'est propre.        text.secondary and underlined on hover)
    [reactions]                            outside the bar: they belong to the forward
```

- **The bar** is `component.message.quote-bar-*` (4 px, `border.quote`), the same plum line as Markdown quotes and the reply spine: in Mellow, that line always means "these words come from elsewhere". The block starts 2 px under the author line and keeps 4 px between its parts, 12 px from the bar.
- **The mark** "Transféré" is in the timestamp's size and colour, never italic and never apricot: apricot is "you, now" (ADR-0022), and a forward is neither.
- **The footer** names the original's place and date: "#général" for a server channel, the conversation's name at Home, then the date the original's id carries, in the timestamp format. When the reader knows that channel it is a button, "Aller au message d'origine dans #général, mardi 3 novembre 2026 à 21:54", which opens the channel at the original exactly as a search result does (older pages load up to 1 000 messages back, the message is centred, focused and lit; "too far", "missing" and "failed" say so above the composer). When the reader does not know the channel, the footer is the date alone, read as "Message d'origine du …"; the ids reveal nothing else (ADR-0039).
- **No ping**: a forward never takes the "mentions you" tint or bar, even when its words hold `@paul`; mention chips inside still render as chips.
- **Actions**: Reply, reactions, Pin and Delete as on any message, Forward again (the instance keeps the first origin); no Edit, in the bar, the menu or with Up in an empty composer, which now goes to your last message that is not a forward.

## Design plan

- **Palette**: no new colour. Bar `border.quote`; mark and footer `text.muted` (5.8:1 or more on the sheet and the hover wash in both themes), footer hover `text.secondary`; row refusals `component.dialog.error-text` (4.68:1 on the light row hover, 6.4:1 dark).
- **Type**: `type-message-timestamp` for the mark and footer, `type-message` for the copied words, the picker's `type-member-name` and `type-body-sm` in the rows.
- **Shape**: people are round (avatars, group avatars), places are square: a channel row leads with a `radius.md` tile on `neutral.fill` holding the `#` glyph, the shape of a server in the strip.
- **Dialog**: `ModalDialog` (440 px, the people picker's 48 px rows, `people-picker.*` list tokens, 320 px list scrolling inside); on a phone the same dialog fills the width and scrolls as a whole.
- **Motion**: none of its own; the dialog's settle entrance, the jump's highlight fade.

## Defaults rejected

- **Discord's grey italic "Forwarded" over a box**: Mellow's type scale has no italic role, and a box around a forward would be a card inside the message sheet; the bar alone carries "from elsewhere", as quotes and replies already do.
- **Chips for the chosen places** (the people picker's pattern): places are rows with their own checkbox and the counter keeps the total; chips would show every choice twice.
- **Headings per server in the list**: they fragment short search results; each channel row says its server instead.
- **A toast**: Mellow has none, and the chat's status line above the composer already speaks for the conversation.
- **Apricot for the forward mark or the success line**: apricot stays for "you, now".
- **Hiding Forward in read-only channels**: that is where announcements live, the thing people pass on most.

## Accessibility

- The dialog is a native modal `<dialog>` (focus kept inside, Esc and Cancel close it except while sending, focus back to the opener on cancel, to the composer after success). The list is labelled "Destinations"; each row is a `<label>` around its native checkbox (Space toggles, the name and server are its accessible name); the counter is the search field's description and a polite live region; refusals are in the dialog's alert, successes in a status line.
- 48 px rows; the footer button is 24 px tall with a fine pointer and 44 px under a coarse one, like the outbox's Retry; the hover bar's Forward is 32 px like its neighbours, and the menu item is the keyboard path.
- Checked at 1280 × 860 in both themes and at 390 × 844 (dark) in headless Chrome against a local instance.

## Not done

- The pins panel and the search results render a forward's words without its bar, mark and footer.
- A forward's notification (when it lands in your conversation) reads like any message; it never pings, since the instance gives forwards no mentions.
- A forward of a message whose files the destination cannot take is filtered out client-side only from the permission snapshot; a change between opening and sending is reported as a refusal.

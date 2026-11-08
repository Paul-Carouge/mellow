# Forwards, link previews and provider cards on the desktop (M3 step 5, ADR-0039, ADR-0041, ADR-0044)

Status: built on `feat/desktop-forwards-and-link-previews`. It ports `docs/design/web-forward-messages.md`, `web-link-previews.md` and `web-provider-embeds.md` to the GPUI client. The rules live in client-core: `shared_logic::forward_messages` (places, search, opening order, origin place, nonces per place) and `shared_logic::link_previews` (layout, YouTube links, pill colour, compact counts, Linguist colours, issue badges, wiki language names), with the web's tests ported. The views live in `messages/link_previews.rs`, `messages/forward.rs`, `conversation/forward_dialog.rs` and `conversation/forwarding.rs`. Requests outside the outbox are in `session/forwards.rs`. No new token, icon or string: every word comes from the web's catalogue.

## What differs from the web, and why

- **No player is embedded.** The desktop has no web view, and ADR-0041's promise ("nothing reaches the provider before a press") holds without one. The YouTube poster keeps the web's 16:9 box (9:16 for a Short), its 56 px play key and its "Starts at 1:30" chip. Pressing the key opens the link in the system browser. Since the link carries `t=` and `list=`, the video starts where the message says. YouTube Music's "Listen" does the same. "Close player", "Hide player" and the offline line do not exist on the desktop, because nothing plays inside it.
- **Every address goes through `open_message_link`**, the opener the messages already use, which only accepts http and https. Titles, pictures, the play key and Listen all open `preview.url`. None of them builds an address from embed ids.
- **Pictures come from the instance**, through the picture pipeline (`DownloadableFile::LinkPreviewImage`, decoded for the box). Each box is sized before the picture loads. When a picture fails: the poster stage and the cover placeholder stay, the GitHub owner falls back to their initial, and a thumbnail or a link that is only a picture leaves an empty placeholder.
- **The success line** ("Forwarded to #news and Ana.") uses the web's composer notice: a quiet `text.secondary` line above the composer, also read out through the conversation's polite live region. It goes away at the next channel or the next message sent.
- **The jump to the original** works as a pin's jump in the same channel. For another channel, an `OpenForwardOrigin` action goes to the shell. The shell opens that channel and asks the conversation to reach the message once it is applied.

## Tokens and layout

The desktop adds no tokens of its own. It uses `component.link-preview.*` for the cards (432 px at most, pill, thumbnail of 80 px, cover of 96 px, owner avatar of 40 px, badges, topics, stats) and `component.message.quote-bar-*` with `text.muted` for a forward's bar, mark and footer. The dialog reuses the people picker's tokens (48 px rows, 320 px list, checkbox) inside `ModalDialog`. A channel row starts with a square `radius.md` tile showing a `#` on `neutral.fill`. People keep round avatars, and a group uses `GroupAvatar`.

## Accessibility

- Each card has one Tab stop, its title, with the role Link. Pictures repeat the link for the pointer only. A picture-only preview is a single link named "Picture from {host}". The play key is a button covering the whole poster, named "Play YouTube video: {title}, starting at 1 minute 30 seconds". Its focus ring is drawn inside the stage in `play-button-focus-ring`.
- Compact counts ("104K") speak the exact number ("104,238 stars"), both in their accessible name and in their tooltip. A card's title and pictures show the link's full address in a tooltip before it opens, since the card's site name and title are the page's own. An issue's state always shows a word and a glyph, and draft is the only outlined badge.
- In the dialog, each row is a checkbox (role, toggled state and name). A full selection appends the counter sentence to the name of each disabled row, and a refusal appends its reason. The counter is a status region. Esc closes the dialog unless it is sending.
- Nothing animates beyond the existing hover colours.

## Defaults rejected

- **An embedded web view for players.** It would bring a browser engine into a client meant to stay native and light (ADR-0051), along with the page's whole security surface. The system browser already plays the video, with the reader's own settings and sign-ins.
- **Hiding the YouTube poster and showing only a link.** The poster tells the reader what the link is at a glance, as on the web, and the instance already stores it.

## Not done

- Spotify, Deezer, SoundCloud, Apple Music, Bandcamp, Vimeo and Twitch get the plain card, as on the web, which has no card for them yet either.
- Wikipedia names only the 16 largest wikis' languages, because the web's `Intl.DisplayNames` has no Rust equivalent here. Any other wiki shows its language code, as the web's own fallback does.
- Previews and forward frames in the pins panel and in search, which the web does not have either. A per-message setting to hide previews does not exist on the web.
- Not looked at signed in on screen. Only the component gallery (`examples/gallery_picture.rs`, light and dark) has been rendered, with no instance behind it, so the poster showed its empty stage.

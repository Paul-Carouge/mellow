# Desktop composer (M3 step 3b)

The GPUI composer (`crates/mellow-desktop/src/composer`) ports the web's `Composer.svelte` and follows DESIGN.md §5.5 with the existing tokens only (`component.composer-*`, `component.context-menu-*`, `component.custom-emoji-suggestion-*`, `component.action-bar-*`, `component.message-editor-hint-*`). No new token, colour or size.

## Layout
```
            ╭ Emoji matching :thu ─────────╮   suggestions: deferred, above the box, 320 max
            │ 👍 thumbs                     │   context-menu rows, active row filled
            ╰───────────────────────────────╯
 Messages are at most 4,000 characters…           error line (role alert), composer.error-text
╭───────────────────────────────────────────────╮
│ Replying to Ana                          (x)  │ reply bar, or "Edit message · Esc to cancel · Enter to save"
│ [img][file][file]                             │ tray, scrolls sideways, divider below
│ (+)  Message #general                    120  │ base row = size.dock.height (56), counter near the limit
╰───────────────────────────────────────────────╯
```
The box is `component.composer-radius` on `component.composer-background`, 16 px side margins, `size.dock.margin` below, so its base row lines up with the user card. The focus ring shows only after keyboard input, as on the web.

## Behaviour
Enter sends, Shift+Enter writes a new line, the field grows up to half the window then scrolls. Up in an empty field emits `EditLast`; Esc closes suggestions, then the edit, then the reply, and otherwise emits `Cancel(Nothing)` for the chat to mark the channel as read. Files come from the attach button (native picker), a drop on the box, or pasted copied files.

## Deliberate differences from the web, for the next wave
- Editing happens in the composer's own field with an edit bar, not inline in the message row (DESIGN.md §5.5 "inline"): the chat pane may move it into the row later; `MessageEditor` already holds the refused-save rules.
- The typing row above the composer, the send button setting, the GIF and emoji buttons, `Ctrl+E`, the formatting shortcuts, the "/" menu, server emoji, `@everyone` and role mentions are not there yet (step 5).
- The reply bar has no "@ On/Off" toggle: the protocol has no field for it, and the web has none either.
- A pasted picture (a screenshot, not a copied file) is pasted as nothing; videos upload as they are (compression is step 9).
- Upload results go to a status node; GPUI has no polite live region yet, so screen readers may not speak them. The suggestion count is not announced, and the list is not tied to the field as an active descendant (GPUI needs the list inside the focused element for that).
- Files are read at most up to the instance's limit (25 MiB, the server default, when `Ready` states none). The tray preview is decoded from those bytes by the picture pipeline (AVIF included), scaled to the card, and its atlas slot is freed when the file leaves the tray. Files chosen for a channel that is no longer open when they arrive are dropped.

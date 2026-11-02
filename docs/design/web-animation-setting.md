# Animated pictures setting on the web (ROADMAP item 17)

Status: built on `feat/web-always-animate-setting`, DESIGN.md v0.34. Web only: no server, protocol, token or migration change.

## Brief

The owner's idea box: "Custom emoji, Melli and GIFs do not animate under ad blockers and on Firefox; add a setting 'Always animate' next to today's 'animate on hover'." A friend on desktop Chrome sees his GIF avatar as a still picture, while it plays on his phone.

Cause found in the code (not reproduced on those machines): every animated picture and Melli were forced still whenever the browser reports `prefers-reduced-motion: reduce`. Chrome and Firefox report it on Windows with "Animation effects" off (also turned off by some performance presets and remote desktop sessions) and on KDE or GNOME with animations reduced, so those people never saw a picture move, hovered or not, and had no way to change it in Mellow. Picture URLs are neutral (`/api/v1/profile-images/{id}`, `/still`, `/api/v1/icons/{id}`) and animated pictures are animated WebP (ADR-0033), which every target browser plays: no ad-blocker-specific cause was found.

## Decision: who wins over reduced motion

| Choice | System reduced motion | Avatars, server icons | Custom emoji, banners, profile card, icon previews, Melli |
|---|---|---|---|
| Animer au survol (default) | off | play on hover or focus | play |
| Animer au survol (default) | on | still, hovered or not | still |
| Toujours animer | off or on | play | play |

- **The explicit in-app choice wins, for pictures and Melli only.** It is the reader's own decision made in Mellow, and the only way out for people whose system reports reduced motion without their asking. UI motion (sheets, menus, dialogs, smooth scrolling, the rail morph, the unsaved-changes shake) keeps following the system alone.
- **With "Animer au survol", hover does not override reduced motion.** The brief suggested letting hover animate even under reduced motion, since hover is a gesture. Rejected for an accessibility reason: message, member and conversation rows animate their avatar when hovered, and the pointer crosses those rows constantly while reading, so the hover is incidental, not a request for motion. People with vestibular disorders who set their system to reduce motion would start seeing motion without touching anything in Mellow. The default therefore keeps exactly today's behaviour.
- **No "Never" choice.** With the default choice, the system's reduced motion already stops every picture; and nobody asked for still pictures without it (YAGNI). Two choices, as the owner named them.

The rule is one pure function, `shouldPictureAnimate(choice, isReducedMotionPreferred, isPlayingWithoutChoice)` in `web/src/lib/picture-animation.ts` (tested for every combination). `isPictureAnimationWanted(isPlayingWithoutChoice)` in `web/src/lib/motion.svelte.ts` feeds it the stored choice and the media query; `Avatar`, `ProfileCard`, `ServerRail`, `CustomEmojiImage`, `IconEditor`, `IconCropDialog` and `Melli` ask it instead of reading `prefersReducedMotion`. `isPlayingWithoutChoice` is what the picture does by itself: hover or focus for avatars and server icons, `true` for the others.

## Screen

User settings → App settings → **Accessibilité** (first in the group, as on Discord, and where DESIGN.md §5.9 planned "Play animated images"). Phone: a page of its own like the other sections.

```
Accessibilité                                              (x)
Ce réglage reste dans ce navigateur.                       Échap

Images animées
┌───────────────────────────────────────────────────────┐
│ (o) Animer au survol                                  │
│     Les avatars et les icônes de serveur s'animent…   │
└───────────────────────────────────────────────────────┘
┌───────────────────────────────────────────────────────┐
│ ( ) Toujours animer                                   │
│     Avatars, icônes de serveur, emoji personnalisés…  │
└───────────────────────────────────────────────────────┘
Ton appareil demande de réduire les animations : avec
« Animer au survol », les images et Melli restent fixes.   (only while the system asks)
```

- Reuses `ChoiceOption` (6.55) in a fieldset with its legend, the Voix page's spacing tokens (`settings.section-gap`, `choice.list-gap`) and `text-field.helper-text` for the note: no new token, no new component except the page.
- The note explains a still avatar where the reader goes to fix it; it is plain text inside the fieldset, so screen readers read it with the choice. It is not a live region: it only changes when the system setting changes.
- The choice applies at once in the tab and is stored as `mellow.picture-animation` (`"on-hover"` or `"always"`). Anything else, or an unreadable storage, gives the default; a storage that refuses writes keeps the choice for the tab. Other open tabs pick it up on their next load.

## States

There is no loading, empty, error or offline state: the setting is local and synchronous. Long content: the descriptions wrap inside the rows at 360 px and at 200 % text.

## Rejected defaults

- A switch "Always animate" (on/off): rejected for two named choice rows, because the owner named two behaviours and each needs its sentence saying what moves.
- A live preview with a sample animated avatar: removed; the reader's own avatar and the rail answer at once behind the dialog, and a preview would itself animate under reduced motion.
- An "Appearance" page holding only this: the owner's report is about motion and accessibility, and Discord keeps its animated-media switches in Accessibility.

## Verified

Headless Chrome over CDP against a local instance built from the branch, with `Emulation.setEmulatedMedia` `prefers-reduced-motion: reduce` and an animated GIF avatar: by default the user card's avatar uses `/still`, also while hovered, and Melli has `is-still`; after choosing "Toujours animer" on the page, the avatar uses the animated URL at once and after a reload, and Melli plays. Screenshots in French and English, dark and light, 1280 px and 360 px. Not verified: Firefox, Safari, a real Windows machine with "Animation effects" off.

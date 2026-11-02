# Web client: framing the avatar and the banner

DESIGN.md §5.9 (Profile) and §6.54 (the crop dialog) on the web client, ROADMAP item 17: "Avatar and banner upload: crop inside the dialog while keeping each one's aspect ratio (the square crop dialog exists for icons)." The server part is `feat/server-crop-avatar-and-banner` (#175): `PUT /api/v1/users/@me/avatar?crop_x&crop_y&crop_size` (`IconCropQuery`, a square) and `PUT /api/v1/users/@me/banner?crop_x&crop_y&crop_width` (`BannerCropQuery`, the height being `width × 384 / 960`); without a crop the instance keeps the largest centred square or 5:2 band, and it cuts every frame the same way, so animations stay animated.

## Brief

- **Who**: anyone setting up their profile, often with a phone photo (portrait, 3:4) or a wide screenshot, sometimes an animated GIF.
- **Job**: choose which part of the picture becomes the avatar circle and the profile card's band, before it is sent, and see it where it will live.
- **Rule kept from the icons**: the browser never re-encodes the picture. The file goes as chosen with the crop in the query; the instance cuts, shrinks and compresses (ADR-0033), so an animated GIF stays animated and a photo is compressed only once.

## Flow and states

```
Profile ── Change avatar ─> file picker ─> Frame your avatar (circle) ─> Save avatar ─> "Avatar saved."
        └─ Change banner ─> file picker ─> Frame your banner (5:2 band) ─> Save banner ─> "Banner saved."
```

| State | What shows |
|---|---|
| Not a picture, over 8 MB | Refused before the dialog, under the editor, as before. |
| Opening | "Opening the picture…" on the stage; Save disabled. |
| Unreadable | The dialog says so; Save stays disabled. |
| Framing | The stage, the zoom slider and "Centre it", then the samples at their real size. |
| Uploading | The dialog is busy and its button reads "Uploading…"; the editor's button reads "Uploading avatar…", announced through the page's status region. |
| Refused by the instance | In the dialog, which keeps the crop, so a retry needs no new pick. |
| Saved | The dialog closes; "Avatar saved." / "Banner saved." under the editor; the preview card, the user panel and every row update from the answer. |
| Reduced motion with "Animer au survol" | An animated picture shows its first frame in the dialog; the file sent is still the animated one. |

## Design decisions

- **One dialog, three shapes.** `PictureCropDialog` (formerly `IconCropDialog`) takes `kind`: `server` and `channel` keep a square, `avatar` a square shown as a **circle**, `banner` the **5:2 band** of the profile card (960 × 384 stored, 300 × 120 shown). The maths (`picture-crop.ts`) works on `{ x, y, width }` and an `aspect` (width over height); the height is derived, so a crop of the wrong shape cannot be built.
- **The avatar frame is a circle, the stored picture a square.** Avatars only ever show round (`Avatar.svelte`), so the frame and the dimmed outside follow the circle: what is inside the ring is exactly what people will see. The corners are kept by the instance but never shown.
- **The stage takes the shape.** The frame is in flow with the kept part's aspect ratio and the stage pads it by `component.icon-picker.frame-inset`, so a square stage is unchanged and a banner's stage is a wide band, not a square with a thin strip in it.
- **Samples where the picture lives.** Avatar: 80 px (profile card) and 40 px (beside messages), round. Banner: the profile card itself (`ProfileCard` with a `bannerPicture` snippet), with the avatar ring covering the band's lower left, so a face is not framed under the avatar by accident. On a card narrower than 300 px the band keeps its shape and the strip cuts its sides, as the real card's `object-cover` does.
- **Rejected defaults.** A separate avatar cropper (a second component to keep in step); cropping on a canvas in the browser (it would flatten animated GIFs and compress photos twice); a square frame for avatars (it would promise corners nobody sees); a generic "Crop image" title (each kind says what it frames: "Frame your avatar", "Frame your banner").

## Copy

| Key | English | French |
|---|---|---|
| `profile.avatar.crop.title` | Frame your avatar | Cadre ton avatar |
| `profile.avatar.crop.subtitle` | Drag the picture and zoom until the circle holds what you want to show. The rest is cut away. | Fais glisser l’image et zoome jusqu’à ce que le cercle contienne ce que tu veux montrer. Le reste est coupé. |
| `profile.avatar.crop.save` | Save avatar | Enregistrer l’avatar |
| `profile.avatar.crop.sampleProfile` / `sampleMessages` | In your profile / Beside your messages | Dans ton profil / À côté de tes messages |
| `profile.banner.crop.title` | Frame your banner | Cadre ta bannière |
| `profile.banner.crop.subtitle` | Drag the picture and zoom until the band holds what you want to show. The rest is cut away. | … jusqu’à ce que la bande contienne … |
| `profile.banner.crop.save` | Save banner | Enregistrer la bannière |
| `profile.formats` | … You frame them once they're open, and animated GIFs stay animated. | … Tu les cadres une fois ouvertes, et les GIF animés restent animés. |

The stage, hint, zoom, "Centre it", "At its real size", "Opening the picture…" and "Uploading…" are the icon dialog's strings, shared.

## Tokens

No new token. The stage, frame and scrim use `component.icon-picker.*`; the samples use `component.profile-popover.avatar-size`, `component.message.avatar-size` and the profile card's own tokens. The banner's 5:2 is `BANNER_ASPECT` in `picture-crop.ts`, the instance's stored size (960 × 384), not a design value.

## Code

| Piece | Files |
|---|---|
| Crop maths (tested) | `web/src/components/picture-crop.ts`, `picture-crop.test.ts` |
| Dialog | `web/src/components/PictureCropDialog.svelte` |
| Profile editors | `web/src/components/ProfileSettings.svelte`, `ProfileCard.svelte` (`bannerPicture` snippet) |
| Query | `pictureCropQuery` in `web/src/lib/api.ts`, `RestClient.setProfileImage(kind, file, crop)` |

# 0042. GIFs from Klipy, loaded by members' browsers

- Status: Accepted
- Date: 2026-11-05
- Amends: [ADR-0019](0019-web-client-served-by-the-server-binary.md) (the Content-Security-Policy gains Klipy's hosts when GIFs are on), [ADR-0041](0041-link-previews-fetched-by-the-server.md) (Klipy's links are never fetched by the server)

## Context
ROADMAP item 18 asks for "a GIF service with favourites", as on Discord. Mellow has no GIF catalogue; Tenor, Discord's, is closing its API in 2026. The owner chose Klipy (2026-11-04), first relayed by the server so that members would never contact it. Klipy's integration requirements (docs.klipy.com, read 2026-11-04) forbid exactly that without their written approval: API requests and media loads must come from the user's app or browser, media must be loaded from the returned URLs without copies, proxies or caches, results must be shown in Klipy's order without filtering, and the picker must credit Klipy ("Search KLIPY" in the search field, "Powered by KLIPY"). The owner then chose Klipy loaded by members' browsers (2026-11-04), accepting that Klipy sees the address of every member who searches for or views a GIF.

## Decision
- **Off unless the owner sets a key**: `[gifs] klipy_app_key = "…"` in `mellow.toml`. Without it, nothing about GIFs reaches clients, the policy is unchanged and the picker is absent. Klipy app keys are public by design (they travel in every request URL); the instance still sends it only to signed-in members.
- **The browser talks to Klipy**: `GET /api/v1/gifs/settings` gives a signed-in member the key and an opaque `customer_id`; the web client calls `https://api.klipy.com/api/v1/{key}/gifs/search|trending|items` itself and shows the media from `static.klipy.com`, `static1.klipy.com` and `static2.klipy.com`. The Content-Security-Policy gains those origins in `connect-src`, `img-src` and `media-src` only when a key is set.
- **`customer_id` reveals no account**: an HMAC-SHA-256 of the user's id under a key derived from the master key (HKDF, its own label), so Klipy can keep a member's recents without learning their Mellow id or linking them across instances.
- **Sending a GIF sends its link**: the message's text is the media URL Klipy returned (the medium WebP, or GIF when absent). Clients show a message whose text is only a Klipy media URL as the GIF itself, sized from the URL's response, and keep the link for older clients. The server's link preview fetcher never fetches Klipy's hosts (ADR-0041 would otherwise copy their media). The web client calls Klipy's share trigger when a GIF is sent, as their API asks.
- **Favourites are Mellow's**: a member stars a GIF in the picker or in a message; Mellow stores its Klipy slug (at most 200 per member, `gif_favourites`, migration 0037), never its media, and the picker loads them back through Klipy's items endpoint.
- **Results as Klipy sends them**: in their order, without filtering. Content rating and ads are set on the key in Klipy's partner panel; the owner sets the key without ads. An ad object, if one came anyway, is shown as a plain tile linking to its target, in its place.

## Consequences
- Klipy sees the address of each member who opens the picker or views a message with a GIF, as Tenor does for Discord. Mellow tells nothing else: no account, no name, no message.
- A Klipy test key allows about 100 requests per hour for the whole instance; a real community needs a production key from Klipy's partner panel.
- GIFs stop showing if Klipy removes them or stops serving the instance's key; messages keep their links.
- Older clients show the link, which their own previews do not fetch (Klipy's hosts are skipped).

## Alternatives considered
- **Relaying through the server** (the owner's first choice): hides members from Klipy, but breaks Klipy's terms without their written approval; possible later if Klipy approves it.
- **Tenor**: its API is closing in 2026.
- **Only GIFs already sent on the instance**: no third party at all, but no search, which is what was asked.

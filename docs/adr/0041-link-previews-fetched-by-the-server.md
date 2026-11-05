# 0041. Link previews fetched by the server

- Status: Accepted
- Date: 2026-11-04
- Amends: [ADR-0019](0019-web-client-served-by-the-server-binary.md) (the web client's Content-Security-Policy gains one `frame-src`)

## Context
ROADMAP item 18 asks for link previews (plain pages, YouTube, Spotify, YouTube Music, Deezer) and YouTube videos playable in the app, with `youtube-nocookie.com` embeds by default (an ad-free player goes against YouTube's terms and waits for the owner's decision and its own ADR). A preview needs the page's title, description and picture. Someone has to fetch the page: if every reader's browser did, each would reveal its address to the site and need the site in the policy's `connect-src` and `img-src`; and the browser cannot read another site's page anyway (CORS). The server can, once, for everyone, but then any member can make the instance send requests where they choose, including to the instance's own network (server-side request forgery). Mellow has made no outbound HTTP request so far, apart from ACME.

## Decision
- **The server fetches, after the message is stored.** On send and on edit, the server finds up to 3 links in the text (`http` and `https`, outside code, not written `<https://…>`, which keeps a link without preview as on Discord) and queues them. Sending never waits for a fetch: when previews are ready the server publishes `MESSAGE_LINK_PREVIEWS_UPDATE` (channel, message, previews) to the message's audience, and `Message.link_previews` carries them in history. Forwards copy no preview. An instance can turn the feature off (`[link_previews] enabled = false` in `mellow.toml`; on by default).
- **What a preview holds**: the link, the page's title, description and site name (Open Graph, then Twitter cards, then `<title>` and `meta description`, each cut to a bounded length), the page's colour (`theme-color`), and one picture. A link that is itself a picture becomes a preview of that picture. Text is plain: clients never render it as HTML or Markdown.
- **The picture follows the picture rules**: fetched with the same guard, at most 8 MiB, only PNG, JPEG, GIF or WebP by their bytes, its first frame only (previews count in no quota), fitted to the attachment preview box by the visually lossless pipeline (ADR-0033, metadata stripped), sealed in the sharded file store (ADR-0034) and served by `GET /api/v1/channels/{channel_id}/messages/{message_id}/link-previews/{preview_id}/image` with the attachments' permission check (`VIEW_CHANNEL` and `READ_MESSAGE_HISTORY`), an ETag and `private, no-cache`. Readers never contact the site.
- **One fetch per link per day**: previews are cached by the link's SHA-256 (`link_previews`, migration 0036) for 24 hours, a failure for 1 hour; messages point at them (`message_link_previews`). Unlinked previews go with the periodic purge 10 minutes after they expire (a job may be about to link one it found fresh), their pictures with the orphan sweep. A preview's id is random, not a Snowflake, and its picture's entity tag uses that id: the cache is shared by every channel, and a time-bearing id would tell a member that someone elsewhere posted the same link, and when.
- **Edits**: every edit queues a job, links or not; the job replaces the message's previews only if the message still says what the job read, and publishes when it shows something or removed something, so a job fetching the text before the edit can never leave its previews on the edited message. Clients take previews from a `MESSAGE_UPDATE` only when it is a newer edit (a pin's event or the editor's REST answer may have read them before the fetcher wrote).
- **The fetcher refuses the instance's own network**:
  - only `http` and `https` on ports 80 and 443; no credentials in links; no proxy from the environment, no cookies;
  - the server resolves the name itself and connects only if every address is public: loopback, private, link-local, carrier-grade NAT, multicast, broadcast, documentation, benchmarking, reserved and unspecified ranges are refused in IPv4 and IPv6, and IPv4 inside IPv6 (mapped, compatible, NAT64) is checked as IPv4. The connection goes to the checked address, so a second DNS answer cannot swap it;
  - redirects are followed by hand, at most 5 (3 until the 2026-11-05 amendment), each checked again (scheme, port, addresses);
  - 3 seconds to connect, 8 seconds per fetch (redirects and body included), 30 seconds per message for its links and pictures together, at most 1 MiB of a page read (the head is enough), only `text/html` and `application/xhtml+xml` parsed, off the async threads and in linear time on hostile markup;
  - links found before the send lock, in linear time on hostile text;
  - a bounded queue (256 messages; a full queue drops the message, which simply gets no preview) and 2 jobs at a time for the whole instance, plus the existing image processing permits; running jobs stop with the server.
- **The user agent says who it is**: `Mellow/<version> (link preview; +https://github.com/Paul-Carouge/mellow)`. Logs say how many links a message had and how many previews it shows, at debug, never which; the HTTP client's own connection lines (`hyper_util`) are capped at errors whatever `RUST_LOG` says.
- **Players**: a YouTube link's preview plays in place after a click, in `https://www.youtube-nocookie.com/embed/<id>` (the policy gains `frame-src https://www.youtube-nocookie.com`; nothing loads from YouTube before the click). YouTube Music watch links carry the same video id and play in the same player. Spotify and Deezer show their preview card and open in a new tab; their players come later, each adding its own origin to `frame-src`.

## Consequences
- Every link posted is fetched by the instance, from the instance's address, private channels and conversations included, as on Discord. Sites see the instance, never its members. An instance that must not reach the Internet turns previews off.
- The HTML parsing is Mellow's own small reader of `<meta>` and `<title>` in the page's head, not a browser engine: unusual markup gives no preview rather than a wrong one. Pages built only by JavaScript give little.
- `reqwest` becomes a dependency of the server binary (it was a test dependency); no new crate.
- A preview is shared by every message linking the same address within a day: an edit of the page shows up the next day at most.
- A cached preview arrives faster than a fetched one: a member timing their own link's preview can still guess that someone posted it in the last day (not who, where or when). Accepted: the alternative is one fetch per channel.
- One member can fill the queue with slow links and delay everyone's previews (never their messages); per-author fairness waits for a measured need.

## Alternatives considered
- **Readers' browsers fetch the previews**: leaks each reader's address to every site linked, needs a wide `connect-src` and `img-src`, and CORS blocks reading most pages.
- **The sender's browser fetches and sends the preview**: CORS again, and a sender could forge any title and picture for any link.
- **An external preview service** (iframely, a self-hosted proxy): one more thing to install or trust, against the one-binary promise.
- **An HTML5 parser crate**: a full parser for a few `<meta>` tags; a bounded reader of the head is enough and keeps the dependency list as it is.

## Amendment (2026-11-05): five redirects
A Shorts link shared from YouTube's app (`youtube.com/shorts/<id>?si=…`) reached from some servers' addresses goes through YouTube's consent page and back, four redirects, so the fetcher gave up and the link got no card and no player. The fetcher now follows at most 5 redirects. Each one is still checked like the first (scheme, port, credentials, every resolved address, Klipy), and the whole fetch keeps its single 8-second budget, so the extra hops add no reach and no time.

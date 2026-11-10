# Spotify metadata fixtures

Created 2026-11-09 from the public schema described in the Spotify matching design's read-only 2026-11-09 observation. These are synthetic, minimized schema fixtures, not complete page captures. Titles, artist names and artwork paths are deliberately synthetic; they establish parser behavior, not live compatibility. No cookies, preview audio URLs, authorization data, page tokens or hydration fields outside the required schema are retained.

`embed.html` supports exactly the inert JSON script identified by `id="__NEXT_DATA__"`, at `props.pageProps.state.data.entity`. Required identity is `type: "track"` and a matching `uri` or `id` (if both exist both must match); required display title is `name`. Optional fields are `artists[].name`, `duration` in milliseconds and `coverArt.sources[].url`. The test deliberately preserves multiple artists, Unicode and fractional seconds. This undocumented embed schema can change.

`oembed.json` models the documented rich Spotify oEmbed response. The iframe `src` is inspected as inert identity evidence only; the HTML is never executed, rendered or fetched. Title/thumbnail are used only after the iframe URL identifies the expected track. Artist and duration remain absent. The official endpoint is `https://open.spotify.com/oembed?url=<canonical-track-url>`.

`embed-missing-optionals.html` and `embed-changed-markup.html` represent missing optional data and an unsupported script schema. Other malformed, duplicate-script, oversize, identity-mismatch and hostile-URL variants are generated from these tiny fixtures inside unit tests. Network tests use an injected loopback endpoint only after the production logical URL policy is checked; no unit test contacts Spotify.

`embed-visual-identity.html` models the additional exact `entity.visualIdentity.image[].url` path observed live on 2026-11-09. The parser accepts this path first, with `coverArt.sources[].url` as a separate fallback; both use the same image-host policy. `youtube-search.json` is synthetic minimized yt-dlp flat search output for deterministic ranking and filtering tests.

# Spotify Review Follow-up Implementation Plan

> Implemented through Claws 1.0.8. The tasks and unchecked steps below preserve the review specification; they are not an active queue. Current evidence and remaining live/native limits are in [Spotify verification](../../spotify-track-matching-verification.md).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correct the confirmed Spotify matching UX issues and improve matching robustness without changing explicit selection, fresh audio verification or download-source guarantees.

**Architecture:** Keep the current resolver/controller/queue boundaries. Give Spotify input failures typed recovery information, retain the query DOM node across matching-state updates, extend Library filtering to recorded Spotify identity, and tune ranking against a fixed adversarial fixture set.

**Tech Stack:** Existing Rust/Tauri, TypeScript, Node tests and headless Edge harness. No new runtime dependencies.

**Spec:** The two reviews supplied by the user, checked against `356819d`, together with `../specs/2026-11-09-spotify-track-matching-design.md`. This follow-up proposes the changes below to the original title-warning, initial-query and artist-score rules; no implementation changes were made while preparing it.

## Review reconciliation

Both reviews report no blocking correctness/security defect and agree on title-warning noise, duplicate fallback copy and test organization. The attachment adds unsupported-input recovery, provenance search, query length, IME editing and verification gaps. The pasted review additionally questions collaboration scoring and import churn.

Current source inspection confirms:

| Item | Assessment | Decision |
| --- | --- | --- |
| Positive “Similar title” warning | Confirmed, including exact titles | Remove positive text from warnings; preserve actual uncertainty/version warnings |
| Duplicate missing-metadata explanation | Confirmed: backend warning + controller error render together | Deduplicate once at presentation boundary; keep distinct useful warnings |
| Unsupported errors and Retry | Confirmed: all unsupported cases use collection wording, and trackless failures always offer Retry | Classify reasons and expose recovery explicitly; no futile retry for invalid/unsupported input |
| Library provenance search | Confirmed: filter checks only YouTube title/uploader | Search Spotify title and artists too |
| Generated query over 300 characters | Confirmed: unbounded artist join; title-only query omits `audio` | Budget automatic queries deterministically without changing identity metadata |
| IME composition | Confirmed structural risk: every input event publishes and stage replacement recreates the focused field; not reproduced with a real OS IME yet | Keep the query node stable and add composition tests plus real IME acceptance |
| Collaboration scoring | Mean dilutes partial artist agreement as stated; correctness of replacing it with max is not established | Test a conservative max/mean blend against cover/collaboration fixtures; no blanket max-only change |
| Test placement/debug noise | Confirmed | Move tests, remove fetch-level status printing; retain intentional opt-in live-test summaries |
| `.js` imports | Deliberate NodeNext test-build compatibility, not unrelated accidental churn | Keep; no revert or new DTO-module refactor just to remove the extensions |
| `audioAvailable` fragment/top-level false negatives | Predicate requires per-format URL; hypothetical for this YouTube-only flow | Defer broadening until an actual supported-source fixture establishes a downloadable format |
| Global DRM text | True outside Spotify; outside this feature | Separate future task, not this follow-up |
| Live tests ignored by reviewers | Reviewers did not rerun them; implementation report records live runs | Preserve historical evidence and distinguish new reruns; do not label previously executed audio tests untested |

The attached review explicitly retracts the suspected navigation/enqueue bug. Do not reopen or redesign that path without a new failing reproduction. Current follow-up check: `npm run test:spotify` passed all 27 tests; no full-suite/live rerun was performed for this planning-only request.

## Global constraints

- Never select or enqueue a candidate automatically, including after ranking changes or navigation restoration.
- Fresh same-video `audioAvailable === true` remains required for confirmation.
- HTTPS allowlists, redirect/body/time limits, canonical source URLs, origin validation, persistent provenance and overwrite protection remain unchanged.
- Preserve raw Spotify identity and candidate display text; query budgeting and ranking normalization operate on derived values only.
- No Spotify authentication, new source/provider, albums/playlists, installer, release or publication.
- Do not change all of `AppError` serialization to solve a Spotify-specific presentation issue.

## Review focus

1. An IME edit invalidates a previous confirmation immediately, even while composition remains active.
2. Retry appears for a retryable transport/short-link failure, not malformed IDs, insecure URLs, unsupported entities or URIs.
3. Long Unicode metadata produces one usable automatic query within the backend's 300-character limit without splitting a surrogate pair.
4. A cover mentioning the original artist cannot outrank a clean matching artist merely because collaboration scoring is relaxed.
5. Library search finds either identity while old records without provenance behave as before.

## Task 1: Correct messages, recovery actions and announcements

**Files:** `src-tauri/src/spotify.rs`, `probe.rs`, `commands.rs`; `src/api.ts`, `spotify-match.ts`, `views/spotify-match.ts`, `store.ts`; Spotify Rust/Node/DOM tests.

**Interfaces and decisions:**

- Give unsupported input a reason: `unsupportedCollection`, `unsupportedUri`, `unsupportedEmbed`, `invalidTrackUrl` or `insecureUrl`. Preserve the exact supported host policy. A malformed or insecure Spotify URL must never be silently repaired into an accepted network request.
- Add Spotify-command-specific serialized failure `{ code, message, retryable }` and mirror `SpotifyLookupIssue` in TypeScript. Only `resolve_spotify_track` uses this error envelope; ordinary commands retain their string errors. Resolver transport errors map to retryable failure, terminal input reasons to nonretryable failure. Unexpected error shapes still get a readable fallback.
- Extend matching state/snapshots with recovery information; render Retry from this information rather than `!track`. A known full track whose metadata is unavailable keeps editable manual search. A failed short-link resolution can retry or accept a full track URL. Unsupported collections/URIs/embed links ask for the appropriate full single-track URL.
- Deduplicate trimmed identical warning/error strings when rendering; preserve stable warning order. Do not classify recovery by matching English error text.
- Remove `Similar title` from backend `warnings`. Do not add a new positive-signal DTO solely for this label.
- Show “N recordings found. Choose a recording.” or the existing empty-results explanation in a persistent polite status region. Do not move focus when results arrive.

- [ ] Write failing Rust tests for distinct collection, URI, embed, malformed ID and HTTP rejection reasons, plus readable generic-probe guidance before engine discovery.
- [ ] Write controller/DOM tests: duplicate fallback occurs once; distinct warnings remain; terminal input has no Retry; retryable short failure has Retry; missing known-track metadata allows edited search; unexpected error object remains readable; results/empty status is announced without selection.
- [ ] Run `cargo test --locked --manifest-path src-tauri/Cargo.toml spotify` and `npm run test:spotify`; observe the new failures.
- [ ] Implement typed failure mapping and state recovery, warning deduplication, title-warning removal and result status. Update dev mock and existing tests for the Spotify-specific rejection shape.
- [ ] Rerun both commands and `npm run build`; all pass. Commit `fix: clarify Spotify matching errors and status`.

## Task 2: Bound automatic queries and search recorded Spotify identity

**Files:** `src/spotify-match.ts`, `src/views/library.ts`, new `src/library-search.ts`, new `scripts/tests/library-search.test.mjs`, `scripts/tests/spotify-match.test.mjs`, `package.json`.

**Interfaces and decisions:**

- Keep `initialSpotifyQuery(track: SpotifyTrack): string`. Use a maximum of 300 Unicode code points, consistent with Rust `.chars().count()`. Return empty only when no usable title exists.
- Reserve the literal suffix ` audio` in every generated query, including title-only metadata. Reserve the full title where it fits; fill the remaining budget with complete artist names in their original order and a ` - ` separator. Never concatenate 20 full artists before checking the cap.
- If the title plus suffix alone exceeds the cap, take a code-point-safe title prefix within the budget, prefer its last whitespace boundary when present, then append ` audio`. Missing trailing words reduce recall, so the query stays visible/editable. Original title/artists and saved provenance must remain untouched.
- Manual queries over the limit still produce validation guidance; do not silently truncate the user's edits.
- Export pure `matchesLibrarySearch(item, query): boolean` from `library-search.ts`. Search the existing title/uploader plus optional `matchOrigin.spotifyTitle` and each Spotify artist, using the current trim/lowercase substring semantics. Blank query returns true. No relevance sorting or accent normalization in this change.

- [ ] Write failing query tests: title only includes `audio`; 20 long artists; title longer than 300 code points; astral Unicode near boundary; empty artists; no title; <=300 characters and preserved metadata in every case. Assert normal short query stays `Artist - Song audio`.
- [ ] Write failing Library tests: different YouTube/Spotify titles, Spotify-only artist query, ordinary title/uploader, case/whitespace, blank query, old record without origin, and unmatched query. Compile/run the new pure helper in `test:logic`.
- [ ] Run the owning tests to observe failures, implement the bounded generator/helper, and replace only the Library filtering predicate.
- [ ] Run `npm run test:spotify`, `npm run test:logic`, `npm run build`; all pass. Commit `fix: bound Spotify searches and find tracks by provenance`.

## Task 3: Preserve composition and prevent redundant search work

**Files:** `src/views/spotify-match.ts`, `src/views/download.ts`, `src/spotify-match.ts`; `scripts/tests/spotify-match-dom.test.mjs`, `spotify-match.test.mjs`, `scripts/dev/spotify-bench.mjs`.

**Interfaces and decisions:**

- Introduce a mounted matching view handle `{ el, update(state), dispose() }` with one query input for the lifetime of a matching session. The parent must retain the matching host across `spotify` and confirmed `result` phases, updating candidate/status/preview regions without recreating or detaching the query field.
- Keep controller query updates and epoch invalidation immediate on composition input; delaying invalidation could leave a stale confirmed Download usable. Update the rest of the controls/results in place. Do not force `.value`, selection ranges or focus on the active composing input.
- Track `compositionstart`/`compositionend`; pressing Enter while `event.isComposing` or the local composition flag is true commits the IME candidate only, never starts search. One subsequent ordinary Enter may search. Navigation disposal must clear view handlers/state safely.
- Add `search()` early return when the current request is already searching the same query. `setQuery` still invalidates that epoch and allows a new query search; old replies remain ignored. No cache, debounce or global process cancellation framework is needed.

- [ ] Write failing mounted/DOM tests preserving the actual input node through composition inputs, end-of-composition, result arrival and selected-probe transitions. Assert composed Japanese text survives, selection/focus stays, previous confirmation is invalidated immediately, and Enter during composition invokes zero searches.
- [ ] Write deferred controller tests: two immediate identical `search()` calls produce one dependency invocation; changed query can start a newer search; late older results cannot restore confirmation.
- [ ] Implement persistent matching host/view updates and the small search guard. Keep ordinary Download rendering and existing enqueue ownership tokens unchanged.
- [ ] Run `npm run test:spotify`, `npm run build`, and the mounted Edge harness. Existing navigation/enqueue/replacement/cache/batch cases plus new composition/query cases must pass with no page exceptions.
- [ ] Manually verify one Japanese or Chinese IME session in an isolated Windows WebView2 app. Synthetic CompositionEvents alone do not establish real OS composition behavior. Record this as unverified if unavailable; do not claim native IME coverage.
- [ ] Commit `fix: preserve Spotify query composition and suppress duplicate searches`.

## Task 4: Improve artist/channel scoring with adversarial fixtures

**Files:** `src-tauri/src/spotify_match.rs`, `src-tauri/tests/fixtures/spotify/youtube-search.json` and new ranking cases; adjust the original design's ranking description after tests settle the behavior.

**Interfaces and decisions:**

- Retain `YouTubeCandidate` and existing 0.55 title / 0.30 artist / 0.15 duration weights and version penalties. No confidence probability, default choice or acceptance threshold is introduced.
- For artist overlap, evaluate `0.70 * max(per_artist_overlap) + 0.30 * mean(per_artist_overlap)`. This gives partial collaboration credit while retaining a completeness component. This is a proposed starting formula, not proven match accuracy; fixtures below are the acceptance gate. Do not simply switch to a pure maximum or assume Spotify's first listed artist is always primary.
- Normalize uploader suffixes only for scoring: remove anchored ` - Topic`; strip a trailing `VEVO` only when the remaining normalized uploader equals a requested artist. Preserve raw channel display text. No verified-channel bonus in this pass; verification alone does not identify the requested recording.
- Strip only explicit bracketed or separator-delimited trailing presentation annotations such as `(Official Audio)`, `[Official Music Video]` or ` - Lyrics` when comparing titles, and only when that annotation is not part of the requested title. Preserve live/remix/cover/karaoke/instrumental/sped-up/slowed categories and legitimate song words. Do not normalize away year/remaster/version identity.
- Add “Artist/channel differs” only when artist metadata exists and no artist overlap can be established; missing metadata retains its separate warning. It remains a caution, not a rejection.

- [ ] Add failing fixture comparisons: exact solo; exact collaboration naming all artists; collaboration uploaded by one participating artist; primary artist's Topic/VEVO upload; a cover mentioning the original artist; official presentation annotations; legitimate song named “Lyrics”; live/remaster differences; missing artist/uploader; non-Latin names; stable ties.
- [ ] Assert clean matching recordings outrank covers/wrong versions where metadata supports that conclusion. Preserve exact solo score and existing duration/version warnings. Evaluate the proposed blend against this corpus; retain the old formula if it cannot satisfy the adversarial cases, and document the reason rather than claiming a quality improvement.
- [ ] Run `cargo test --locked --manifest-path src-tauri/Cargo.toml spotify_match`; observe failures, implement narrowly scoped normalizers/score change, and rerun until green.
- [ ] Commit `fix: improve collaboration and artist-channel match ranking` only when the fixture evidence supports the new behavior.

## Task 5: Small cleanup and final verification

**Files:** `src-tauri/src/spotify_match.rs`, `probe.rs`, `spotify.rs`; `docs/spotify-track-matching-verification.md` and fixture documentation as needed.

- [ ] Move `spotify_match.rs` tests below production definitions; move `use super::*` above the new probe tests. Remove only fetch-level `#[cfg(test)]` HTTP status `eprintln!`; keep opt-in live summaries needed to record evidence. No global formatting/import reshuffle.
- [ ] Run fonts, Spotify, logic, queue and motion Node suites plus `npm run build`; build the fixture tool, run full locked Rust tests and warning-denying all-target Clippy. Run `git diff --check`. New test counts will exceed the original 27/434; record actual totals rather than copying old figures.
- [ ] Extend mounted checks for unsupported-type messages, duplicate warning absence, result announcements, bounded query generation, composition, duplicate search suppression and provenance Library filtering. Retain the original 22 lifecycle checks.
- [ ] Read-only live short-link smoke: use a real public `spotify.link` track URL when available; record every logical redirect host and final entity without tokens. If it traverses `spotify.app.link` or another host, keep rejecting it and require a full track URL. Expanding the allowlist is a separate reviewed decision, never a test convenience. An invalid placeholder is not a live success.
- [ ] Verify an isolated real WebView2 flow: supported full link, unsupported input, edited query with real IME, explicit selection, confirmation, audio options and one saved recording. The existing headless harness and prior Rust live matrix are useful separate evidence, not replacements for this native UI check.
- [ ] Rerun live metadata/search after resolver or ranking changes. Rerun the 12-case audio matrix only if download options, source/provenance mapping or execution changes; otherwise preserve its prior dated evidence and perform one end-to-end save. Explain any blocked live/native checks precisely.
- [ ] Update the verification report and original design only for implemented changes. Keep the feature branch and installed-app state explicit. No installer, release or merge in this plan.

## Deferred items

Global DRM messaging, broadened fragments/top-level audio detection, previews, “best match” badges and a verified-channel ranking signal are optional independent enhancements. None is necessary to address the confirmed feedback. Candidate preview links need a scoped opener policy; audio detection needs a real supported-source fixture. Do not weaken fresh-audio or source-identity checks to make unsupported fixtures appear playable.

## Delivery order and completion

Implement Tasks 1–3 first: these fix concrete user-visible errors, findability and input reliability. Task 4 follows as a separately reviewable matching-quality change. Task 5 is the final acceptance/cleanup pass. Each implementation task starts with a regression that fails on `356819d`, then passes with its change.

Completion means the confirmed issues have regression coverage, existing confirmation/security/persistence behavior remains green, and live/native results are accurately reported. Remaining verification gaps must stay visible; they are not evidence of a blocking architecture defect.

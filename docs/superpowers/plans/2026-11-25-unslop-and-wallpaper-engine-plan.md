# Unslop UI follow-up and Wallpaper Engine completion plan

- Date: 2026-11-25. Base: `main` at e49997f (Claws 1.0.9a).
- Inputs: the review at `C:\Users\A3\AppData\Local\Temp\claude\fb7dc4b4661c7741\08c0237f-388f-4190-9bad-fd84cdce835a\scratchpad\claude-review.md` (with the merged Gemini section) and `...\scratchpad\gemini-review.md`. Skill text: `...\scratchpad\unslop\SKILL.md`, `surface.md`, `examples.md`.
- Read first: `docs/HANDOFF.md`, `docs/CURRENT_DEVELOPMENT_STATUS.md`, `PRODUCT.md`, `DESIGN.md`.

## Ground rules
- Keep app ID `dev.claws.downloader`, settings schema 2 / queue schema 3 contracts in HANDOFF. New settings fields must be optional with serde defaults and survive `Settings::repair`; if a schema bump is unavoidable, write the migration and its test.
- Rust runs tools with argument arrays, never a shell.
- Do not touch `installers/`, the version files, or the untracked `SaltedFluttersPolaris/` folder. No release build.
- DESIGN.md values (13px button padding, 26px section margin, springs) are intentional; change spacing only where a finding below names it.
- Screenshots: headless Chrome against `npm run dev` via `scripts/dev/cdp.mjs` only. Never capture the desktop or screen regions.
- Commit on the worktree branch in small phase-sized commits, no attribution trailers, no push, no PR.
- Every phase ends green: `npm run build`, `npm run test:logic`, `npm run test:queue`, `npm run test:motion`, `npm run test:spotify`, `cargo test` and `cargo clippy --all-targets -- -D warnings` in `src-tauri`.

## Phase W0: no-sign-in download spike (do this first; it decides W3)
Goal: find a legitimate way to export a Wallpaper Engine Workshop item without Claws asking for a Steam login, and prove it with evidence.

Test these paths, in order, and record results in `docs/wallpaper-engine-no-login-spike.md` (commands run, item IDs, responses, outcome):
1. **Anonymous SteamCMD** `+login anonymous +workshop_download_item 431960 <id> +quit`. Expected: refused (app 431960 requires ownership). Re-confirm with the exact error line so the UI can recognise it.
2. **Public file URL.** Call `ISteamRemoteStorage/GetPublishedFileDetails/v1` for at least 10 real Wallpaper Engine items (mix of scene, video, web, and old vs new) and record whether `file_url` / `hcontent_file` / `file_size` are present. Where `file_url` is present, try an unauthenticated GET (respect redirects only to Valve UGC hosts) and inspect what arrives (a full project, or only one file). Pick IDs from the public Workshop browse page.
3. **Installed Steam client (local library).** If Steam is installed and signed in with Wallpaper Engine, subscribed items already sit in `<library>\steamapps\workshop\content\431960\<id>`. Find libraries from the registry (`HKCU\Software\Valve\Steam\SteamPath`) plus `steamapps\libraryfolders.vdf`; check whether the item folder exists and contains `project.json`. Claws needs no login for this: the user subscribes in Steam (`steam://url/CommunityFilePage/<id>` page, Subscribe button) and Claws exports the folder.

Out of scope and must not be built: third-party mirror or "workshop downloader" services, shared/borrowed accounts, or anything else that gets content the user's own Steam account is not entitled to. Note this in the spike doc.

Decision gate (write it in the spike doc):
- Path 3 works → implement it as the default "From your Steam library" source in W3; SteamCMD becomes the fallback.
- Path 2 gives a complete project for some items → implement as an extra source only for those items, with the item-type check.
- Neither works → keep SteamCMD only and say so in README; still implement W1–W2.

Tests for the spike outcome: a Rust unit test that parses a fixture `libraryfolders.vdf` (several libraries, odd paths, Unicode) and finds `content\431960\<id>`; a test that the anonymous-refusal line is classified correctly; a fixture-based test for whichever public-URL handling ships. A live check on this PC if Steam is installed; if it is not, record that the live check is still owed.

## Phase W1: Wallpaper Engine correctness and robustness (`src-tauri/src/workshop.rs`, `state.rs`, `lib.rs`)
1. Return item details, not a bool: a typed `WorkshopItem { id, title, preview_url, file_size, time_updated, kind }` and an enum for Wallpaper / Collection / OtherApp / NotFound. Reject collections with a clear message (use `IPublishedFileService/GetDetails` or the `children`/file-type fields; test with fixtures). Detect required child items (dependencies) and either download them too or warn before downloading.
2. One shared no-redirect `reqwest::Client` on `AppState` with a `Claws/<version>` User-Agent.
3. Cancellation and shutdown: store the running SteamCMD PID plus a cancel flag on `AppState`; add `cancel_wallpaper_download`; check `shutting_down` before spawning; kill the process tree (`downloader::kill_tree`) on cancel, timeout and in `downloader::shutdown`.
4. Progress: read SteamCMD stdout and stderr (bounded), emit `workshop-progress` events with stages (Starting SteamCMD / Updating SteamCMD / Signing in / Downloading / Saving ZIP) and a percentage when SteamCMD reports one; check the exit status.
5. Classify failures from the captured tail: invalid password / cached credentials expired, Steam Guard needed, no subscription / access denied, item not found, timeout, disk full, rate limited. Each gets its own one-or-two-sentence message saying what to do; keep the raw tail for a "Show the full message" disclosure (redact the account name and home path).
6. Persist the SteamCMD path and last account name in settings (optional fields); validate on load; never store passwords. Verify the chosen `steamcmd.exe` has a valid Authenticode signature from Valve (WinVerifyTrust + signer name) and warn if it sits in a OneDrive/sync folder (env vars `OneDrive*`, reparse attributes on parents). Drop the OneDrive hint text if the check is not implemented.
7. ZIP export: `BufWriter` (256 KB) around the file, a reusable copy buffer, entries sorted by name, `large_file` only when needed, Deflate for text-like files (`.json .html .js .css .txt .frag .vert`) and Stored for media, open files with `FILE_FLAG_OPEN_REPARSE_POINT` and re-check the handle is a file, a total-bytes cap, all filesystem work in `spawn_blocking`.
8. Free-space check on both the SteamCMD volume and the output volume before starting (use `file_size`, then the real folder size before zipping), reusing `disk_budget.rs` helpers.
9. Write the `.partial` under a Claws work folder that the existing startup sweep cleans, or extend the sweep to `.claws-wallpaper-*.partial`.
10. Name the ZIP `<sanitized title> (<id>).zip` (Windows reserved names, length cap, fallback `Wallpaper Engine <id>.zip`), still never replacing an existing file.
11. Optional after-save cleanup of SteamCMD's copy (`content\431960\<id>` and its acf entry) behind a setting, default off.
Tests: extend the existing `workshop.rs` tests for each item above (fixtures for details JSON, collections, dependency lists, failure lines, file naming, sort order, partial cleanup, cancellation of a fake long-running child).

## Phase W2: Wallpaper Engine UI (`src/views/workshop.ts`, `download.ts`, `store.ts`, `api.ts`, `mock.ts`, `settings.ts`)
1. Keep the Workshop session in the store (like `store.spotifyMatch`) so leaving and returning to Download shows the running export, its progress and result; `stageKey()` includes the URL; run `settleIn` like other stages.
2. Preview card identical in structure to the video preview (art, title, "Wallpaper Engine · <type> · <size>"), then a group: source (Steam library / SteamCMD), account (only for SteamCMD), "Saves to <folder>", primary action, Cancel while running, progress bar plus stage text.
3. Disabled action says why next to it ("Choose SteamCMD first", "Enter your Steam account name"). Enter in the account field submits.
4. Errors in `.notice.notice-error` with `role="alert"` and the "Show the full message" disclosure; success line with "Show in folder".
5. A Steam check that fails on the network shows "Couldn't reach Steam to check this Workshop item." with Try again, instead of falling through to yt-dlp (download.ts:366 and store.ts batch path). A Workshop link for another game says Claws handles Wallpaper Engine items only.
6. "View in Steam" falls back to the https page when `steam://` fails.
7. Guided sign-in for SteamCMD: "Sign in to Steam…" opens a visible SteamCMD console with `+login <account>` so password and Steam Guard are entered there; Claws detects success from the next run. Link to Valve's SteamCMD page.
8. Settings: a "Wallpaper Engine" section with the SteamCMD path (Choose…, Clear), account name, and the cleanup toggle. Optionally a managed SteamCMD install from Valve's official `steamcmd.zip` into the tools folder, following the yt-dlp/ffmpeg install pattern and checks (only if it fits the existing tools.rs flow cleanly; otherwise list it as follow-up).
9. Library: record exports as a `wallpaper` kind in history (Library filter, Show in folder, re-paste shows "Downloaded <date>"). Respect history schema rules.
10. Mock: full simulated flow (choose SteamCMD, details with synthetic preview, staged progress, success, each error class, cancel) so every state can be rendered in `npm run dev`.
Tests: `isWorkshopLink` unit test (add to `test:logic`), DOM test for the Workshop panel states like `spotify-match-dom.test.mjs`, headless screenshots of each state at 1100x760 and 700px wide in light and dark.

## Phase W3: implement the no-sign-in source chosen in W0
Per the decision gate. For the local Steam library source: detect Steam and libraries, show "Found in your Steam library" or "Subscribe in Steam, then come back" with a button that opens the item page; watch the folder (poll every few seconds with a timeout, cancellable) and export when `project.json` appears; never write into Steam's folders. Tests per W0, plus a fixture library tree end-to-end export test.

## Phase U: unslop-ui fixes (copy, alignment, small bugs)
Bugs:
- `probe.rs:312` site shows "Youtube": map extractor keys to display names (YouTube, Vimeo, SoundCloud, X/Twitter, TikTok, Twitch…) or derive from `webpage_url_domain`; test.
- `format.ts:36-47` mixed locales: format with the UI's locale ("en") everywhere or use `Intl.RelativeTimeFormat` consistently; test.
- "Download N videos" / "N videos added to the queue" when Audio only: branch on mode (download-batch.ts:182, download.ts:613, download-playlist.ts:48).
Copy (apply exactly unless a better string uses only words the screen already shows):
- download.ts:143/176 unify on "Look up" / "Look up N links"; download-batch.ts:178,222 "Read … again" → "Look up … again".
- resolution.ts quality chips: one encoding, e.g. "4K" / "1080p", exact WxH in the chip title; batch and single screens agree.
- download.ts:889 empty state: drop the duplicate "Paste a link" title (keep icon and the Ctrl+V line) per DESIGN empty-state pattern, or retitle "No link yet".
- download.ts:458 colon → period. download.ts:681 freshness line: show the warning alone when present, otherwise the date.
- Tooltips with internals: download-shared.ts:143,167,256-257; download-batch.ts:95,119; download-playlist.ts:28; queue.ts:273-275 (delete titles that repeat the label); processing-options.ts:27,40-41; transcript-options.ts:106 (show the limit only near it).
- Settings: settings.ts:119 "Processing alongside" → "Simultaneous conversions" with "Downloads wait when this many are merging or converting."; :124 speed hint → "Applies to each download. Takes effect on the next start or resume."; :134, :142-157, :286, :377 shorten as in the review; :360 row title "Deno 2.9.8".
- setup-panel.ts:415-416 "Downloaded from each project's GitHub releases."
- spotify-match.ts:87 two-sentence intro; :114 em dash → comma; add a visible label for `#spotify-query`.
- queue.ts:288 empty state "Finished files go to your Library."
- shell.ts:167 window title separator.
- main.ts:53 fatal screen: add a next step (restart Claws; if it repeats, reinstall) or a Retry button if startup can be retried.
- U11: drop icons from text buttons where siblings are text-only ("Open folder", "Copy"); keep Download-button icons only if DESIGN.md requires them (check and note).
Layout (verify with screenshots before/after):
- Queue rows: fixed-width action cluster so kind label and progress bar end on one line; "Drag file" becomes an icon button with a name, or moves.
- Download result: estimate line and "Saves to" on the section-title text line (one shared padding variable).
- Spotify identity block aligned with the intro/field line.
- Speed limit: avoid the orphan chip (fewer chips or a select).
- Traffic lights vs toolbar vertical centre: align and update DESIGN.md if a recorded value changes.
- Library: tiles show at most three sub-lines (merge date into the meta line); move "Clear all" out of the toolbar beside Search (e.g. to the end of the grid or behind a menu) keeping its confirm sheet.
- Remove unused `spinner()`, `.loading`, `.banner` code; `aria-hidden` on the sidebar tool dot.

## Phase V: verification and docs
- All test commands above, plus new tests. Headless screenshots of every screen and every Workshop state, before and after, saved under `docs/performance/2026-11-25-unslop/` (only images of the dev mock).
- Update README (Wallpaper Engine section, no-sign-in path or why not), `docs/CURRENT_DEVELOPMENT_STATUS.md`, `docs/HANDOFF.md` ownership table, `docs/UNSLOP_UI_REVIEW.md` (append a 2026-11-25 section) and DESIGN.md where a recorded value changed.
- Final report: what shipped per phase, the W0 decision with evidence, test output, anything not done and why, and what still needs a live check by the owner (owning-account SteamCMD download, native app run).

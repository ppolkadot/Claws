# Developer handoff

Start with [current status](CURRENT_DEVELOPMENT_STATUS.md) and [release notes](releases/1.0.9.md). The handoff describes how to work on Claws; release chronology is recorded in `releases/` and git history.

## Architecture and scope

Claws is a Windows Tauri 2 desktop app with plain TypeScript/CSS. Rust invokes yt-dlp and FFmpeg with argument arrays, never a shell. The user guide is [README](../README.md); product and visual constraints are in [PRODUCT](../PRODUCT.md) and [DESIGN](../DESIGN.md).

| Responsibility | Owning modules |
| --- | --- |
| Queue, execution and lifecycle | `downloader.rs`, `download_policy.rs`, `transfer_progress.rs` |
| Owned files and durable recovery | `download_files.rs`, `queue_writer.rs`, `store.rs`, `history.rs`, `history_mutation.rs` |
| Space accounting and previews | `disk_budget.rs`, `estimate.rs` |
| Probing, saved links and transcripts | `probe.rs`, `links.rs`, `transcript.rs` |
| Spotify identity and YouTube search/ranking | `spotify.rs`, `spotify_match.rs`; frontend `spotify-match.ts` and its view |
| Tools and desktop integration | `tools.rs`, `desktop.rs`, `file_drag.rs`, `window.rs`, `commands.rs` |
| Frontend state and navigation | `store.ts`, `api.ts`, `views/`, pure helpers and `ui/` |

## Contracts to preserve

- Keep the application ID `dev.claws.downloader`. Settings, queue, history, caches and managed tools live under `%APPDATA%\dev.claws.downloader`. Preserve the owner's profile; use temporary profiles/output for tests.
- Settings schema is 2; queue schema is 3. Preserve newer/corrupt data and ambiguous backups rather than guessing or starting an empty writable queue. Keep migrations and recovery journals durable.
- A job freezes output/template/processing choices before first execution. Retry/resume uses that execution and source; current cookie/tool locations may change. Cancellation cleanup is scoped to the verified attempt's work folder.
- Download into an owned `.claws-work` folder, then publish verified outputs without overwriting existing files. A recorded path alone is not permission to delete; keep file identities, publication evidence, history mutations and kept-file recovery intact.
- `max_concurrent` counts transferring jobs. Up to `max_processing` jobs in final processing hold no transfer slot; each one beyond that does, so at most the two limits added together run at once. A merge already running inside yt-dlp is never held back.
- A speed limit is read when a download starts and is passed only to downloads, never to lookups or previews. yt-dlp applies `--limit-rate` to each fragment connection, so a limited start runs with one fragment; the job's frozen `fragment_concurrency` is not rewritten.
- Tools are installed only when the user asks (the setup panel or Settings); there is no update check at launch. Every tool archive is checked against its publisher's SHA-256 before it is opened. Deno publishes PowerShell `Get-FileHash` output rather than a checksum listing; `tools::checksum_in` reads both and requires the `Path` line to name the archive. The Deno that Claws installs is passed to yt-dlp as `--js-runtimes deno:<path>`.
- **Move to Top** raises a job's saved `priority`; `pump` starts the waiting job with the highest one, then list order, and the interface lists by priority, then creation time. The processing `stage` is transient: sent to the interface, never written to the queue file. Both are defaulted fields, so queue schema stays 3.
- The bulk commands (`pause_all`, `resume_all`, `retry_failed`) go through the per-job `pause` and `requeue`, so they keep every per-job rule.
- Notifications are sent from Rust (`desktop::notify_finished`) only when the setting is on and the window is hidden, minimized or unfocused. The capability file grants only `core:window:allow-set-progress-bar` beyond what was there; add nothing that is not used.
- SponsorBlock is part of a job's frozen options, off by default, and the only feature that contacts `sponsor.ajay.app`; keep it opt-in per download and say so in the interface. It counts as a rewrite pass (`ProcessingPolicy::rewrites_output`), Fast Save turns it off, and a removal lowers an exact size preview to approximate. Only the categories in `SPONSORBLOCK_CATEGORIES` reach the command line.
- A playlist folder is a prefix (`download_policy::playlist_prefix`) stored on the job and put in front of the filename template when the job first starts. It is sanitized to one plain folder name and escaped for the template. Publication is unchanged: the same ownership and no-overwrite rules as any template with a folder.
- Chapters are embedded by the metadata pass (`--embed-metadata --embed-chapters`), never by a pass of their own.
- Progress reaches 100% only after publication. Late telemetry cannot undo Processing. Queue event revisions prevent older replies from reverting state.
- The window is borderless with its own traffic lights. tao still gives it `WS_SYSMENU`, and the glass/Acrylic/Mica frame extension (`window::apply_backdrop`) makes DWM paint Windows' own minimize/restore/close glyphs over the top-right corner when the window is maximized. `window::hide_system_buttons` removes `WS_SYSMENU` through a subclass that edits every `WM_STYLECHANGING`, because tao rewrites the style whenever a window flag changes. Alt+Space and the taskbar's system menu are the cost; snap, Win+Arrow, minimize and maximize still work. Do not extend the frame again without checking a maximized window.
- Keep size preview, admission and processing policy consistent. Processing monitoring is best effort; a preview is not a reservation or a strict gate before every FFmpeg launch.
- yt-dlp calls include `--encoding utf-8`; environment variables alone did not preserve printed Unicode paths on Windows.
- A Spotify URL identifies a song, not downloadable Spotify audio. Preserve explicit user selection, fresh same-video audio evidence, canonical YouTube sources and separate persistent provenance. Maintain HTTP allowlists, body/redirect/time limits and inert metadata parsing.
- Outgoing drag uses a Library ID and verified file location, never a frontend-supplied path. Shell/OLE work runs on a dedicated STA worker; an unresponsive receiver cannot safely be forcibly terminated.
- Preserve the four bundled Polaris weights and font license. Keep fingerprinted assets and the private font alias; do not restore a `local()` font fallback.
- The desktop supplies the glass background. Follow the existing material tokens, focus behavior and reduced-motion/transparency rules rather than adding a background image or decorative effects.

## Build and checks

Run from the repository root:

```powershell
npm ci
npm run test:fonts
npm run test:spotify
npm run test:logic
npm run test:queue
npm run test:motion
python scripts/test_font_metrics.py
npm run build
cargo build --locked --manifest-path src-tauri/Cargo.toml --features test-tools --bin claws-fixture-tool
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets --features test-tools -- -D warnings
git diff --check
```

`npm run dev` uses labelled synthetic data when no native backend is present; open it with `?setup=fresh` to see first-run setup with sample installs. `npm run tauri dev` runs the native app and can access its profile; isolate data before testing lifecycle actions. Native fixture tests use disposable data. Real-tool fixture tests require Python, yt-dlp and, where applicable, FFmpeg/ffprobe; see `CLAWS_TEST_YTDLP` and `CLAWS_TEST_FFMPEG_DIR` in `download_tests.rs`. The three live Spotify tests are opt-in and are not exercised by the default suite.

Repository-wide `cargo fmt --check` has pre-existing drift. Keep unrelated formatting out of focused edits.

For an authorized installer build, synchronize `package.json`, `package-lock.json` (root and package entries), `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` and `src-tauri/tauri.conf.json`, then run `npm run tauri build -- --bundles nsis`. Output is under `src-tauri/target/release/bundle/nsis/`. Record the source commit/snapshot, installer hash, extracted version and embedded-asset checks in the release notes under `releases/`. Attach the installer and its `.sha256` to the GitHub Release; do not commit new installers (`installers/*` is ignored). The files already tracked in `installers/` are historical records: keep them immutable, and a source edit does not replace a packaged release.

`.github/workflows/ci.yml` runs the frontend tests, the build, the backend tests and Clippy on `windows-latest` for every push and pull request. Tests that need a real yt-dlp or FFmpeg skip there, so run them locally before a release.

## Development tools

- `node scripts/dev/spotify-bench.mjs --out .test-build/spotify-bench/report.json` drives the actual frontend with synthetic IPC in an isolated headless Edge profile. Queue, batch and motion benchmarks use the same approach.
- `node scripts/dev/shell-bench.mjs --browser <chromium exe> --out file.json` measures navigation, the sidebar toggle, Library card movement and the lookup skeleton, with the sample queue and with 500 more rows. The benches default to Edge's usual path; pass `--browser` when it is elsewhere.
- `scripts/dev/download-fixtures.py` supplies controlled localhost transfer faults. `download-bench.mjs` measures integrity and transfer behavior; `processing-bench.mjs` measures processing. Raw evidence is in `docs/performance/`.
- `scripts/dev/phases.mjs <yt-dlp> <info.json> <outdir> [args]` times transfer and processing phases.
- `npx tauri dev --config scripts/dev/debug-config.json` enables the local WebView2 debugging port; that overlay does not isolate the production profile. Supply a separate test identifier/profile and output before native lifecycle tests. `node scripts/dev/cdp.mjs eval`, `wait` and `shot` control the view; inspect script arguments before use. A minimized renderer can throttle screenshots.
- `scripts/dev/glass-shot.ps1 -Out image.png` captures the native DWM composite and rejects overlapping windows. Preserve that guard when checking transparency. `blur-probe.ps1` tests DWM independently of Tauri.
- Stop only test processes and clean only test-owned data. Do not clean the owner's Downloads or Library to reset a test.

## Manual acceptance

Use disposable output and profiles. For native drag, keep the original Library file and verify its bytes after each case:

1. Copy from Library and completed Queue rows into Explorer; test an existing destination name without altering the source.
2. Import into an editor; record editor/version/container/codec and actual import separately from an OLE drop result.
3. Exercise Escape, self-drop, quick release and missing-file errors; repeat a drag and normal UI actions afterward.
4. Check Unicode, spaces, long paths, UNC/network locations and unavailable shares; the UI must remain responsive.
5. Check swapped primary buttons, receiver dialogs, keyboard reveal, touch/pen and thumbnail non-drag behavior.
6. Exercise a stalled receiver on supported Windows versions without starting unlimited workers or forcibly terminating COM calls.

Not yet exercised against a real download: SponsorBlock marking and removal, chapters in the saved file, and a playlist folder end to end (yt-dlp accepts the arguments and the prefixed template in a simulated run).

Other outstanding acceptance: clean install/upgrade, native reduced-motion behavior, real WebView2/OS-IME editing, real Spotify share links, live selected-source failures and constrained-disk processing. See [current status](CURRENT_DEVELOPMENT_STATUS.md) for scope and [Spotify verification](spotify-track-matching-verification.md) for the existing evidence.

## Deferred: dropping a link onto the window

Not built. Read from the code, not measured: the window is created with Tauri's native drag-and-drop handler, which on Windows replaces HTML5 drop events in the WebView and reports only dropped file paths, not a dragged link's text. Accepting dropped links would mean turning that handler off (`dragDropEnabled: false`), and `file_drag.rs` relies on its `DragDrop` event (`note_window_drop`) to tell a drag that began in Claws and ended on it from an export. That change cannot be accepted without the outgoing-drag manual acceptance list above, which needs a real desktop session. Paste (Ctrl+V anywhere) and Quick Open cover the same need today.

## Documentation maintenance

Use [the documentation index](README.md). Keep current guidance here, verification in the owning report and per-build provenance in release records. Dated plans/reviews are historical specifications, not competing task lists. Do not append another session transcript or repeat old release/test totals in this file.

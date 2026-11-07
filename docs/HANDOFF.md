# Claws: handoff

Everything a new session (human or AI) needs to pick this project up. Lives at `docs/HANDOFF.md`. Last updated 2026-11-05.

## Current state: 1.0.5 (2026-11-05)

This section supersedes the historical font, packaging and publication instructions below. Sections 1–17 record earlier work and observations, including the previous font setup; they are not the current build instructions.

- Source version is **1.0.5** in package.json, package-lock.json, Cargo.toml, Cargo.lock and tauri.conf.json. The installer was rebuilt from the reviewed source with `npm run tauri build` and copied to `installers/Claws_1.0.5_x64-setup.exe`: 2,839,767 bytes, SHA-256 `5380cffc46dd7ef85a52e5d44d5a6eb18b76e176164f9546296d9d19421bcfae`. Its archive passes integrity checks and its executable matches the optimized build exactly after Tauri's expected NSIS bundle marker patch; its executable version is 1.0.5. A fresh Windows installation and upgrade from an installed 1.0.4 remain untested.
- Release publication is authorized after review. Target: https://github.com/ppolkadot/Claws/releases/tag/v1.0.5. Notes: `docs/releases/1.0.5.md`. Publish from the exact reviewed commit, verify uploaded asset digests and Latest, and retain the speed limitations documented there.
- **Salted Flutters Polaris** is the interface font. Current source keeps four WOFF2 weights (400, 500, 600, 700) in `src/assets/fonts/` and the unmodified license in `public/fonts/LICENSE.txt`. Vite fingerprints fonts into `dist/assets/` and copies the license to `dist/fonts/`; Tauri embeds both. Normal builds verify source and packaged hashes. A fresh checkout needs no separately installed font or Python to build.
- The four matching OTF sources and their license are project assets in `assets/fonts/polaris/`. To regenerate the WOFF2 files: install `fonttools[woff]`, then run `python scripts/build-fonts.py`. Missing sources fail explicitly. The separate local `Salted-Flutters-Polaris/` clone is optional and ignored.
- The stylesheet and Settings use Polaris. Settings shows the face as in use once it has loaded; only if the bundled face fails to load does it show a "Font details…" button, which opens https://github.com/ppolkadot/Salted-Flutters-Polaris. There is no optional proprietary font build path.
- Published release: https://github.com/ppolkadot/Claws/releases/tag/v1.0.3, targeting `e4f66695c3e618bceaac23e4bdbf6ddc3eb20807`. Installer: `Claws_1.0.3_x64-setup.exe`, 2,803,813 bytes, SHA-256 `5e6a59650246d8837a66f058101c82acbd6af5914b0f3a966d4445e0f4c14183`. GitHub's uploaded asset digest matched the local installer.
- The owner authorized publishing that release with existing access. No collaborator or permission changes were made. The cleanup described here is subsequent source work. A local installer was rebuilt successfully with it using npm run tauri build: 2804600 bytes, SHA-256 2f5caa5843506f362484a4ec1aff0db9088b830a8e709755d618a9bda739fe8d. The published GitHub release still contains the earlier installer.

### 1.0.4 release

The owner authorized reviewing, committing and publishing the latest cleanup as a new release. Version 1.0.4 includes the committed Polaris assets, corrected Settings font link, synchronized version files and updated documentation. The older 1.0.3 release above records the previous published artifact. The 1.0.4 installer is Claws_1.0.4_x64-setup.exe (2803353 bytes), SHA-256 f723526a69a4c9aee6411ef9be2b6f62084b07fea0329a4bc229f658fd6b44ba. Release URL: https://github.com/ppolkadot/Claws/releases/tag/v1.0.4.

Validation: all 35 backend tests passed; cargo clippy --locked -- -D warnings passed; an export containing only staged files passed npm ci, npm run build and byte-identical font regeneration. Font attribution and unmodified license copies were verified, and npm run tauri build produced the NSIS installer successfully. No fresh Windows installation or interactive download was exercised during this release check. The source Bold font reports weight 600 internally; the CSS face explicitly assigns it 700, and the build preserves upstream metadata.

### Published history cleanup

At the owner's request, main and all four release tags were rewritten to remove obsolete interface font references from historical files. Commit IDs changed; historical references in current documentation now use the rewritten IDs. Existing clones should be replaced with a fresh checkout to avoid restoring the old history. A recovery bundle is stored outside the project. Release installer files and their checksums are unchanged.

### Font spacing repair (included in the repository 1.0.4 installer and release 1.0.5)

The supplied CFF outline programs inherit oversized default advances that disagree with the proportional widths in hmtx. Chromium/WebView2 uses those outline advances, spreading interface letters apart despite normal CSS tracking. The font builder now expands subroutines and writes each glyph's intended hmtx advance into its outline program while preserving the drawing instructions and name metadata. Regenerated WOFF2 files include this repair; source OTF files remain untouched. At the time, font URLs carried a `?v=metrics-1` cache version; the current source instead relies on Vite content fingerprints (see the next section), so changed bytes always get a new URL.

The issue was reproduced in the browser Settings screen: the 28 px heading measured 341.828 px before the repair and 103.453 px afterward. Regression checks cover every bundled glyph's outline advance, sampled outline geometry and preserved font metadata. Run `python -m unittest discover -s scripts -p test_font_metrics.py`. This is subsequent work after the published 1.0.4 release. A local installer was rebuilt successfully with the corrected fonts: Claws_1.0.4_x64-setup.exe, 2769214 bytes, SHA-256 9aadc5fedde599b89fc53e199366caac465a4c3ddcf4032b0c7454f07efce955. The repaired installer and its SHA-256 file are stored in `installers/` and committed with the font-spacing fix, as requested by the owner. The published release asset has not been replaced.

### Recurring font report and prevention (working changes after 1.0.5)

The reported recurrence was reproduced in the installed app and traced to the original published 1.0.4 executable, SHA-256 `af86afed1a002fecf7224b80e6a97e4ebda9c62d8bdaa2365fbe517298c0248b`. It matched the executable extracted from the original GitHub 1.0.4 asset, rather than the repaired repository installer. The installed app was updated to the already-published 1.0.5 using the installer's `/S /UPDATE` mode after confirming no active downloads. The existing queue was backed up; its job IDs survived migration. The real installed Settings screen was visually checked and has correct proportional spacing. No font was installed globally and no user cache was deleted.

Preventive source changes use a private CSS alias (`Claws Polaris`) without `local()` substitution and import the committed WOFF2 files through Vite for automatic content fingerprints. `scripts/check-fonts.mjs` rejects unapproved font bytes both before a normal frontend build and in its final `dist` assets. The Python generator approves hashes only after checking every serialized CFF advance. Seven Node regressions cover font selection, build enforcement, corrupt source files, installed-font fallback, replaced packaged bytes, orphaned or legacy packaged font files, and running the check through a junction; two Python regressions retain metric/outline checks. DESIGN.md declares the alias as the same Polaris face; no design-hook suppression is needed. These preventive source changes are not part of the published 1.0.5 installer.

## Download reliability work included in 1.0.5 (branch `download-reliability`)

The first pass implemented the original plan's first release candidate (its Tasks 1–10 and 15). A review of that pass (`docs/CURRENT_PROBLEMS_AND_SPEED_REVIEW.md`) found five lifecycle defects, B1–B5, and the plan was rewritten as remaining work only. The second pass (2026-11-05) closed B1–B5, measured the speed question under controlled conditions and batched large queue additions. Its details are under "Remaining-work pass" below; where they differ, they supersede "What changed".

Not done:
- The real slow-source diagnosis: no affected URL was available. See `docs/performance/speed-diagnosis.md`.
- The remaining plan's Tasks 2–4 (a measured speed fix, managed processing, optional aria2c) depend on that diagnosis.
- Fresh Windows installation and upgrade testing. Source versions are now synchronized at 1.0.5; see the release record above for packaging status.

### Remaining-work pass (2026-11-05)

- **Required saves (B4).** `flush_queue` returns a `Result`; the queue writer fails a barrier as soon as a save attempt that began after it fails. Every start saves before yt-dlp is spawned. Cleanup deletes nothing unless the queue state and the `cleanup.json` record are both saved; `remove_job` returns an error and keeps the job when the record cannot be saved. Shutdown keeps cancelled files when it cannot save. `store::write_atomic` retries a rename refused for a moment (an antivirus opening the fresh file).
- **Attempts (B1).** Try Again on a cancelled job calls `start_new_attempt`: cancel, error and progress cleared, `execution` kept, `attempt` + 1, `workspace` cleared. Work folders are `<12 hex>` for attempt 0 and `<12 hex>-<n>` after; `WorkspaceRef::attempt` and the marker's `attempt` must match (both default to 0 for older data). Pending cleanups are kept per work folder. `remove_workspace` deletes the marker last, so a file still in use never leaves an unprovable folder.
- **Publication journal (B3).** Manifest schema 2 adds `moves`: per output, `Moving` (with a `FileIdentity`) is saved before the move and `Done` after; a side file whose name is taken is `Withheld`. The identity is the NTFS/ReFS volume serial and 128-bit file id (`GetFileInformationByHandleEx(FileIdInfo)`), else size and SHA-256. Recovery accepts a destination only if its identity matches. Schema 1 `published` entries still count. `WorkspaceRef` keeps its own schema (1).
- **Completion transaction (B2).** `finish` records the Library entry (`history::record` now returns a `Result`) before settling; only after the queue save succeeds is the work folder cleaned, never while a side file is withheld. A failed Library save leaves the job Failed with its manifest; Try Again goes through the `Published` shortcut and only records it. `restore_queue` records an unrecorded completion once (an existing entry is left alone) or leaves the job Failed.
- **Disk ledger (B5).** `AppState::disk_ledger` (`disk_budget::Ledger`) holds each running job's remaining demand: peak minus the work folder's bytes (sampled every 2 s), plus margin. Admission reads free space under its lock. Each `CLAWSPP|started` triggers a best-effort space recheck; it cannot gate FFmpeg before launch. Successful completed output wins over a late space notification, and a reaped process PID is never reused by that notification. Stopped jobs retain owned artifacts, which may be streams or merged output. The claim is released when the process exits. `AppState::free_space` is replaceable for tests.
- **Queue view.** Structural syncs build at most 40 new rows per frame (`ROWS_PER_FRAME`), including when the view opens.
- **Lifecycle tests** (`src-tauri/src/lifecycle_tests.rs`). The real downloader, writer and stores run through Tauri's mock runtime (`crate::Runtime` is `MockRuntime` under `cfg(test)`), with a disposable data folder and the fixture tool as yt-dlp. The job link selects its behaviour: `https://fixture.invalid/<ok|hang|fail>/<name>[/<planned bytes>]`. Saves are made to fail with a folder where the JSON file must go. The mock runtime needs the Common Controls v6 manifest, so `build.rs` links tauri-build's own manifest (`windows-app-manifest.xml`) into every target. Build the tool first: `cargo build --features test-tools --bin claws-fixture-tool`; without it these tests skip.
- **Benchmarks.** `download-fixtures.py` adds per-connection (`/capped`, `/capped-hls`) and server-wide (`--global-rate-kib`) caps and `--segments`. `download-bench.mjs` adds `--jobs`, `--downloader native|aria2c|ranges:N` (`ranges:N` is a reference strategy run by the script itself, not a Claws code path) and process-tree CPU. `processing-bench.mjs` times merge, metadata, combined and audio steps with ffmpeg.

### What changed

- **Frozen execution.** At first start a job saves `execution`:
  - output folder;
  - filename template;
  - metadata;
  - audio preference;
  - fragment concurrency;
  - transport.

  It is saved durably before yt-dlp starts, and resumes and retries reuse it. Sign-in and tool locations stay current.
- **Work folders.** yt-dlp writes only into `<output>\.claws-work\<first 12 hex of the job id>\`, which is hidden.
  - The folder holds `.claws-job.json`: the manifest, with the schema version, full job id, destination root, state, outputs and published paths.
  - The short name keeps paths under 260 characters.
  - Ownership requires all of: a correct name, a direct parent named `.claws-work`, no link or junction, and a marker with the same job id.
- **Publication.** The manifest is set to Ready with the media plus subtitle sidecars sharing its stem. Then `MoveFileExW` runs without `MOVEFILE_REPLACE_EXISTING`.
  - Each move is recorded, so an interrupted publication resumes.
  - A taken name fails the job with the files kept, and Try Again only publishes.
  - Destinations are reserved across jobs (`AppState::destinations`).
  - At startup, a job whose manifest says Published but whose completion was not saved is completed.
- **Cancel and remove.**
  - `cancelRequested` is set, the queue is flushed, and the cleanup is recorded in `cleanup.json`. Only then is the verified work folder deleted (8 tries, 250 ms apart).
  - A folder still in use stays in `cleanup.json` and is retried at startup.
  - Legacy 1.0.4 jobs delete only exact `<name>.part`, `.ytdl` and `.part-Frag<n>` files.
- **Queue file.** `{ schemaVersion: 2, revision, jobs }`.
  - The 1.0.4 bare list is read and copied once to `queue.pre-1.0.5.json`.
  - A file from a newer version is set aside with a notice.
  - Saves go through `queue_writer`, a single thread that coalesces routine changes for 250 ms. Flush barriers apply when a work folder is committed, before deleting cancelled files, after completion, and at shutdown.
- **Settings.** `schemaVersion: 2`. Files without it are migrated, and `audioSelection` becomes `best` (old behaviour). New installs get `preferMatching`.
- **yt-dlp policy** (`download_policy.rs`):
  - `--retries 10 --fragment-retries 10`, with exponential retry sleeps (http and fragment, 1–30 s);
  - `--socket-timeout 30`;
  - `--abort-on-unavailable-fragments`;
  - `--progress-delta 0.2`.
  - Failures are classified, and a cached probe is refreshed only for expired media.
- **Events.** Every job change bumps `revision` under the jobs lock.
  - `job-update` carries `{schemaVersion: 1, jobId, revision, job}`.
  - An enqueue is one `jobs-added` batch, and the enqueue reply is the same batch.
  - `job-removed` carries a revision.
  - `src/queue-events.ts` drops stale or duplicate events and keeps tombstones.
- **Progress.** Streams are identified by `%(info.format_id)s`, and `CLAWSFMT` carries the selected streams' exact and approximate sizes.
  - Progress is sum(downloaded)/sum(expected).
  - `progressKnown` is false while any size is unknown.
  - `progressEstimated` marks estimates.
  - The ETA covers transfer bytes only.
  - 100% is reached only after publication.
- **Disk budget.** Checked when the selection is known:
  - the peak is the streams ×2 if merged or rewritten, or the stream plus the converted output for audio;
  - minus bytes already in the work folder;
  - plus a margin of max(512 MiB, 10%);
  - against free space minus other jobs' reservations on the same volume.

  Unknown sizes are watched every 5 s or more, with a stop below 256 MiB.
- **Bounded output.** Lookup JSON is capped at 32 MiB, and the process tree is killed on overflow. Error output keeps its last 256 KiB, and lines are capped at 256 KiB.
- **Fast Save** (`options.fastSave`) turns off the metadata pass and embedded subtitles for one job.

Lock order is unchanged: store write lock, `running`, `jobs`, `live_jobs`. `pending_cleanup`, `destinations` and `disk_ledger` are leaf locks (free space is read while `disk_ledger` is held).

### Verified

- `cargo test`: 94 tests. They include real yt-dlp 2026.10.26 against `scripts/dev/download-fixtures.py`:
  - a missing HLS segment now fails (it exited 0 before);
  - 503s recover with backoff and intact bytes;
  - a dropped connection resumes to an identical file;
  - a kill ends a backoff;
  - the progress cap holds;
  - real output parses into exact byte totals;
  - audio selection was checked offline with `--simulate`.
- `npm run test:queue`: 10 reducer tests.
- `cargo clippy --all-targets -D warnings` and `npm run build` are clean.
- A real dev-build run driven over DevTools against the fixtures passed 23/23 checks:
  - a 1.0.4 queue loads and is backed up;
  - publish and work-folder removal;
  - pause, a settings change, then resume into the frozen folder;
  - cancel removes only the work folder;
  - a name conflict without overwriting, then Try Again;
  - a missing fragment;
  - duplicate refusal;
  - the queue view rows.

  The app data folder was backed up first and restored byte-identical afterwards.
- Queue view (`docs/performance/queue-ui.md`), with 500 jobs and 8 active:
  - adding them took 4.4 s of layout and style before, 73 ms after;
  - progress cost 729 ms before, 63 ms after;
  - long tasks during progress went from 39 to 0.

### Not verified

- No ffmpeg on this PC, so:
  - no merge, metadata or embedded-subtitle runs;
  - no ffprobe check that Prefer Matching copies without encoding.
- The disk-shortfall stop inside a running download. The decision logic is unit-tested.
- Live sites, an installed NSIS build, and the upgrade from an installed 1.0.4.

### Follow-up recovery and processing fixes (2026-11-05)

The published media is validated against the destination's existence and stored identity before publication retry or startup recovery. A missing/replaced media file stays Failed with its recovery record retained. Already-published subtitle sidecars can be renamed or deleted without blocking Try Again. Published completion is considered before stale cancellation during startup; cancellation intent is cleared once publication has committed even if Library persistence fails. Startup restores these jobs as Processing and verifies/records them on a background worker, so FAT/exFAT hashing does not block app setup; they are reserved against duplicate starts. The frontend subscribes before bootstrap and replays buffered events afterward so recovery completion is not lost. Hashing still costs disk time on those filesystems; a large-file USB startup has not been measured.

Processing-space notifications are explicitly best effort, preserve valid completed results and consult the current process registry before stopping a process. A strict pre-launch gate remains work for the managed processing pipeline. The speed diagnosis now limits its uncapped-throughput conclusion to standalone yt-dlp. The fixture's intentionally surviving child has a local Clippy allowance. Validation: 134 Rust tests, 11 queue tests, two font regression tests, a clean dependency install, the frontend build, Clippy with `test-tools` and warnings denied, and whitespace checks passed. These corrections are packaged in the 1.0.5 installer recorded above.

## Historical handoff (superseded where noted above)

## 1. What this is

Claws is a local Windows video downloader. You paste a link from almost any site, pick a quality, and the file is saved to disk. It is a **Rust (Tauri 2) app with a plain TypeScript + CSS frontend** that drives **yt-dlp** (site coverage) and **ffmpeg** (merging and conversion). The interface imitates Apple's design as closely as possible, with real desktop glass behind the window.

Repo: `https://github.com/ppolkadot/Claws`, branch `main`. The first build was committed as `97c93ed Claws 1.0.0`; the fix passes and features in sections 14 and 15 shipped as **1.0.1**, and the security pass in section 16 is **1.0.2**, the version at that time (`package.json`, `package-lock.json`, `Cargo.toml`, `Cargo.lock`, `tauri.conf.json`). See section 10.

## 2. What the owner asked for (binding requirements)

The first request, answered in a Q&A at the start:

- Rust local downloader; "put a link and download it no matter the platform, it must support all sites possible".
- Interface "as similar as possible with the one made by Apple", with a "big focus on blur effects" and a "glass appearance".
- Choices made by the owner: **yt-dlp + ffmpeg** as the engine, **Tauri + web frontend**, **Windows only for now**, and all four feature groups: format/quality picker, queue + playlists, history/library, settings + cookies.

Second request:

- Glass "must have desktop transparency simulating a liquid glass effect present on Apple devices; it must not display a background image as substitute".
- Use **Salted Flutters Polaris** as the main font (current requirement).
- Follow Apple's design philosophy, add fluid Apple-like animations, and avoid AI-looking design patterns.
- Make the sidebar collapsible.
- Focus on download speed, then look for any other optimizations.

Third request:

- Bundle the four required Polaris weights, keep the sources and license in the project, and document the build in this handoff.

The owner also asked for a "testing release" (the installer is built, see section 10) but has not answered whether to commit and publish it.

## 3. Architecture

### Backend: `src-tauri/src/`

| File | Responsibility |
| --- | --- |
| `lib.rs` | App setup: plugins (single-instance first, then dialog and opener), state, loads data files with recovery, restores the queue, prunes the probe cache, applies theme and window material, shows the window (4 s fallback), registers commands. On `RunEvent::Exit` it calls `downloader::shutdown`. |
| `commands.rs` | Every `#[tauri::command]`. `bootstrap` returns settings/jobs/history only; tool status is separate and loaded after first paint. File actions open only paths the app recorded, never paths sent by the UI. `open_font_page` opens the fixed Polaris project URL. |
| `downloader.rs` | Queue + concurrency (`pump`), building yt-dlp args, progress parsing, pause/resume/cancel/remove, partial-file cleanup, cached-probe retry. |
| `probe.rs` | `validate_url` (http/https only), runs `yt-dlp --dump-single-json --flat-playlist`, parses formats/heights/subtitles/playlist entries, and keeps the **probe cache**. |
| `tools.rs` | Finds, installs and updates yt-dlp (folder build) and ffmpeg, verifying each against its published SHA-256. `common_args` sets cookies, ffmpeg location and the Node JS runtime. PATH lookups are cached. |
| `window.rs` | Window material: **glass** (accent-policy blur-behind via `SetWindowCompositionAttribute`, loaded at runtime), **acrylic/mica/tabbed** (DWM system backdrop), **none**. The DWM frame is extended over the whole client area for every material except none. |
| `history.rs` | Library entries, local thumbnail cache (`%APPDATA%\dev.claws.downloader\thumbs`, our own uuid filenames, 5 MB cap). |
| `settings.rs` | Settings struct + `validate()`. Every value that reaches the yt-dlp command line is checked: browser allow-list, cookie file exists, filename template must be relative with no `..`. |
| `desktop.rs` | Quick access: the Quick Open global shortcut (toggles the window, emits `quick-open`), the notification-area icon (Open / Quit), open-at-login via `tauri-plugin-autostart` with `--background`. `apply()` runs at startup and on every settings save and returns a warning when something could not be applied. |
| `links.rs` | The link cache: `ProbeResult`s of downloaded links in `cache/links/<sha>.json`, 2000 entries / 1 year. Saved on `enqueue` (its `source`), refreshed by any later lookup of a known link. |
| `state.rs`, `store.rs`, `error.rs` | Shared state (short std mutexes, never held across await; the lock order is documented in `state.rs`). `engine` is a tokio RwLock: yt-dlp runs read it, installs write it. `store.rs` serialises every JSON write behind one lock, snapshots under that lock, and moves damaged files aside with a notice. `error::notify` shows background failures as a toast. |

**yt-dlp invocation rules:**
- Arguments are always passed as an array, never through a shell.
- The URL always goes after `--`, or `--load-info-json <cache file>` is used instead of the URL.
- Progress comes from our own markers: `CLAWS|` download progress on **stdout**, and `CLAWSPP|` post-processing on **stderr**. That stdout/stderr split was verified empirically. `CLAWSFMT|` gives the stream count and `CLAWSDONE|` the final path.
- Windows specifics: `CREATE_NO_WINDOW`, `PYTHONIOENCODING=utf-8`, and kill with `taskkill /T /F`, because yt-dlp.exe launches child processes.

**Data:** `%APPDATA%\dev.claws.downloader\` holds `settings.json`, `history.json`, `queue.json` (unfinished jobs, restored as paused), `thumbs\`, `cache\probe\` (20-minute info cache) and `bin\yt-dlp\yt-dlp.exe` (folder build). Managed ffmpeg would go in `bin\`; it currently comes from PATH via winget.

### Frontend: `src/`

| File | Responsibility |
| --- | --- |
| `main.ts` | Boot: springs, dev mock (browser only), bootstrap, appearance, shell, events. Then **background** tool check plus a one-time yt-dlp folder-build migration, and waits for the font (800 ms cap) before `app_ready`. |
| `store.ts` | Central state + pub/sub topics; debounced settings save; `toolsKnown` / `toolsReady`. |
| `api.ts` | Typed `invoke` wrappers and the shared types. |
| `views/shell.ts` | Fixed window chrome (traffic lights + sidebar toggle), floating collapsible sidebar (state in `localStorage` key `claws.sidebar`), travelling selection indicator, large title that collapses into the toolbar on scroll (IntersectionObserver), routing with View Transitions, shortcuts (**Ctrl+Alt+S** sidebar, **Ctrl+1–4** sections). |
| `views/download.ts` | URL bar, first-run engine install, probe → preview → options (type/quality/audio format/subtitles), playlist selection, "Added ✓" confirmation, Recent list. |
| `views/queue.ts` | Keyed rows patched in place; rows spring in and out (`expandIn` / `collapseOut`). |
| `views/library.ts` | Tile grid with keyed reuse, debounced search, remove/delete via a confirmation sheet. |
| `views/settings.ts` | Downloads, Appearance (theme, material, Transparency, Font status), Sign-in (cookies), Tools. |
| `ui/motion.ts` | Spring physics (response/damping, SwiftUI-style) sampled into CSS `linear()` and published as `--spring-{smooth,snappy,bouncy}` and `--spring-*-d`. |
| `ui/components.ts`, `ui/layout.ts`, `ui/icons.ts`, `ui/dom.ts` | Controls, rows/groups/progress/spinner, the authored SVG icon set (static strings only), and `h()`. Remote text is always inserted as text nodes. |
| `styles/fonts.css` | `@font-face` for Salted Flutters Polaris 400/500/600/700 from `/fonts/*.woff2`. |
| `styles/base.css` | Tokens, materials, shell. |
| `styles/components.css` | Glass panel rim, controls, toasts, sheet. |
| `styles/views.css` | The four screens. |
| `mock.ts` | Synthetic data for `npm run dev` in a plain browser (dev builds only). |

## 4. Design system (the short version; `DESIGN.md` has the full record)

- **No painted background.** The window is clear desktop glass. The UI lays tint materials over it, from thinnest to densest: **window tint** → **regular** (grouped panels) → **thick** (floating sidebar) → **chrome** (scrolled toolbar, sheets, toasts). `--gi` is the Transparency setting (0–1) and scales the tint alphas. Solid mode and `prefers-reduced-transparency` use the opaque `--solid-base`.
- **Liquid edge:** `.glass::before` is a masked 1px gradient ring, bright at the top-left.
- **Panels do not use `backdrop-filter`:** there is nothing in-page behind them to blur. Only the chrome surfaces and the library duration badge blur in-app content.
- **Rule:** never put an opacity, filter or mask animation on an ancestor of an element that uses `backdrop-filter`. That makes a backdrop root and flattens the blur. Route changes use View Transitions snapshots for this reason.
- **Apple conventions used:**
  - inset grouped rows with separators inset from the leading edge;
  - empty states as symbol + title + text;
  - capsule primary actions;
  - Cancel focused by default in destructive sheets;
  - traffic lights fixed while the sidebar slides away;
  - large title → toolbar title.
- **Avoided on purpose:** colour blobs and orbs, gradient text, glowing halos, icon tiles in empty states, staggered fade-up reveals, eyebrow labels.
- **Accent:** `#0071e3` behind white text (4.7:1), with darker hover shades `#0066cc` and `#c4261b`. The confirmation green is `#1d7a36`.

## 5. Fonts

- Current font: **Salted Flutters Polaris**. Four OTF sources and `LICENSE.txt` are committed under `assets/fonts/polaris/`; four WOFF2 files are committed under `src/assets/fonts/` and the license under `public/fonts/`.
- Normal builds use the committed web fonts. Optional regeneration uses `pip install "fonttools[woff]"` and `python scripts/build-fonts.py`.
- The script preserves name records and OpenType features, subsets the supported character ranges, and fails if required sources or the license are missing.
- Vite fingerprints the web fonts into `dist/assets/` and copies the license to `dist/fonts/`; Tauri embeds them in the application. No separate font installation is required.

## 6. Speed work and measurements (this PC)

| What | Before | After | How |
| --- | --- | --- | --- |
| Click Download → first progress | ~4.7 s | **0.77 s** | probe cache + `--load-info-json` |
| yt-dlp startup | ~1.4 s | ~0.4 s | folder build `yt-dlp_win.zip` (checksum-verified) instead of the single-file exe |
| Work before first paint | ~500 ms | ~2 ms | tool version checks moved out of `bootstrap`, run in parallel in the background |
| Per-stream connections | 4 | 8 | measured: 4 already saturates this line (~55–70 MB/s); 8 helps high-latency links and HLS |

Measured and **rejected**:
- More than 8 connections: no gain.
- Disabling ffmpeg's `+faststart` rewrite: no measurable difference; post-processing time is dominated by disk noise.

The remaining lever is **Embed metadata**, which rewrites the file once more (0.7–2.5 s on a 170 MB file). It's a setting, and its hint says so.

The cached-probe path retries once with a fresh extraction if a cached run fails on its own, since media links expire.

## 7. Bugs found and fixed during the session

- Post-processing markers are written to stderr, not stdout, so the "Processing" state never showed. Both streams are now parsed live.
- Cancel left `.part` files behind after pause → resume → quick cancel. Partial files are now tracked on the job and cleaned for running, paused, failed and removed jobs, including `.part-FragN` and `.ytdl` files.
- The window never blurred the desktop:
  - the system Acrylic goes flat grey when the window is inactive;
  - for the clear blur, the DWM frame was not extended.
  - Fixed with the "glass" material, now the default.
- A bare "error sending request" message on a transient network failure. Errors now include the cause chain.
- Existing installs never moved to the yt-dlp folder build. There's now an automatic one-time migration, and the update check also reinstalls the single-file build.
- Review fixes from the documenter pass:
  - sidebar material denser than the panels;
  - reduced transparency now makes the window opaque;
  - white-text contrast on hover and confirmation states;
  - the slider variable no longer shadows `--fill`;
  - the sheet blur uses the shared token.

## 8. Verified vs not verified

**Verified in the running app, through the DevTools driver and guarded native captures:**
- probe, download, queue states;
- pause (process tree killed), resume (continues the `.part` file), cancel while running or paused (leftovers removed);
- Clear Finished, library, settings persistence;
- light, dark and Solid themes, and Transparency from 0 to 100;
- desktop glass over a known test pattern;
- sidebar collapse and the shortcuts;
- the scrolled toolbar;
- the confirmation sheet;
- the cached-probe speed path and the yt-dlp migration;
- The font available in that historical build loaded successfully; current Polaris packaging checks are recorded above.

**Not verified:**
- sites other than YouTube;
- cookie import from browsers;
- playlist downloads end to end;
- the ffmpeg installer;
- the yt-dlp update button with a newer release available;
- the installed NSIS build (built, never installed or run);
- behaviour on other GPUs or Windows builds;
- window resize and drag performance with the glass material, which I can't measure from here.

**Known limits:**
- A webview can't see the desktop pixels behind it, so Apple-style *refraction* of the desktop is impossible. The blur comes from Windows; the liquid look comes from the tints and rims.
- Windows' clear blur radius is fixed by the OS.

## 9. Environment facts (this machine)

- Windows 11 Pro 25H2, build 26200, console session.
- GPU: NVIDIA GeForce GT 340.
- System Acrylic flattens when the window is inactive. The accent blur-behind works.
- Rust 1.99, Node 24.19, WebView2 153.
- ffmpeg 9.0.2 from winget, on PATH.
- Disk: Kingston A400 SATA SSD.
- Network saturates at roughly 55–70 MB/s.

**Owner data currently in the app (do not delete):**
- The owner's own library entries.
- Settings they chose: Transparency 100%, simultaneous downloads 8, embed metadata on, theme System, material Glass.

## 10. Build, release and the open decision

```powershell
npm install
python scripts/build-fonts.py      # optional: regenerate from committed Polaris sources
npm run tauri dev                  # dev with hot reload
npm run tauri build                # NSIS installer
```

- The installer goes to `src-tauri/target/release/bundle/nsis/Claws_0.1.0_x64-setup.exe`.
- It is unsigned, so SmartScreen warns on first run.
- Current installers include the bundled Polaris web fonts and license.
- `npm run build` runs `scripts/check-fonts.mjs`, `tsc`, Vite, then `scripts/check-fonts.mjs --dist` on the bundle.
- `cargo clippy` was clean at the end of the session.

**Release:** current source imports Polaris from `src/assets/fonts/`, with the license in `public/fonts/`. Keep both in release builds. Earlier 1.0.4 publication is recorded at the top of this file.

## 11. Testing workflow (`scripts/dev/`)

The real WebView2 app can be driven over the local DevTools port. Never leave it running afterwards.

1. `npx tauri dev --config scripts/dev/debug-config.json`. This adds `--remote-debugging-port=9222` for the main window, as an overlay; the project config is untouched. The environment variable route does **not** work.
2. `node scripts/dev/cdp.mjs eval "<js>"`, `wait "<predicate>" [ms]`, `shot out.png`.
   - `window.__TAURI_INTERNALS__.invoke(...)` reaches the real backend.
   - `shot` is unreliable while the window is minimized, because the renderer throttles. Check `IsIconic` first.
   - PowerShell eats `\"` and backticks inside evals; use single quotes in the JS.
3. `scripts/dev/glass-shot.ps1 -Out x.png` shows the real DWM composite:
   - it places a test pattern behind Claws;
   - lifts Claws topmost;
   - **refuses to capture if any other window overlaps Claws**;
   - captures only Claws' rectangle.
   - **Privacy:** never capture by screen region without this guard. Twice this session an unguarded capture caught another app's window; both images were deleted.
4. `scripts/dev/blur-probe.ps1`: a Tauri-independent check that this PC renders DWM blur at all.
5. `scripts/dev/phases.mjs <yt-dlp> <info.json> <outdir> [args]`: times each download phase (transfer, merger, metadata).
6. Clean up afterwards:
   - stop the `claws` process and the node/cargo processes whose command line contains `claws`;
   - delete test files from `Downloads\Claws`;
   - delete test library entries with `delete_history`, keeping the owner's;
   - restore any settings you changed.
7. `tauri dev` restarts the app on Rust edits. Wait for a new `claws` process with a StartTime after your edit. Foreground sleeps are blocked in this harness, so use condition loops.

## 12. Suggested next steps

- The work from sections 14–16 shipped in earlier releases. Current packaging and release status are recorded at the top of this file; retain `src/assets/fonts/` and `public/fonts/LICENSE.txt` in builds.
- Test a non-YouTube site (Vimeo, an HLS site), a playlist download, and Firefox cookie import.
- Install and run the NSIS build once.
- Consider:
  - Apple-style video thumbnails with a hover scrubber in the Library;
  - a menu-bar-style tray icon;
  - automatic yt-dlp update checks;
  - code signing for releases.
- If the design changes, re-run the impeccable documenter so `DESIGN.md` and `.impeccable/design.json` stay true to the code.

## 13. Project documents

- `PRODUCT.md`: product truth. Users, purpose, constraints; inferred facts are labelled.
- `DESIGN.md`: the design system as built (refreshed after the glass redesign). Its machine sidecar `.impeccable/design.json` is gitignored along with the rest of `.impeccable/`.
- `README.md`: user-facing overview, fonts, speed notes, development.
- `docs/HANDOFF.md`: this file.

## 14. Bug-fix pass (2026-11-04)

A review listed concrete defects; all of them were fixed, along with a few found while reading the code.

| Problem | Fix |
| --- | --- |
| Settings edits lost: an older save's response replaced newer edits (reproduced). | `store.ts` gives every edit a revision. Saves run one at a time on a chain, and a response is only adopted when no newer edit happened. A rejected save reverts just the fields it changed that were not edited since, then emits `settingsReverted`; the Settings view rebuilds on it. Saves are flushed before the window closes (`onCloseRequested`, 1.5 s cap, needs `core:window:allow-destroy`). |
| JSON writes raced: one fixed temp name per file, stale snapshots could win, errors were ignored. | `store::save_snapshot` takes the snapshot and writes it under one global lock, through a uniquely named temp file. Queue and library save failures raise a `notice` toast. Leftover temp files are swept at startup. |
| Partial-file names lived only in memory until the job ended. | `handle_line` persists the queue as soon as a new file name appears. |
| Pause, cancel and resume could act on half-settled jobs; a pause racing a successful exit showed a finished file as paused. | All three run under the `running` lock. `execute` registers its process under it, and `finish` keeps the entry until cleanup is done, then removes it and sets the final status in the same critical section. A successful exit always wins. Cancel is never downgraded to pause. A retry keeps an intent that arrived between attempts. Removing a running job hands its leftovers to `finish`. Queue buttons disable until the status changes. |
| Choosing Browser after File used the file path as the browser name; choosing File saved an empty path. | The Settings view remembers the last browser. File only takes effect once a file is chosen, and cancelling the picker shows the saved source again. |
| "Delete File Too" removed the entry even when the file couldn't be deleted. | The file is deleted first; on failure (other than not-found) the entry stays and the error is shown. |
| Reduced transparency stayed translucent: the media query lost the cascade. | Overrides moved after the rules they replace and extended to panels and sidebar, matching Solid. Normal glass rules are untouched. |
| Installs were not coordinated with probes, downloads or each other; a failed update could leave no yt-dlp. | Installs take the `engine` write lock with `try_write` and fail at once if anything uses yt-dlp. Downloads and probes hold the read lock, so nothing starts on a half-installed tool. The yt-dlp folder swap keeps a backup and restores it on failure. ffmpeg and ffprobe are both staged before either replaces the old one. |
| Download buttons stayed enabled during enqueue (duplicate jobs). | An `enqueuing` guard, and the button is disabled. |
| Ctrl+V only worked on the Download view. | A global paste handler in `main.ts` routes links through `store.pasteLink` to the Download view from anywhere. |
| Flat-playlist entries with only an id were silently dropped. | They are kept with `playlistIndex`; the job downloads that item from the playlist URL (`--yes-playlist --playlist-items N`, never from the probe cache). |
| Playlists only offered "Best". | Playlists offer the standard height caps (4K to 360p). |
| Thumbnails were fully buffered before the 5 MB check. | Content-Length is checked, then the body is streamed with a cap. Artwork for an entry deleted meanwhile is discarded. |
| Corrupt JSON silently became defaults. | `store::load_or_recover` renames it to `<name>.corrupt-<time>.json` and the UI shows a notice. Loaded settings go through `Settings::repair`. |
| No shutdown coordination or single-instance protection. | `tauri-plugin-single-instance` focuses the running window. On exit every yt-dlp tree is killed (waiting for `taskkill`), cancelled jobs' leftovers are removed, and the queue is saved as paused. |
| JS runtime availability was invisible. | `ToolsStatus.jsRuntime`, shown in Settings → Tools. |
| Desktop-blur failures were swallowed; on Windows 10 the missing backdrop attribute aborted glass entirely. | The backdrop-type attribute only fails the call for Acrylic or Mica. An accent-policy failure is reported, as a startup notice or the `warning` of `save_settings`, which now returns `{ settings, warning }`. |
| Subtitle downloads counted as a media stream, so progress jumped to 50%. | Side files (subtitles, thumbnails, info JSON) are tracked for cleanup but don't advance the stream counter. |
| An installed font was hidden when its bundled files were absent. | Historical fix: `local()` fallbacks after bundled `url()` entries. |

**Tests:** `cargo test` in `src-tauri/` runs 18 unit tests:
- progress maths;
- intent precedence;
- partial-file cleanup;
- URL validation;
- playlist parsing;
- error text;
- settings validation and repair;
- corrupt-file recovery;
- concurrent saves;
- yt-dlp swap and rollback.

The settings race was verified against the real `store.ts` with an esbuild harness and a fake `api`, including the reported 20 → 90 case.

**Still not verified in the running app:** the items in section 8, plus the single-instance focus and the exit-time process cleanup.

## 15. Third pass (2026-11-04): shutdown, rollback, quick access, link cache

**Review findings fixed:**
- **P1, shutdown could start new downloads.** `AppState::shutting_down` is set before anything else. `pump` and `enqueue` refuse once it is set. `execute` spawns and registers its process under the `running` lock after checking the flag under that same lock, so shutdown's snapshot always includes it or it never starts. The cached-probe retry is skipped while closing. Shutdown then waits (up to 5 s) for `AppState::workers` to reach zero before its fix-up and save.
- **P2, lookups were not stopped on exit.** `probe::probe` registers its process in `AppState::probes` under the same flag check, and shutdown kills those trees too. A timed-out lookup now kills its whole tree before the child is dropped; `kill_on_drop` alone orphaned the interpreter.
- **P2, ffmpeg had no rollback.** `tools::replace_all` renames each existing executable to `.old` (allowed even while it runs), moves the new one in, and on any failure restores every target already replaced.

**Also fixed:**
- `store.runTool` dropped a request when another tool was busy, so a first-run yt-dlp install could be skipped. Installs now queue, and a tool already waiting is not queued twice.
- **P1, duplicate workers.** Pause → Resume while a job's worker waited behind a tool install let `pump` start a second worker on the same job; the extra process escaped every map. `AppState::live_jobs` now keeps one worker per job. A worker frees its job just before its final `pump`.
- **P2, pid reuse.** `Running.pid` is cleared once `child.wait()` returns, before the handle closes, so a late Cancel/Remove/Quit never `taskkill`s a reused pid.
- **P3, version checks.** `tools::status` holds the engine read lock and its version runs use `kill_on_drop`.
- **P3, quitting mid-install.** `tools::recover_bin` at startup puts back a tool left as `.old` and removes `.part` leftovers.
- **P3, probe cache growth.** The probe cache is also pruned (at most every 5 minutes) as entries are written, not only at startup.

The P1 duplicate-worker, P2 pid-reuse and three P3 findings came from an independent review of this pass, which also confirmed the shutdown, lock-order, `replace_all` and link-cache logic.

**New features:**
- **Quick Open** (Settings → Quick Access). A system-wide shortcut, Ctrl+Alt+D by default; Ctrl+Alt+Space, Ctrl+Shift+Space or off are the other choices.
  - It brings Claws forward and looks up a link on the clipboard (not the same one twice in a row); pressed while Claws is in front, it puts Claws away.
  - Registration conflicts come back as a warning.
- **Keep running when closed** (default on). Closing hides the window to the notification area (`onCloseRequested` prevents the close, needs `core:window:allow-hide`). Ctrl+Q and the icon's Quit really exit, through `quit_app` → `RunEvent::Exit` → `shutdown`.
- **Open at login** (default off). Starts with `--background`, which hides the window until asked, but only when Keep Running is on, so the window can never be lost.
  - Note: toggling it in a dev build registers the dev executable.
- **Link cache.** A lookup of a link downloaded before shows its saved details at once ("Checking for changes…") while a fresh lookup runs.
  - The view only redraws if the details changed, keeping the user's choices.
  - A failed refresh keeps the saved details and says so.
  - The preview notes when a link was downloaded before.
  - Recent rows reopen their link.

**Tests:** 26 Rust unit tests:
- the earlier ones;
- `replace_all` (whole-set swap and restore-on-failure) and `recover_bin`;
- link cache save, read, damage and prune;
- shortcut parsing and labels.

## 16. Security-report pass (2026-11-04): the `zip` crate

Three advisory write-ups were dropped in the repo root as `sec-report-1.txt` to `sec-report-3.txt`. All three describe the `zip` crate, which Claws uses to unpack the yt-dlp and ffmpeg downloads. The files are still untracked; commit, move to `docs/` or delete them as preferred.

| Report | Problem | Reachable in Claws? |
| --- | --- | --- |
| 1 | AES entries: the authentication code is skipped when an entry ends early or is empty, so tampered data is accepted | No. Claws never decrypts. The code was still compiled in. |
| 2 | `ZipArchive::extract()` allocates from a declared symlink size and aborts the process | No. Claws never calls `extract()`; it copies entries itself. It did write symlink entries as plain files. |
| 3 | `ZipStreamReader` trusts local headers while `ZipArchive` trusts the central directory, so an archive can show two different file lists | No. Claws only uses `ZipArchive`. |

None was exploitable here, and both archives are also checked against their publisher's SHA-256 before they are opened. They were fixed anyway:

- **Upgrade:** `zip` 4.6.1 to **9.3.0**, past the fixes for reports 2 (9.0.0) and 3 (9.0.5). Report 1's fix (a short read is an `UnexpectedEof`) is present in 9.3.0's source.
- **Smaller attack surface:** `default-features = false, features = ["deflate"]`. The real archives use only stored and deflate entries (checked with Python on the actual files: none encrypted, no symlinks). This removes the AES, zstd, bzip2, lzma and ppmd code from the build, 11 crates in all, so report 1's code is no longer compiled.
- **Stricter extraction in `tools.rs`:**
  - `check_entry` refuses encrypted entries and symlinks.
  - `copy_limited` counts the bytes actually produced and stops at the limit, so a lying declared size is harmless.
  - `Limits` caps entry count (2,000), per-entry size (256 MiB) and total size (512 MiB) for yt-dlp; ffmpeg entries are capped at 512 MiB each.
  - The ffmpeg unpacking moved into `stage_ffmpeg`, so it can be tested.
- `enclosed_name()` in zip 9 already skips `..`, absolute paths, `:` (NTFS streams) and Windows device names; this was verified with a throwaway probe. An earlier extra `:` check of mine was dead code and was removed.

**Verified:** the hardened extraction was run on the real, checksum-verified archives (143 yt-dlp files; ffmpeg and ffprobe at about 170 MB each). The extracted yt-dlp reports its version and the extracted ffmpeg runs. Tests: 35 pass (9 new, covering ordinary extraction, links, traversal, hidden streams and device names, per-entry/total/count caps, and the ffmpeg staging). `cargo clippy --all-targets` is clean and `npm audit` finds nothing.

**`cargo audit`:** no vulnerabilities. It lists 5 informational notices (`proc-macro-error`, `zbus_names`, `zvariant`, `zvariant_derive` as unmaintained; `glib` as unsound). `cargo tree --target x86_64-pc-windows-msvc` finds none of them in the Windows build; they are Linux-only dependencies of Tauri in the lockfile.

**Not covered:** the reports' proof-of-concept archives were not run; the reachability analysis is from reading the code and the upstream write-ups. The installed app was not re-run after this change, only the extraction code against the real archives.

## 17. Historical 1.0.2 installer verification (2026-11-04)

Archived artifact: `src-tauri/target/release/bundle/nsis/Claws_1.0.2_x64-setup.exe`, built 2026-11-04 09:52, SHA-256 `A0F8AAFA69F98B1172BB8020E5BB098904B92268785A0CBE1DCA806F31C91AD3`. This predates the current Polaris packaging.

That session checked embedded font delivery through a release build with a temporary DevTools port. The page origin was `http://tauri.localhost/`, all four font weights loaded, and interface elements used embedded font files. The final installer binary contained the expected asset paths and no debugging-port argument. Those observations describe the historical build, not a runtime verification of the current installer.

For the current release, use the checks and artifact details at the top of this file. The filename template field intentionally uses a monospace font.

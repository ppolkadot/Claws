# Claws: handoff

Everything a new session (human or AI) needs to pick this project up. Lives at `docs/HANDOFF.md`. Last updated 2026-11-03.

## 1. What this is

Claws is a local Windows video downloader. You paste a link from almost any site, pick a quality, and the file is saved to disk. It is a **Rust (Tauri 2) app with a plain TypeScript + CSS frontend** that drives **yt-dlp** (site coverage) and **ffmpeg** (merging and conversion). The interface imitates Apple's design as closely as possible, with real desktop glass behind the window.

Repo: `https://github.com/ppolkadot/Claws`, branch `main`. **Nothing from this work is committed yet.** The only commit is `c31405f Initial commit` (README + LICENSE). See section 10.

## 2. What the owner asked for (binding requirements)

The first request, answered in a Q&A at the start:

- Rust local downloader; "put a link and download it no matter the platform, it must support all sites possible".
- Interface "as similar as possible with the one made by Apple", with a "big focus on blur effects" and a "glass appearance".
- Choices made by the owner: **yt-dlp + ffmpeg** as the engine, **Tauri + web frontend**, **Windows only for now**, and all four feature groups: format/quality picker, queue + playlists, history/library, settings + cookies.

Second request:

- Glass "must have desktop transparency simulating a liquid glass effect present on Apple devices; it must not display a background image as substitute".
- Use Apple's **Legacy UI Display** as the main font.
- Follow Apple's design philosophy, add fluid Apple-like animations, and avoid AI-looking design patterns.
- Make the sidebar collapsible.
- Focus on download speed, then look for any other optimizations.

Third request:

- Use the Legacy UI Display files the owner dropped into the repo as the default font. Choose the best-fitting weights, relocate the folder where appropriate, and write this handoff.

The owner also asked for a "testing release" (the installer is built, see section 10) but has not answered whether to commit and publish it.

## 3. Architecture

### Backend: `src-tauri/src/`

| File | Responsibility |
| --- | --- |
| `lib.rs` | App setup: plugins (dialog, opener), state, restores the queue, prunes the probe cache, applies theme and window material, shows the window (4 s fallback), registers commands. |
| `commands.rs` | Every `#[tauri::command]`. `bootstrap` returns settings/jobs/history only; tool status is separate and loaded after first paint. File actions open only paths the app recorded, never paths sent by the UI. `open_font_page` opens a fixed Apple URL. |
| `downloader.rs` | Queue + concurrency (`pump`), building yt-dlp args, progress parsing, pause/resume/cancel/remove, partial-file cleanup, cached-probe retry. |
| `probe.rs` | `validate_url` (http/https only), runs `yt-dlp --dump-single-json --flat-playlist`, parses formats/heights/subtitles/playlist entries, and keeps the **probe cache**. |
| `tools.rs` | Finds, installs and updates yt-dlp (folder build) and ffmpeg, verifying each against its published SHA-256. `common_args` sets cookies, ffmpeg location and the Node JS runtime. PATH lookups are cached. |
| `window.rs` | Window material: **glass** (accent-policy blur-behind via `SetWindowCompositionAttribute`, loaded at runtime), **acrylic/mica/tabbed** (DWM system backdrop), **none**. The DWM frame is extended over the whole client area for every material except none. |
| `history.rs` | Library entries, local thumbnail cache (`%APPDATA%\dev.claws.downloader\thumbs`, our own uuid filenames, 5 MB cap). |
| `settings.rs` | Settings struct + `validate()`. Every value that reaches the yt-dlp command line is checked: browser allow-list, cookie file exists, filename template must be relative with no `..`. |
| `state.rs`, `store.rs`, `error.rs` | Shared state (short std mutexes, never held across await), atomic JSON save, error type (reqwest errors include their full cause chain). |

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
| `styles/fonts.css` | `@font-face` for Legacy UI Display 400/500/600/700 from `/fonts/*.woff2`. |
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

- Source files: `assets/fonts/Legacy UI Display/`. The full family of 18 `.otf` files supplied by the owner. I removed three byte-identical duplicates: `Thin(1)`, `Ultralight(1)` and a root-level copy of Regular.
- `scripts/build-fonts.py` subsets **Regular 400, Medium 500, Semibold 600 and Bold 700** to Latin, Greek, Cyrillic and punctuation, keeping all OpenType features. Output goes to `public/fonts/` as WOFF2, about 107–128 KB each, down from about 2.2 MB. Vite copies it into `dist/fonts/`.
  - Needs `pip install "fonttools[woff]"`.
  - The dropped `MERG`, `meta` and `trak` tables are Apple-specific and unused by browsers.
- Every CSS weight maps to a shipped file. `650` was replaced with `600`.
- Body tracking is `+0.008em`, because Display is spaced for large sizes.
- **Licence:**
  - Apple's Legacy UI licence only covers mock-ups for Apple platforms. Both font folders are **gitignored** so the repo never redistributes them.
  - **The installer built from this machine contains the font.** Publishing it would redistribute Legacy UI. That is the owner's decision; they were told.
  - Without the files the build still works and falls back to Segoe UI Variable.

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
- Legacy UI loaded (Settings reports "Built into this copy of Claws").

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
python scripts/build-fonts.py      # once, needs the Legacy UI source files
npm run tauri dev                  # dev with hot reload
npm run tauri build                # NSIS installer
```

- The installer goes to `src-tauri/target/release/bundle/nsis/Claws_0.1.0_x64-setup.exe`.
- It is unsigned, so SmartScreen warns on first run.
- It contains the bundled Legacy UI.
- `npm run build` runs `tsc` and Vite.
- `cargo clippy` was clean at the end of the session.

**Pending owner decision:** whether to commit and push, and whether to publish a GitHub pre-release (proposed tag `v0.1.0-test.1`). The owner asked for a "testing release", but neither question has been answered. Do not push or publish without explicit confirmation.
- The repo may be public; committing makes all source public.
- The fonts stay out of git either way.
- An uploaded installer would include Legacy UI.

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

- Get the owner's answer on commit, push and pre-release (section 10).
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

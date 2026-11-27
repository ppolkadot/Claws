# Wallpaper Engine: scene recording, export progress, 4K, and SteamCMD removal

**Status:** Prompt for the implementing agent. Not started. Written 2026-11-26.

**You are implementing** the next Wallpaper Engine release of Claws on Windows (Tauri v2, Rust backend, plain TypeScript frontend):

1. Remove SteamCMD. The Steam library becomes the only source.
2. Add a progress tracker to every wallpaper export (copy, convert, ZIP, record). It shows time left while running and the total time when done.
3. Record scene and web wallpapers to MP4 (and a still to PNG) by having Wallpaper Engine play them in a window and capturing that window with FFmpeg.
4. Support 4K recording where the display allows it. Keep 4K video wallpapers at full resolution.
5. Save H.264 video wallpapers by copying the original file instead of re-encoding.

**Read first:** [the study](../../wallpaper-engine-local-library-study.md) (all measurements behind this plan), [DESIGN.md](../../../DESIGN.md) (Progress and activity, Motion, Do's and Don'ts), [PRODUCT.md](../../../PRODUCT.md) (Honest status), [HANDOFF](../../HANDOFF.md) (schema contracts), and the current code in `src-tauri/src/workshop.rs`, `workshop_media.rs`, `tools.rs`, `settings.rs`, `src/views/workshop.ts`, `src/workshop-session.ts`, `src/ui/layout.ts`, `src/views/queue.ts`, `src/format.ts`.

## Evidence this plan rests on (measured on the owner's PC)

| Fact | Consequence |
|---|---|
| Steam library and SteamCMD produce identical project folders; both need an account that owns Wallpaper Engine | SteamCMD adds setup and failure modes and gains nothing for an owner with Steam |
| 216 local items: 84 video (all `.mp4`, 82 H.264), 122 scene (`scene.pkg`, no video inside), 6 web, 4 presets | Copy video, record scenes, ZIP the rest |
| Videos run up to 64 min and 5.6 GB. A copy takes 1–7 s. The current re-encode takes about 6 min for 4K, and GIF about 78 min for 4K or about 3 h for 64 min, all with an indeterminate bar | Progress and estimates are required, and GIF needs limits |
| `wallpaper64.exe -control openWallpaper -file <project.json> -playInWindow <name> -width W -height H` and `-control closeWallpaper -playInWindow <name>` work. The window appears in 0.13–0.4 s | Recording pipeline |
| Claws' managed FFmpeg (yt-dlp build) has `gfxcapture` | No new tool |
| 20 scenes recorded: no moving scene froze. Still scenes are predictable from `scene.json` plus sprite-sheet `.tex` (`TEXS`) data | Detect still scenes before recording |
| A 1.2 GB scene showed black for 1.6 s | Capture a discarded pre-roll |
| 60 s at 1080p: 1,800 frames, no drops, 62.9 s total (0.3 s window, 2 s pre-roll, 0.6 s capture start, 0.05 s finish) | Recording time is predictable |
| A framed window is clamped to the virtual screen (here 3860×1106 max: two 1080p screens side by side) and loses its 13 px caption. A **borderless** window gave exactly 1920×1080 | Make the window borderless. 4K needs a display area at least 2160 px tall |
| 50 of 121 fixed-size scenes are authored at 4K or larger. Aspect ratios vary (21:9, 3:2) | Offer sizes up to the authored size, in the scene's aspect ratio |
| libx264 veryfast encodes real 4K at 0.84× real time. `h264_nvenc`, `hevc_nvenc` and `h264_mf` work on the owner's RTX 3060 Ti | 4K recording uses a hardware encoder |
| `wallpaper64.exe` 2.8.0.868, Authenticode valid, signer `Skutta Software GmbH`. `steamapps\appmanifest_431960.acf` gives `installdir` `wallpaper_engine` | How to find and trust Wallpaper Engine |

## Ground rules

- Keep app ID `dev.claws.downloader`, settings schema 2 and queue schema 3. New settings fields are optional with serde defaults and survive `Settings::repair`. `Settings` has no `deny_unknown_fields`, so removed SteamCMD fields in old files are ignored. Add a test that a 1.0.9e settings file with `steamcmdPath`, `steamAccount` and `removeSteamcmdCopy` loads.
- Rust runs every process with argument arrays, never a shell. Untrusted inputs are bounded: package entry counts, name lengths, offsets and JSON sizes.
- Claws only reads Steam and Wallpaper Engine folders. It never writes into them, never changes Wallpaper Engine settings, and never runs an application wallpaper.
- Follow DESIGN.md exactly for the tracker (see Phase 3). Use the existing `progressBar`/`setProgress`, status-line styles, `formatEta`/`formatDuration`, springs and reduced-motion rules. Add no new colours, fonts, shadows or animation curves.
- Do not touch `installers/`, the version files or `Salted-Flutters-Polaris/`. No release build unless the owner asks.
- Screenshots: headless Chrome against `npm run dev` via `scripts/dev/cdp.mjs` only. The live recording tests capture only the Wallpaper Engine window Claws opened, never the desktop.
- Commit in small phase-sized commits. Every phase ends green: `npm run build`, `npm run test:logic`, `npm run test:queue`, `npm run test:motion`, `npm run test:spotify`, and in `src-tauri`, `cargo test` and `cargo clippy --all-targets -- -D warnings`.

## Phase 0: settle the open questions (short spikes, record results in the study)

- [x] **Hidden-window capture (done 2026-11-26).** The same scene recorded at full motion (92–100% of frames changing, same brightness) in every case:
  - visible
  - pushed behind every window (`HWND_BOTTOM`)
  - moved off-screen (x = 5000)
  - made transparent and click-through (layered alpha 1, `WS_EX_TRANSPARENT`)
  - with the user's window focused again
  - with another app maximized in front. The owner's Wallpaper Engine is set to pause when an app is maximized or fullscreen, and that did not pause the pop-out window.

  The opened window takes keyboard focus for a moment; handing it back with `SetForegroundWindow` to the previous foreground window took about 0.3 s, and recording continued. **Decision:** borderless, off-screen, `WS_EX_TOOLWINDOW` (no taskbar button), bottom of the Z-order, `SWP_NOACTIVATE`, and focus handed back at once. The user keeps working and never sees the window.
- [ ] **Still to check:**
  - recording while a fullscreen (exclusive) game runs
  - recording while the screen is locked or the PC goes to sleep (Wallpaper Engine `playbacksleep` is `stop`)
  - whether a scene with sound plays it aloud during recording, and whether `-control mute` (or Wallpaper Engine's own per-window audio) can silence only that window
  - GPU contention from a heavy app: count duplicated frames (gfxcapture `dup_frames`) and warn when more than 5% of frames are repeats.
- [ ] **4K.** Ask the owner to enable NVIDIA DSR (Display Scaling ×4.00 → 3840×2160) or use a 4K display. Verify that a borderless 3840×2160 client is reached and captured exactly at 30 and 60 fps with `h264_nvenc`, `h264_mf` and `libx264`, with no dropped or duplicated frames (`nb_frames`, `freezedetect`). If the owner can't do this, implement the detection below and mark live 4K as unverified.
- [ ] **Encoder pick.** Settle the probe order (`h264_nvenc` → `h264_amf` → `h264_qsv` → `h264_mf` → `libx264`), the quality settings per encoder (target about 20 CRF-equivalent, `yuv420p`, H.264 level fitting the size and fps), and which sizes and frame rates `libx264` may use (proposed: up to 1440p at 30 fps).

## Phase 1: remove SteamCMD

The Steam library is the only source. Remove the code. Don't hide it.

- [ ] **Rust, `workshop.rs`:** remove `check_account`, `validate_steamcmd`, `valve_publisher`, `signed_by_valve`, `signer_organization` (keep a generic Authenticode helper; Phase 4 needs it for Wallpaper Engine), `choose_steamcmd`, `sign_in_steamcmd`, `steamcmd_path`, account detection (`steamcmd_accounts`, `steam_client_accounts`, `steam_auto_login_user`, `merge_accounts`, `detect_steam_accounts`), `run_steamcmd`, `read_output`, `progress_of`, `update_percent`, `classify`, `redact*`, `checked_project`, `matches_reported_path`, the `source`/`account` parameters of `download_wallpaper`, the `remove_steamcmd_copy` path, `Stage::{Updating, SigningIn, Downloading}`, and their tests and live tests (`CLAWS_TEST_STEAMCMD`). Keep `parse_vdf` only if Phase 4's Wallpaper Engine lookup uses it. Keep `parse_library_paths`, `library_folders`, `library_project`, `workshop_folder` and the shutdown hook, which will stop FFmpeg and close the recording window.
- [ ] **Rust, elsewhere:** in `tools.rs` remove `managed_steamcmd`, `find_steamcmd`, `steamcmd_info`, `stage_steamcmd`, `install_steamcmd`, `first_run_steamcmd`, `ToolsStatus.steamcmd` and their tests. Remove `commands::install_steamcmd` and the `lib.rs` registrations. Remove the `settings.rs` fields and repair branches, keeping the old-file test from the ground rules. Update `downloader.rs`'s shutdown chain.
- [ ] **Frontend:** remove SteamCMD from `api.ts`, `store.ts` (tools, busy, progress, error), `setup.ts` (`ManagedTool`), `views/shell.ts`, `views/settings.ts` (the Wallpaper Engine rows), `views/workshop.ts` (Source control, SteamCMD, account, sign-in and cleanup rows), `workshop-session.ts` (`WorkshopSource`, `missingForStart`), `views/download.ts`, `mock.ts`, and `scripts/tests/workshop-session.test.mjs`.
- [ ] **Leftover files:** an earlier Claws may have installed SteamCMD in its tools folder, and SteamCMD's copies of wallpapers can be gigabytes there. Never delete automatically, and never touch a path the user chose. When the managed folder exists, Settings → Wallpaper Engine shows one row: "SteamCMD files from an earlier version · 2.1 GB" with **Remove…**, which opens the standard alert sheet (danger button) and then deletes only that folder after the usual link/junction checks. The row disappears once the folder is gone.
- [ ] **No Steam, or no Wallpaper Engine:** the wallpaper card says so in one line with **View in Steam**. Explain that Steam with Wallpaper Engine is needed and nothing else is.
- [ ] **Docs:** README Wallpaper Engine section and privacy paragraph (Claws no longer receives a Steam account name), `CURRENT_DEVELOPMENT_STATUS.md`, `HANDOFF.md`, and the release notes draft.
- [ ] **Acceptance:** `rg -i steamcmd src src-tauri/src scripts` matches only the leftover-folder row and its test. A 1.0.9e settings file loads. An item already in the library still saves as ZIP.

## Phase 2: library states and video copies

- [ ] **Item state:** read `steamapps\workshop\appworkshop_431960.acf` (`WorkshopItemsInstalled`, `NeedsUpdate`, `NeedsDownload`) and `workshop\downloads\431960\<id>`. Report `installed`, `downloading` (with bytes on disk against Valve's `file_size`, so the existing ten-minute watch shows a real percentage) or `updatePending` (Valve's `time_updated` is newer than the ACF's `timeupdated`). Keep the read-only rule. Use fixture tests built from the real formats.
- [ ] **Copy the original:** when `project.json` declares a video that ffprobe reports as H.264 in an MP4/M4V container, **MP4 (original)** copies the file with byte progress. Write to a partial name, check the size, publish without overwriting and clean up on cancel; publishing is the same as today. HEVC, VP9, 10-bit and other containers keep the current conversion and are labelled **MP4 (converted)**. Fixtures must cover `[ ]`, CJK, a space before the extension, silent video and a 10-bit file. Bundle ffprobe from the same yt-dlp FFmpeg build if it isn't already, or read the codec with `ffmpeg -i`.
- [ ] **GIF limits:** cut to the first 30 s by default (60 s maximum, user's choice) and cap at 720p, keeping aspect ratio and never upscaling. Show the estimate before starting, and ask for confirmation with the alert sheet when it is over 10 minutes.
- [ ] **4K video wallpapers:** copy keeps any resolution. Conversion keeps source resolution (today's behaviour; keep the test) and uses the Phase 0 hardware encoder when the source is over 1440p, so a 4K conversion runs faster than real time.

## Phase 3: the export tracker (backend contract and UI)

**Backend.** Replace the `workshop-progress` payload with:

```ts
{ url: string; stage: Stage; done: number | null; total: number | null;
  unit: "seconds" | "bytes" | null; etaMs: number | null; approximate: boolean; elapsedMs: number }
```

- `Stage`: `checking | opening | loading | recording | finishing | copying | converting | zipping`.
- `WallpaperOutcome::Saved` adds `elapsedMs` and `stages: { name, ms }[]`.
- Emit at most 4 events per second, plus every stage change.
- A number is reported only when it can be backed, per DESIGN.md's honest-progress rule:

| Stage | `done` / `total` | Time left |
|---|---|---|
| checking, opening, loading, finishing | null (indeterminate) | none |
| recording | seconds since FFmpeg's first captured frame, against the chosen length, capped 0.2 s below the length until FFmpeg exits. Don't use `out_time_us`: it trails by about 1.5 s because of x264 look-ahead. | `total − done`, exact |
| copying, zipping | bytes written / bytes to write | from the rate over the last 3 s, after at least 1 s; `approximate` |
| converting | FFmpeg `-progress` `out_time_us` / source duration (from ffprobe) | `(total − done) / speed`, after at least 3 s of data; `approximate` |

- Add a backend `estimate_wallpaper_export(url, format, options)` that returns `{ seconds, approximate, bytes }` for the format row:
  - **Recording:** length + pre-roll + 1 s, exact. Bytes from a bitrate table per size and fps, approximate.
  - **Copy:** size ÷ 200 MB/s, approximate.
  - **Conversion:** duration ÷ the speed last measured for that format and size bucket. Store it in a new optional settings field `workshopSpeeds` (map of `"mp4-2160"` and the like → speed); null until the first conversion.
- **Free space:** check the estimated output plus 20% before starting. Record and conversion stop with a clear message if the drive drops under the reserve the existing disk-budget code uses.

**Frontend.** In `src/views/workshop.ts`, inside the existing running block (no new surface):

- The 4px bar via `progressBar`/`setProgress`, gliding on `--spring-smooth`:
  - Indeterminate sheen for null stages.
  - Leaving the sheen snaps, as DESIGN.md requires.
  - Recording, copying, zipping and converting are determinate.
  - The bar never passes 100% and never slides back.
  - It reaches 100% and turns green only on `Saved`. While finishing it holds at its last value.
  - Red on failure, standard colour on cancel.
  - Reduce motion makes it jump. A hidden window snaps.
- One status line, 12px secondary ink, `font-variant-numeric: tabular-nums`, parts joined by `"  ·  "` as in Queue rows:
  - `Checking the wallpaper…` / `Opening in Wallpaper Engine…` / `Loading the scene…`
  - `Recording 0:42 of 1:00  ·  156 MB  ·  18s left`
  - `Finishing the file…`
  - `Copying 2.6 of 5.6 GB  ·  about 4s left`
  - `Converting 38%  ·  about 3:10 left`
  - `Saving the ZIP 64%  ·  about 12s left`
  - Done: green check, `Saved to Videos  ·  took 1:03`. Use the Queue completion movement (3px rise from 45% opacity in 220 ms, only on the change to saved) and keep the **Show in folder** / drag actions.
- Under the format choice, before starting, the estimate in caption style: `About 1:04  ·  about 230 MB`, or `Estimate after a few seconds` when there's no measurement yet.
- **Cancel** stays where it is. Cancelling closes the Wallpaper Engine window, stops FFmpeg and deletes the partial file; the line reads `Cancelled`.
- **Elsewhere:** feed the running export into the taskbar progress (`queue-summary.ts` `taskbarState`) and send the existing finish notification (`desktop.rs` `notify_finished` rules) when an export over 30 s finishes while Claws is hidden.
- **Tests:**
  - Pure helpers in `workshop-session.ts`: stage text, fraction, ETA wording, monotonic clamp, approximate prefix. Add them to `test:logic`.
  - Rust: parsers for recorded FFmpeg `-progress` transcripts, including `N/A` values before the first frame.
  - `test:motion`: the bar's snap and glide on stage changes, and that it never goes backwards.
  - Mock: a dev-mock scenario that steps through each stage with synthetic, labelled numbers, for screenshots in both themes, at Transparency 0 and 100, and with Reduce motion.

## Phase 4: scene and web recording (new `src-tauri/src/workshop_record.rs`)

- [ ] **Find Wallpaper Engine:** use the library listing app `431960` in `libraryfolders.vdf`, then `steamapps\appmanifest_431960.acf` `installdir`, then `common\<installdir>\wallpaper64.exe`. It must be a regular file (not a link) with a valid Authenticode signature whose subject organization is exactly `Skutta Software GmbH`; test this against lookalikes. Missing or invalid means recording is not offered, and the card says why in one line.
- [ ] **Read the scene (bounded):**
  - Parse the `scene.pkg` header: version string ≤ 32 bytes, entry count ≤ 100,000, names ≤ 1 KB, every offset and length inside the file.
  - Read `scene.json` (≤ 4 MB; JSON may contain keys that differ only in case).
  - Read only the header bytes of each `.tex` entry to detect sprite-sheet frames (`TEXS`).
  - Report:
    - `moves`: effects, particles, `animationlayers`/puppet, scripts, camera shake or sprite textures.
    - `authored` size: `general.orthogonalprojection` width/height, `auto`, or perspective.
    - whether it has audio.
  - Test with synthetic packages and malformed ones (count overflow, offset past the end, truncated name).
- [ ] **Formats offered for scenes and web projects:**
  - **MP4 (recorded)**, default for moving scenes.
  - **Picture (PNG)**, default for still scenes. One frame after the pre-roll. The card says `This wallpaper doesn't move`.
  - **ZIP**.
  - Application wallpapers stay ZIP only, and presets stay as they are today.
- [ ] **Options** (inset-grouped rows, existing segmented/stepper controls):
  - **Length:** 10 s, 30 s (default), 60 s, or custom up to 10 min.
  - **Size:** in the scene's aspect ratio, heights from {720, 1080, 1440, 2160}. Each must be ≤ the authored height (perspective/auto: no cap) and ≤ the largest borderless client the display allows (`SM_CXMAXTRACK`/`SM_CYMAXTRACK`, then confirmed with `GetClientRect` after sizing).
    - Default: 1080p, or the largest available if smaller.
    - 2160 is offered when it fits. Otherwise it is shown disabled with the caption `4K needs a display set to 3840×2160 or larger, such as a 4K monitor or NVIDIA DSR / AMD VSR`.
    - Never upscale a capture to fake a size.
  - **Frame rate:** 30 (default) or 60. 60 at sizes over 1440p only with a hardware encoder.
- [ ] **Pipeline** (one export at a time; reuse `RunGuard`, cancellation and shutdown):
  1. Run `wallpaper64.exe -control openWallpaper -file <canonical project.json> -playInWindow "Claws <uuid>" -width W -height H`. If Wallpaper Engine isn't running this starts it; say so under Size before the first recording (`Wallpaper Engine will start`).
  2. Find the window with `EnumWindows`: exact title, and its process image must be the validated `wallpaper64.exe`. Time out after 15 s.
  3. Hand keyboard focus back to the window that had it before step 1 (record it first; `SetForegroundWindow`). Make Wallpaper Engine's window borderless (`SetWindowLongPtrW` `GWL_STYLE` → drop `WS_CAPTION | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX`, add `WS_POPUP`) and remove it from the taskbar (`GWL_EXSTYLE` add `WS_EX_TOOLWINDOW`, drop `WS_EX_APPWINDOW`). Then `SetWindowPos(HWND_BOTTOM, x off-screen right of the virtual screen, W, H, SWP_FRAMECHANGED | SWP_NOACTIVATE)`. Never minimise it: that stops capture. Verify the client size; on a mismatch, fail with a clear message rather than scale.
  4. Run FFmpeg (managed, under the engine lock):
     `-filter_complex gfxcapture=hwnd=<hwnd>:capture_cursor=0:max_framerate=<fps>,hwdownload,format=bgra,fps=<fps>,format=yuv420p -ss <pre-roll> -t <length> -c:v <encoder> … -movflags +faststart -progress pipe:1 -nostats <partial>`
     Pre-roll is 2 s, or 4 s when `scene.pkg` is over 500 MB. Use a bounded stderr tail for diagnostics.
  5. Close with `closeWallpaper -playInWindow "Claws <uuid>"`, confirm the window is gone, and fall back to `WM_CLOSE`. This runs from a drop guard, so it happens on success, failure, cancel and app shutdown.
  6. Check the result with ffprobe (size, frame count within 1% of length × fps). Publish like other exports: title-based name with the item ID, no overwrite, no partial left behind.
  7. Treat a frozen capture as a failure. If a scene classified as moving records with no frame change after the pre-roll, report `The recording didn't change; Wallpaper Engine may have paused. Keep its window visible and try again.` rather than saving a still MP4.
- [ ] **Out of scope, listed in the release notes:** scene sound in recordings (the audio is inside `scene.pkg`; follow-up), seamless loops, and mouse- or audio-reactive behaviour (recorded idle).
- [ ] **Tests:**
  - Unit: size-option matrix (authored size × display limit × encoder), pre-roll choice, window matching, encoder probe fallbacks, ffprobe verification.
  - Opt-in live test `CLAWS_TEST_WALLPAPER_ENGINE=<itemId>`: records 3 s of a subscribed scene and checks size, frames and motion.
  - Opt-in `CLAWS_TEST_WALLPAPER_ENGINE_4K=<itemId>`: runs only when the display allows 2160.

## Phase 5: verification and docs

- [ ] All suites green, Clippy clean. Dev-mock screenshots of every tracker state, light and dark.
- [ ] **Live checks on the owner's PC, recorded in the study with numbers:**
  - Copy of the 5.6 GB 4K video, with its progress and time-left accuracy.
  - Conversion of a non-H.264 video, with ETA accuracy at 10%, 50% and 90%.
  - 30 s and 10 min 1080p recordings.
  - A still scene to PNG.
  - A 4K recording if Phase 0 had a 2160-tall display.
  - Cancel during each stage, leaving no window, process or partial file behind.
  - Closing Claws during a recording.
- [ ] **Docs:**
  - README: sources, formats, recording, 4K requirements, time estimates.
  - DESIGN.md: add the export tracker to Progress and activity, and to the Motion table if anything new moves.
  - `CURRENT_DEVELOPMENT_STATUS.md` and `HANDOFF.md`.
  - Release notes draft, which says SteamCMD is gone and how to remove its old files.
- [ ] Independent review of the diff, then fix material findings. Building and publishing an installer happens only on the owner's instruction, with the version nickname they choose.

## Decisions taken in this plan (change before starting if the owner disagrees)

- SteamCMD is removed in this release, not hidden.
- Recording defaults: 30 s, 1080p (or the authored size if smaller), 30 fps, silent.
- 4K appears only when real (display and authored size allow it). There is no upscaled "4K".
- Recording runs in the background. The Wallpaper Engine window is borderless at the exact size, off-screen, with no taskbar button. Focus returns to the user's window at once, and the user can keep working, switch Claws pages or hide Claws. The tracker, taskbar progress and finish notification report the job.
- GIF: 30 s default, 60 s maximum, 720p cap.
- MP4 (original) copies H.264 only. Everything else converts.

# Claws feature expansion and animation implementation plan

> For implementers: use the executing-plans workflow. Execute one deliverable at a time; do not delegate unless the owner requests it. Recheck HEAD and remove tasks already completed before starting.

**Date:** 2026-11-08 (scope revised by owner)
**Baseline:** `6794e41`, version 1.0.5, plus current working changes. See `docs/CURRENT_DEVELOPMENT_STATUS.md` for the implementation audit; transcript export was already in progress when native dragging/completion feedback were added.
**Goal:** Extend the Windows downloader with useful batch, editorial and text workflows, and restrained animations, while retaining reliability and pursuing the measured speed bottleneck first.
**Architecture:** Retain Tauri, TypeScript, yt-dlp, owned workspaces, publication journals, revisioned events and the queue writer. Introduce focused parsing/processing modules rather than expanding downloader.rs with every feature. Native file dragging requires a verified Windows integration, not HTML dragging alone.
**Stack:** Rust/Tauri 2, TypeScript/CSS/Web Animations, yt-dlp and FFmpeg. Additional model/tool downloads remain optional.

Scope update: macOS support, vocal removal, ProRes presets and clip sections are excluded at the owner’s request. Claws remains Windows-only. Compatible and Original output choices, transcript export, native file dragging and animation improvements remain in scope. No product code is changed by this scope revision.

## 1. Feature audit

| Requested feature | Verified current state | Remaining scope |
| --- | --- | --- |
| Downloads anything / X / 1,200+ sites | yt-dlp engine and HTTP(S) validation already exist; coverage depends on installed engine, source and authentication | Test supported source categories; improve unsupported/authentication errors. Do not promise universal coverage or publish an unverified site count. |
| Spotify | Full music-track download is not established by this code or this review | Verify representative URL types using the supported engine. Separate music, podcasts, previews and metadata. Do not substitute another recording while presenting it as the requested Spotify source. |
| 4K | Explicit 2160p choice exists | Retain; no new 4K implementation task. |
| 6K / 8K | Implemented in `6794e41`: source-driven explicit higher-resolution choices | Preserve source-driven choices; never upscale to manufacture availability. |
| Compatible / Original output | Both choices are now implemented; audio output supports several formats | Preserve source selection and verify output compatibility; no new editing-transcode preset. |
| Bulk downloads | Playlists, selection, batch enqueue and bounded queue rendering exist | Independent-link batches are committed in `d9cc1bb`; preserve their coverage. |
| Under 100 MB | 1.0.5 installer is 2,839,767 bytes | Preserve installer budget; disclose separate yt-dlp, FFmpeg, runtime and optional model downloads. Installer size is not total installed size. |
| Very fast | Queue improvements, matching-audio option, Fast Save and resilient retries exist | Actual affected slow-source diagnosis remains incomplete. No general speed claim is justified by current fixture results. |
| Drag into an editor timeline | Native copy-only integration now implemented in the working tree | Live Explorer/editor interoperability tests remain; do not reimplement the native path. |
| Transcript export | Dedicated transcript mode is in progress in the working tree | Caption-derived text/SRT/VTT export. Speech recognition is a separate optional capability. |
| Custom animation | Springs, selection movement, transitions, row entry/exit and reduced-motion handling exist | Extend the current system; do not rebuild it or animate every progress event. |

## 2. Priority and dependencies

1. **P0 — Source integrity and measured speed diagnosis.** Preserve the committed font safeguards; preserve the already-fixed lifecycle safeguards. Diagnose the affected download path before choosing acceleration.
2. **P1 — Batch link input and honest size previews.** Strong day-to-day value with minimal new infrastructure.
3. **P1 — Explicit high-resolution and original-format choices.** Expose existing engine capability accurately.
4. **P1 — Transcript export.** Finish the partial implementation and correct text-preservation bugs before shipping.
5. **P2 — Native dragging into editors.** Begin an early feasibility check; ship only after real Windows/editor validation.
6. **P2 — Animation improvements.** Add small interaction animations alongside their owning feature; perform the broader motion pass after behavior stabilizes.
7. **P3 — Replay heatmaps and speech recognition.** Conditional features, not dependencies for the core release.

Do not delay every independent feature while waiting for a slow URL. Continue P1 work while keeping speed diagnosis the main performance priority. A critical correctness regression takes precedence over all additions.

## 3. P0: Review pending safeguards and isolate the speed problem

**Files:** pending font files, `scripts/dev/download-bench.mjs`, `scripts/dev/download-fixtures.py`, `docs/performance/speed-diagnosis.md`; existing speed plan `2026-11-04-download-reliability-and-performance.md` remains authoritative for acceleration details.

- [ ] Run font tests, queue tests and the frontend build; confirm source and built assets match the approved hashes.
- [ ] Capture an affected source, selected formats, tool versions and options; redact signed URLs and credentials from stored results.
- [ ] Compare Claws with standalone yt-dlp using identical source formats. Separate lookup, transfer, merge, encoding and publication time, including child-process CPU.
- [ ] Repeat comparable measurements with one warm-up and three timed runs; verify hashes or streams/duration.
- [ ] Implement acceleration only for the demonstrated cause, following the existing speed plan. An external connection/source cap must be reported as such.

**Exit:** a diagnosed bottleneck or an explicit missing-evidence record; no unsupported speed claim. Preserve publication, cancel/retry and Library-failure regressions.

## 4. Independent-link batches — implemented

Committed in `d9cc1bb`. Preserve existing behavior and regression coverage; no implementation task remains here.

## 5. Download size and temporary-space previews — implemented

Committed in `88f5f02`. Preserve existing behavior and regression coverage; no implementation task remains here.

## 6. Higher resolutions and original-format choices — implemented

Committed in `6794e41`. Preserve existing behavior and regression coverage; no implementation task remains here.

## 7. P1: Transcript export

**Modify:** `src/views/download.ts`, `src/api.ts`, `probe.rs`, `downloader.rs`, `download_policy.rs`, `download_files.rs`, `history.rs`, `src/mock.ts`.
**Create:** `src-tauri/src/transcript.rs` for parsing/normalization and unit tests.

- [ ] Add a distinct transcript output mode; it must not require a downloaded video or invent media-byte progress.
- [ ] Offer available caption languages, distinguishing human captions and automatic captions. No captions means an explicit unavailable state.
- [ ] Parse VTT/SRT and produce TXT with optional timestamps; preserve Unicode, line order and meaningful speaker/text content. Remove markup and repeated rolling-caption overlap without deleting legitimate repeated speech.
- [ ] Publish text through the existing owned workspace and journal, with text-specific artifact validation and Library metadata. Keep existing video/audio queue formats readable.
- [ ] Test empty captions, malformed cues, overlapping text, multilingual content, filename collisions, cancellation and Library-save failure.

**Exit:** text export works without downloading the full media; no speech-recognition or arbitrary lyrics guarantee is implied.

## 8. P2: Native outgoing file drag

**Modify:** `src/views/library.ts`, completed-row UI in `src/views/queue.ts`, `commands.rs`, `lib.rs`, Tauri capabilities and Cargo dependencies only if required.
**Create:** focused `src-tauri/src/file_drag.rs` integration module.

- [ ] Before selecting a plugin or Windows API, verify supported Tauri/Windows versions, maintenance status, license, native drag formats and threading requirements. Do not invent a Tauri start-drag API.
- [ ] Prove dragging one local completed file into Windows Explorer and at least one installed target editor. HTML `draggable` alone is not acceptance.
- [ ] Accept a Library/job identifier from the frontend, resolve its authoritative published path in Rust, and reject incomplete, removed, missing or untrusted paths. Do not accept arbitrary frontend paths as drag authority.
- [ ] Export real Windows file-drop data. Use a copy-oriented operation; never relocate or delete the Library file when the receiver completes a drag.
- [ ] Add a clear drag handle/affordance and retain Open/Reveal as keyboard-accessible alternatives. Dragging a thumbnail must not export the image URL.
- [ ] Test Unicode/spaces/long paths, missing files, network destinations, cancelled drags and actual timeline import. Record editor/version/codec compatibility separately from successful file dropping.

**Exit:** real media imports into a verified editor and remains in the Library. Multiple-file dragging is a subsequent increment after single-file reliability.

## 9. P2: Animation pass, incremental by feature

**Modify:** `src/ui/motion.ts`, `src/ui/layout.ts`, `src/views/download.ts`, `src/views/queue.ts`, `src/views/library.ts`, relevant CSS and `DESIGN.md`.

Reuse existing springs, reduced-motion handling and bounded row creation. Recreate chosen effects with original code; inspect ID4's actual interactions before claiming visual parity or reusing any licensed material.

| Order | Effect | Constraints / acceptance |
| --- | --- | --- |
| 1 | Link lookup to preview transition | Cancel superseded transitions; keep focus stable; errors appear immediately. |
| 2 | Video/Audio/Transcript selection indicator | Extend existing segmented controls; do not introduce a second selection system. |
| 3 | New queue rows and reordering | Preserve existing small-batch entry animation; do not animate hundreds of playlist rows or rebuild rows on progress. |
| 4 | Progress and completion checkmark | Smooth displayed progress only; never fabricate received bytes, delay completion, or overshoot 100%. Cancel animations on retry/removal. |
| 5 | Library drag pickup and landing feedback | Native drag success comes from the native operation, not an optimistic animation. |

- [ ] Record before/after examples and frame-cost measurements for each effect; use transform/opacity where practical, avoiding repeated layout measurement per progress event.
- [ ] Test 500 queued rows and 8 active transfers with the current queue benchmark. Reject changes that add progress-induced long tasks or sustained CPU churn.
- [ ] Test Reduce motion, keyboard focus, rapid repeated actions, view disposal and hidden windows. Keep static success/error indicators when motion is disabled.
- [ ] Do not add decorative looping hero animations to the working downloader.

**Exit:** improved transitions preserve accuracy, accessibility and responsiveness. Commit each effect with its owning feature rather than making motion a prerequisite for that feature.

## 10. P3 / separate decisions

- **Most replayed:** inspect whether selected sources expose usable heatmap data; show the shortcut only when present. Treat this as source-specific replay navigation or preview metadata; it must not reintroduce clip-export controls.
- **Speech recognition:** optional local model and language selection, separate download size, timestamp output and cancellation; do not bundle a large model just to claim transcript support.
- **Lyrics:** export only text available from a legitimate supported source; distinguish lyrics from speech transcripts and generated recognition.

## 11. Release sequence and verification

Release independently reviewable increments: (A) batch/estimates/high-resolution UI, (B) transcript export, (C) native file dragging, with animation improvements attached to each. Select actual version numbers only after checking current tags.

For every increment:
- [ ] Write a failing regression for the changed behavior, implement the smallest change, verify it passes and retain existing lifecycle tests.
- [ ] Run `npm ci`, `npm run test:fonts`, `npm run test:queue`, `python scripts/test_font_metrics.py`, and `npm run build`.
- [ ] Build the fixture tool, then run `cargo test --locked --manifest-path src-tauri/Cargo.toml` and `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets --features test-tools -- -D warnings`.
- [ ] Exercise affected native flows using disposable data, including cancellation, restart, blocked persistence and output conflicts. Do not use the owner's real queue as a fixture.
- [ ] Measure installer bytes, first-run tool bytes, total installed footprint, startup and relevant job-stage timing separately. Keep the base installer below 100 MB; keep optional models/tools outside it.
- [ ] Validate new media paths using ffprobe and actual editor import where promised. Verify preserved streams instead of assuming a successful FFmpeg exit proves parity.
- [ ] Update README, HANDOFF, feature status and the remaining-work plan; remove completed tasks rather than accumulating obsolete implementation instructions.
- [ ] Synchronize all five version files, build a new installer after the final source change, test upgrade, record SHA-256 and publish only with authorization. Verify uploaded artifacts against the exact release commit.

## Sources and limits

- Feature inspiration: https://polyviral.org/id4/ (page advertising reviewed; the ID4 app and animation implementation were not tested).
- yt-dlp behavior: https://github.com/yt-dlp/yt-dlp#download-options and its format/subtitle/postprocessing documentation. Recheck against Claws' supported installed version when implementing.
- Existing performance evidence and deferred acceleration: `docs/performance/speed-diagnosis.md` and `docs/superpowers/plans/2026-11-04-download-reliability-and-performance.md`.

A successful download from one site does not establish all-site support. A small installer does not establish a small tool/model footprint. Smooth animation does not establish higher network throughput.

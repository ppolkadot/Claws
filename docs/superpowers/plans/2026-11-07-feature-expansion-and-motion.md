# Claws feature expansion and animation implementation plan

> For implementers: use the executing-plans workflow. Execute one deliverable at a time; do not delegate unless the owner requests it. Recheck HEAD and remove tasks already completed before starting.

**Date:** 2026-11-07
**Baseline:** `d7b7263`, version 1.0.5, plus the uncommitted font safeguards.
**Goal:** Extend the Windows downloader with useful batch, editorial and text workflows, and restrained animations, while retaining reliability and pursuing the measured speed bottleneck first.
**Architecture:** Retain Tauri, TypeScript, yt-dlp, owned workspaces, publication journals, revisioned events and the queue writer. Introduce focused parsing/processing modules rather than expanding downloader.rs with every feature. Native file dragging requires a verified Windows integration, not HTML dragging alone.
**Stack:** Rust/Tauri 2, TypeScript/CSS/Web Animations, yt-dlp and FFmpeg. Additional model/tool downloads remain optional.

This is a proposed implementation design, not authorization to implement a macOS port or optional large processing tools. No code changes are required merely to review this document.

## 1. Feature audit

| Requested feature | Verified current state | Remaining scope |
| --- | --- | --- |
| Downloads anything / X / 1,200+ sites | yt-dlp engine and HTTP(S) validation already exist; coverage depends on installed engine, source and authentication | Test supported source categories; improve unsupported/authentication errors. Do not promise universal coverage or publish an unverified site count. |
| Spotify | Full music-track download is not established by this code or this review | Verify representative URL types using the supported engine. Separate music, podcasts, previews and metadata. Do not substitute another recording while presenting it as the requested Spotify source. |
| 4K | Explicit 2160p choice exists | Retain; no new 4K implementation task. |
| 6K / 8K | Best selection can request the highest source resolution, but explicit choices stop at 2160p | Source-driven higher-resolution choices and format diagnostics; never upscale to manufacture availability. |
| ProRes and other codecs | Video selection prefers H.264/AAC and merges to MP4; audio output supports several formats | Distinguish original codec preservation, compatible remuxing and explicit transcoding; add an optional editing preset. |
| Mac native / yt-dlp | yt-dlp already used; shipped application is Windows-only Tauri | Retain Windows-native integration. macOS requires a separate product/platform scope and real Mac testing; Tauri does not imply a Swift-native UI. |
| Clip sections | Missing | Validated ranges, approximate/precise cut policy, preview controls and lifecycle coverage. |
| Bulk downloads | Playlists, selection, batch enqueue and bounded queue rendering exist | Multiline independent URL input, bounded concurrent lookup, partial failure handling and batch summaries. |
| Under 100 MB | 1.0.5 installer is 2,839,767 bytes | Preserve installer budget; disclose separate yt-dlp, FFmpeg, runtime and optional model downloads. Installer size is not total installed size. |
| Very fast | Queue improvements, matching-audio option, Fast Save and resilient retries exist | Actual affected slow-source diagnosis remains incomplete. No general speed claim is justified by current fixture results. |
| Drag into an editor timeline | No native outgoing-file drag integration found; thumbnail images explicitly disable HTML dragging | Windows file-drop integration for verified completed media, plus actual editor interoperability tests. |
| Transcript export | Subtitles exist; dedicated transcript mode does not | Caption-derived text/SRT/VTT export. Speech recognition is a separate optional capability. |
| Custom animation | Springs, selection movement, transitions, row entry/exit and reduced-motion handling exist | Extend the current system; do not rebuild it or animate every progress event. |

## 2. Priority and dependencies

1. **P0 — Source integrity and measured speed diagnosis.** Finish reviewing the pending font changes; preserve the already-fixed lifecycle safeguards. Diagnose the affected download path before choosing acceleration.
2. **P1 — Batch link input and honest size previews.** Strong day-to-day value with minimal new infrastructure.
3. **P1 — Explicit high-resolution and original-format choices.** Expose existing engine capability accurately.
4. **P1 — Transcript export and time-range clipping.** Independent increments; implement text export first, approximate clipping next, precise clipping after processing controls.
5. **P2 — Native dragging into editors.** Begin an early feasibility check; ship only after real Windows/editor validation.
6. **P2 — Editing codecs and ProRes.** Requires bounded processing, robust cancellation and larger disk estimates.
7. **P2 — Animation improvements.** Add small interaction animations alongside their owning feature; perform the broader motion pass after behavior stabilizes.
8. **P3 — Replay heatmaps, vocal removal and speech recognition.** Conditional features, not dependencies for the core release.
9. **Separate scope — macOS.** No porting work in the Windows feature release.

Do not delay every independent feature while waiting for a slow URL. Continue P1 work while keeping speed diagnosis the main performance priority. A critical correctness regression takes precedence over all additions.

## 3. P0: Review pending safeguards and isolate the speed problem

**Files:** pending font files, `scripts/dev/download-bench.mjs`, `scripts/dev/download-fixtures.py`, `docs/performance/speed-diagnosis.md`; existing speed plan `2026-11-04-download-reliability-and-performance.md` remains authoritative for acceleration details.

- [ ] Review the moved WOFF2 assets, integrity manifest, Python generator, private CSS alias, Settings detection and build checks. Do not reimplement them.
- [ ] Run font tests, queue tests and the frontend build; confirm source and built assets match the approved hashes.
- [ ] Capture an affected source, selected formats, tool versions and options; redact signed URLs and credentials from stored results.
- [ ] Compare Claws with standalone yt-dlp using identical source formats. Separate lookup, transfer, merge, encoding and publication time, including child-process CPU.
- [ ] Repeat comparable measurements with one warm-up and three timed runs; verify hashes or streams/duration.
- [ ] Implement acceleration only for the demonstrated cause, following the existing speed plan. An external connection/source cap must be reported as such.

**Exit:** a diagnosed bottleneck or an explicit missing-evidence record; no unsupported speed claim. Preserve publication, cancel/retry and Library-failure regressions.

## 4. P1: Independent-link batches

**Modify:** `src/views/download.ts`, `src/store.ts`, `src/api.ts`, `src/mock.ts`.
**Create:** `src/download-input.ts`, `scripts/tests/download-input.test.mjs`.
**Reuse:** `commands.rs` enqueue validation and downloader batch events; do not create a second queue.

- [ ] Write and run failing parser tests for LF/CRLF input, blank lines, duplicates, malformed schemes and one invalid link among valid links.
- [ ] Parse one HTTP(S) URL per nonempty line. Preserve URL query parameters; deduplicate exact normalized URLs only, without assuming distinct signed links are identical.
- [ ] Limit one submitted batch to 500 links and look up at most three concurrently. Keep a per-row status and cancellation token; ignore responses from superseded submissions.
- [ ] Show selection, title and lookup failure per item. A failed lookup must not discard successful rows. Never automatically start every pasted link.
- [ ] Feed selected rows into existing batch enqueue; retain backend validation, duplicate detection and revision handling.
- [ ] Test 500 links, duplicate replies, lookup cancellation, partial failure and repeated clicks. Measure layout work and responsiveness against the existing queue benchmark.

**Exit:** a mixed batch enqueues valid selected entries exactly once; failed entries can be retried independently. Commit boundary: `feat: add independent URL batches`.

## 5. P1: Download size and temporary-space previews

**Modify:** `probe.rs`, `disk_budget.rs`, `src/api.ts`, `src/views/download.ts`, `src/mock.ts`.
**Create:** `src/download-estimates.ts` and corresponding Node tests.

- [ ] Add optional selected-stream size, estimate confidence and codec/container metadata to probe results. Old cached results without those fields must still load.
- [ ] Derive estimates from the same selection policy used to download, not the largest unrelated format in the response.
- [ ] Display download bytes separately from estimated peak temporary disk demand. Reuse the budget formulas and avoid counting current reservations twice.
- [ ] Aggregate known sizes and unknown-item counts: for example, “About 2.1 GB; 3 items have unknown sizes.” Missing values are never zero.
- [ ] Bound playlist probing and offer an explicit estimate action for large lists; do not silently fetch every item on initial lookup.
- [ ] Test changing quality, audio selection, mixed known/unknown sizes, clipping and ProRes estimates. Admission still rechecks actual free space when starting.

**Exit:** estimates are honest and correspond to selected output; previews never replace backend space checks.

## 6. P1: Higher resolutions and original-format choices

**Modify:** `probe.rs`, `download_policy.rs`, `downloader.rs` option validation, `settings.rs` if defaults change, `src/api.ts`, `src/views/download.ts`, `src/mock.ts`.

- [ ] Add fixtures with 2160p, 2880p, 3240p and 4320p sources, portrait video and missing codec fields. Confirm current explicit choices fail the higher-resolution tests.
- [ ] Build choices from available source heights instead of the fixed 2160p ceiling. Display exact resolution; use 6K/8K labels only when the dimensions justify them.
- [ ] Separate source selection from output processing. Proposed persisted option: `videoPreset: "compatible" | "original" | "editingProRes"`; missing values migrate to current compatible behavior.
- [ ] Original preserves source streams where possible. Compatible retains current behavior initially; describe preferences accurately rather than implying every resulting MP4 is H.264.
- [ ] Reuse frozen execution settings so a paused job cannot change codec policy halfway through a retry. Defer exposing editingProRes until Task 10 is complete.
- [ ] Validate stream codecs, dimensions, HDR/color metadata and audio with ffprobe fixtures. Never promise that remuxing repairs an editor-incompatible codec.

**Exit:** explicit 6K/8K choices appear only when offered by the source; default behavior and existing saved jobs remain compatible.

## 7. P1: Transcript export

**Modify:** `src/views/download.ts`, `src/api.ts`, `probe.rs`, `downloader.rs`, `download_policy.rs`, `download_files.rs`, `history.rs`, `src/mock.ts`.
**Create:** `src-tauri/src/transcript.rs` for parsing/normalization and unit tests.

- [ ] Add a distinct transcript output mode; it must not require a downloaded video or invent media-byte progress.
- [ ] Offer available caption languages, distinguishing human captions and automatic captions. No captions means an explicit unavailable state.
- [ ] Parse VTT/SRT and produce TXT with optional timestamps; preserve Unicode, line order and meaningful speaker/text content. Remove markup and repeated rolling-caption overlap without deleting legitimate repeated speech.
- [ ] Publish text through the existing owned workspace and journal, with text-specific artifact validation and Library metadata. Keep existing video/audio queue formats readable.
- [ ] Test empty captions, malformed cues, overlapping text, multilingual content, filename collisions, cancellation and Library-save failure.

**Exit:** text export works without downloading the full media; no speech-recognition or arbitrary lyrics guarantee is implied.

## 8. P1: Clip ranges

**Modify:** options/execution in `downloader.rs` and `download_policy.rs`, `disk_budget.rs`, `src/api.ts`, `src/views/download.ts`, `src/mock.ts`.
**Create:** `src/clip-range.ts` and tests; processing helper module when precise mode is introduced.

Proposed optional persisted value:
```ts
interface ClipRange { startSeconds: number; endSeconds: number; precise: boolean }
```

- [ ] First test finite values, negative start, zero/reversed duration, missing source duration and end beyond known duration. Validate again in Rust before launching a process.
- [ ] Start with timestamp inputs and a duration summary. Preserve fractional seconds; distinguish an entire-video job from a clipped job when checking duplicates.
- [ ] Verify yt-dlp section options against the installed supported version and FFmpeg; freeze the range with the attempt. A source lacking efficient seeking may still transfer substantial data.
- [ ] Approximate mode may align to keyframes; disclose that limitation. Precise mode must explicitly use the controlled processing path and disclose encoding cost.
- [ ] Add preview thumbnails and keyboard-accessible range handles only after the timestamp path works. Use the same range model for handles and inputs.
- [ ] Test cancellation during seek/transfer/encoding, restart, multiple ranges from one source, source-duration change and conflicting output names. Include range in the output name or otherwise avoid collisions.
- [ ] Compare result duration, first/last frames, audio sync and subtitles. Decide whether clipped subtitles are offset or excluded; never publish incorrectly timed captions silently.

**Exit:** valid clips are reproducible after resume; approximate and precise output have separately measured tolerances recorded in tests.

## 9. P2: Native outgoing file drag

**Modify:** `src/views/library.ts`, completed-row UI in `src/views/queue.ts`, `commands.rs`, `lib.rs`, Tauri capabilities and Cargo dependencies only if required.
**Create:** focused `src-tauri/src/file_drag.rs` integration module.

- [ ] Before selecting a plugin or Windows API, verify supported Tauri/Windows versions, maintenance status, license, native drag formats and threading requirements. Do not invent a Tauri start-drag API.
- [ ] Prove dragging one local completed file into Windows Explorer and at least one installed target editor. HTML `draggable` alone is not acceptance.
- [ ] Accept a Library/job identifier from the frontend, resolve its authoritative published path in Rust, and reject incomplete, removed, missing or untrusted paths. Do not accept arbitrary frontend paths as drag authority.
- [ ] Export real Windows file-drop data. Use a copy-oriented operation; never relocate or delete the Library file when the receiver completes a drag.
- [ ] Add a clear drag handle/affordance and retain Open/Reveal as keyboard-accessible alternatives. Dragging a thumbnail must not export the image URL.
- [ ] Test Unicode/spaces/long paths, missing files, network destinations, cancelled drags and actual timeline import. Record editor/version/codec compatibility separately from successful file dropping.

**Exit:** real media imports into a verified editor and remains in the Library. Multiple-file dragging is a subsequent increment after single-file reliability.

## 10. P2: Editing presets and ProRes

**Modify:** `tools.rs`, `disk_budget.rs`, job options, `history.rs`, download UI and queue stages.
**Create:** `src-tauri/src/processing.rs` for managed FFmpeg stages and tests.

- [ ] Introduce independently bounded processing capacity: one encoding job initially, while respecting existing download limits. Persist processing inputs/outputs and restart state.
- [ ] Check encoder availability in the installed FFmpeg build before enabling a preset. Offer explicit ProRes profile/container and audio settings; never present transcoding as restored source quality.
- [ ] Keep original source artifacts until processed output passes validation and publication/Library persistence completes. Preserve original media on encoding failure.
- [ ] Recheck disk space before launching processing; model large intermediate/output demand conservatively and show unknown estimates honestly.
- [ ] Stop the FFmpeg process tree on cancel, retain retryable owned files, and publish only validated output. Wire encoding progress separately from transfer progress.
- [ ] Validate duration, dimensions, color/HDR handling, pixel format, audio and actual editor import. Measure CPU, output size and total time.

**Exit:** ProRes is explicitly optional, recoverable and editor-tested; FFmpeg downloads remain separate from the small app installer.

## 11. P2: Animation pass, incremental by feature

**Modify:** `src/ui/motion.ts`, `src/ui/layout.ts`, `src/views/download.ts`, `src/views/queue.ts`, `src/views/library.ts`, relevant CSS and `DESIGN.md`.

Reuse existing springs, reduced-motion handling and bounded row creation. Recreate chosen effects with original code; inspect ID4's actual interactions before claiming visual parity or reusing any licensed material.

| Order | Effect | Constraints / acceptance |
| --- | --- | --- |
| 1 | Link lookup to preview transition | Cancel superseded transitions; keep focus stable; errors appear immediately. |
| 2 | Video/Audio/Transcript selection indicator | Extend existing segmented controls; do not introduce a second selection system. |
| 3 | New queue rows and reordering | Preserve existing small-batch entry animation; do not animate hundreds of playlist rows or rebuild rows on progress. |
| 4 | Progress and completion checkmark | Smooth displayed progress only; never fabricate received bytes, delay completion, or overshoot 100%. Cancel animations on retry/removal. |
| 5 | Clip handle and timestamp feedback | Pointer and keyboard paths share values; no motion may hide the selected cut boundaries. |
| 6 | Library drag pickup and landing feedback | Native drag success comes from the native operation, not an optimistic animation. |

- [ ] Record before/after examples and frame-cost measurements for each effect; use transform/opacity where practical, avoiding repeated layout measurement per progress event.
- [ ] Test 500 queued rows and 8 active transfers with the current queue benchmark. Reject changes that add progress-induced long tasks or sustained CPU churn.
- [ ] Test Reduce motion, keyboard focus, rapid repeated actions, view disposal and hidden windows. Keep static success/error indicators when motion is disabled.
- [ ] Do not add decorative looping hero animations to the working downloader.

**Exit:** improved transitions preserve accuracy, accessibility and responsiveness. Commit each effect with its owning feature rather than making motion a prerequisite for that feature.

## 12. P3 / separate decisions

- **Most replayed:** inspect whether selected sources expose usable heatmap data; show the shortcut only when present. Numeric clipping remains fully functional without it.
- **Vocal removal:** optional local separation engine, verified model distribution/license, download consent, bounded processing and disk use; benchmark on CPU-only machines before offering it.
- **Speech recognition:** optional local model and language selection, separate download size, timestamp output and cancellation; do not bundle a large model just to claim transcript support.
- **Lyrics:** export only text available from a legitimate supported source; distinguish lyrics from speech transcripts and generated recognition.
- **macOS:** separate plan for platform-specific glass/tray/shortcuts, paths, tool distribution, native dragging, signing/notarization, packaging and real Apple hardware validation. No macOS-native claim for the Windows release.

## 13. Release sequence and verification

Release independently reviewable increments: (A) batch/estimates/high-resolution UI, (B) transcript and clipping, (C) native editor workflow and processing, with animation improvements attached to each. Select actual version numbers only after checking current tags.

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

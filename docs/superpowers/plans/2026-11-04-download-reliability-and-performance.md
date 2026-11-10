# Remaining Download Fixes and Speed Improvements — Implementation Plan

> Historical baseline and implementation plan. Reliability fixes were delivered; source versions and test totals below are historical. Remaining real-source speed/acceptance work is tracked in [current status](../../CURRENT_DEVELOPMENT_STATUS.md) and [speed diagnosis](../../performance/speed-diagnosis.md).

> **For agentic workers:** Use superpowers:executing-plans to execute this plan task-by-task. Do not dispatch agents unless the owner requests delegation. Before starting, compare HEAD with the baseline below and remove any task subsequently completed.

**Goal:** Demonstrate a meaningful improvement to the actual slow-download scenario and package the verified result while preserving the completed reliability corrections.

**Architecture:** Extend the current job workspaces, execution snapshots, queue writer, revisioned events and progress model only where the measured transfer/processing bottleneck requires it. Keep the existing yt-dlp path available for unsupported cases.

**Tech stack:** Existing Rust/Tauri/Tokio and TypeScript application, yt-dlp and FFmpeg; optional aria2c only when measurements support it.

---

## 1. Current baseline and scope

- Branch: `download-reliability`; baseline: `624dd7e` plus the current follow-up fixes.
- Source version: 1.0.5. The reliability release includes a newly built installer; its release record is in `docs/HANDOFF.md`. Future speed changes need another version and artifact.
- Latest validation includes 134 Rust tests, 11 queue tests, two font tests, the frontend build and feature-enabled Clippy. Dedicated regressions cover publication interruptions and follow-up recovery cases; those fixes are complete.
- The recorded plain-HTTP policy comparison was effectively unchanged: 4.005346 seconds before versus 4.006694 seconds now for a 64 MiB payload under the same per-connection rate limit. This was not an Internet or full native-app benchmark.
- See [current problems and speed review](../../CURRENT_PROBLEMS_AND_SPEED_REVIEW.md) and [raw HTTP measurements](../../performance/download-speed-review-2026-11-04.json).

This plan contains **remaining work only**. Existing functionality is a dependency, not a task to implement again. Historical findings and completed changes belong in the review summary and handoff.

## 2. Priorities and execution order

**Main improvement priority: resolve low download speed on the affected source.** The lifecycle fixes and the follow-up recovery corrections have been implemented; the processing-space monitor remains best effort until a managed pre-launch processing gate exists.

1. Finish Task 1's real-source diagnosis; controlled fixture measurements are already recorded.
2. Deliver Task 2's measured speed fix. Use Task 3 for a demonstrated processing/scheduling bottleneck or Task 4 for a demonstrated per-connection HTTP limit.
3. Validate and package through Task 5. Do not repeat lifecycle, queue-batching, native-audio or retry-policy implementations already in this branch.

A source-wide cap or the connection's maximum bandwidth may remain outside the application's control. A faster Queue screen is not proof of higher network throughput.

## 3. Task 1 — Diagnose the actual low-speed scenario

**Priority:** primary; begin now. **Files:** extend `scripts/dev/download-bench.mjs` and `scripts/dev/download-fixtures.py` only where needed; add results to `docs/performance/`. Use existing frontend/native development helpers.

- [ ] Identify a representative slow URL and intended quality/output. Use owner-provided or suitable lawful test content. If no affected URL is available, continue controlled fixtures and label real-source diagnosis pending.
- [ ] Record the actual executable under test. A run of the older repository installer is not a test of this branch. Record source commit, tool versions, selected format IDs, codecs, sizes, protocol, settings and environment.
- [ ] Compare standalone yt-dlp with Claws using identical source formats, authentication and output options. Redact cookies, signed media URLs and authorization headers from reports.
- [ ] Measure extraction, transfer, conversion/merge and publication separately. Include total completion time. For application-level measurements, include FFmpeg/yt-dlp child processes rather than reporting only the launcher CPU time.
- [ ] Use one warm-up and at least three timed runs per comparable configuration. Alternate the ordering, record spread and received bytes, and avoid comparing different resolutions/codecs as though only the engine changed.
- [ ] Reuse the existing controlled HTTP result; do not repeat it as if no baseline existed. Extend to a sufficiently long transfer and a relevant large-media case to expose sustained limits and processing costs.
- [ ] Compare fragment counts 1, 4 and 8 for HLS/DASH, and job counts 1 and 3. Use eight jobs as a stress case. Plain HTTP comparisons must change the transfer strategy, not only `--concurrent-fragments`.
- [ ] Validate output hashes for synthetic files and ffprobe streams/duration for media. Retain raw results even when a configuration provides no improvement.

**Decision output:** classify the main delay as extraction, transfer, processing, scheduling, or an external limit. Select the next implementation based on this evidence. Do not require every optional acceleration feature to be built.

**Acceptance:** a reproducible baseline identifies the affected stage, or explicitly records which real-source evidence is missing. No general speed claim based solely on localhost measurements.

**Commit boundary:** `test: measure the remaining slow-download bottleneck`.

## 4. Task 2 — Deliver the measured speed fix

**Priority:** main improvement after the necessary correctness gates. **Dependency:** Task 1 measurements and passing lifecycle regressions before release. **Files:** only those needed by the confirmed bottleneck; preserve current policy, source selections and output correctness.

Use this decision table rather than implementing every candidate unconditionally:

| Measured bottleneck | Action | Evidence needed |
| --- | --- | --- |
| Extraction slower in Claws than identical standalone yt-dlp | Trace invocation, cache/authentication/runtime differences and remove the confirmed extra work | Same-source extraction timing and a reproduced before/after cause |
| Range-capable plain HTTP capped per connection | Task 4 optional HTTP acceleration | Same payload/hash; repeated transfer-rate and total-time comparison |
| HLS/DASH transfer | Tune existing fragment/job limits and inspect source/tool behavior | Same formats at fragment counts 1/4/8; integrity and aggregate throughput |
| Audio conversion dominates | Measure the existing native-audio preference and identify any remaining unnecessary encoder invocation | ffprobe parity and encoder/process timing |
| Merge/metadata/subtitles dominate | Task 3 managed processing and scheduling | Equivalent streams/metadata with fewer passes and lower total completion time |
| Waiting while all slots process | Task 3 separate scheduling | Network idle time reduced without runaway CPU/disk contention |
| Server-wide cap or saturated connection | Report the limit; avoid adding connections that cannot help | Standalone/application parity and controlled aggregate bandwidth evidence |

- [ ] Preserve identical selected formats when comparing engines. A smaller source file is a valid separate user choice but not proof of faster transport.
- [ ] Record per-run extraction, transfer, processing and total time, CPU including children, peak disk usage, retries and integrity.
- [ ] Require at least three timed runs and report median/spread. The initial decision gate is at least 10% lower median end-to-end time in the intended scenario, with no unexplained correctness failure or material controlled regression. This is a gate, not a promised result.
- [ ] If a different bottleneck is found, update this task with its reproduced cause before coding. Do not broaden the project into unrelated refactors.
- [ ] Keep acceleration off when the relevant tests fail or there is no meaningful benefit. Distinguish a scenario-specific improvement from a universal speed claim.

**Acceptance:** a tested cause and a measurable improvement to the intended scenario, or an honest documented external limit. Rendering improvements alone do not satisfy this task.

**Commit boundary:** one focused commit per reproduced speed bottleneck, naming its cause and validation.

## 5. Task 3 — Managed processing and independent scheduling, if justified

**Priority:** main speed work when Task 1 identifies processing/scheduling as significant. **Files:** new `src-tauri/src/transfer_plan.rs`, new `processing.rs`, `downloader.rs`, `state.rs`, `settings.rs`, `api.ts`, `queue.ts`, `settings.ts`, `mock.ts`; process integration fixtures.

This capability remains unimplemented. Current yt-dlp performs transfer and launches FFmpeg inside one process; a post-processing notification is too late to act as a pre-launch processing gate.

- [ ] Define a typed selected-format plan using actual extraction metadata: IDs, protocols, codecs, containers, sizes, duration, language and required subtitle/metadata steps. Reuse the current execution snapshot and byte-progress model rather than replacing them.
- [ ] Download managed separate streams using explicit individual selections without automatic merge. Defer fixups/conversion/embedding to the controlled processing stage. Handle combined media and audio-only cases explicitly.
- [ ] Introduce a waiting-for-processing state consistently in Rust, TypeScript, queue actions, restore logic and counts. Release a transfer permit only after transfer children exit and artifacts are checkpointed.
- [ ] Add a distinct processing limit, initially one. Start FFmpeg only after acquiring that permit. Changing a limit affects future starts, not already-running processes.
- [ ] Keep a tested legacy path for unsupported format plans. A legacy job whose internal processing cannot be gated must reserve both budgets conservatively; do not claim its hidden FFmpeg invocation is separately controlled.
- [ ] Build FFmpeg arguments as arrays, with explicit stream mapping. Stream-copy compatible video/audio. Combine merge, requested metadata and embedded subtitles into one operation where the output container supports it.
- [ ] Preserve chapters, language, rotation, relevant HDR metadata, requested subtitles and audio properties. Use ffprobe comparisons; do not treat a valid container header as full parity.
- [ ] Retry processing failures from retained input streams rather than re-downloading. Cancel/pause/shutdown must obey workspace ownership and the corrected completion transaction.
- [ ] Evaluate unconditional MP3/M4A thumbnail embedding only if measured as material overhead. If exposing a new thumbnail option, freeze it per job, preserve existing preferences, and clearly define whether Fast Save disables it.
- [ ] Test simultaneous processing admission, cancel while waiting, failed FFmpeg, native audio copy/remux, subtitles, long paths, unsupported-plan fallback and interrupted publication.

**Acceptance:** reduced relevant completion/idle time, bounded processing concurrency and format parity. Enable the managed path only for cases with validated support.

**Commit boundaries:** `refactor: define managed transfer and processing plans`; `perf: separate transfer slots and combine compatible processing`.

## 6. Task 4 — Optional parallel HTTP acceleration, if justified

**Priority:** main speed work for a demonstrated per-connection HTTP cap. **Files:** `tools.rs`, `download_policy.rs`, `downloader.rs`, `download_files.rs`, `state.rs`, `settings.rs`, `commands.rs`, `api.ts`, `settings.ts`, `queue.ts`, `mock.ts`; download fixtures/benchmarks.

This capability remains unimplemented despite the existing transport enum. It can use the current yt-dlp process pipeline and does not require Task 3 first.

- [ ] Add an explicit native/aria2c setting, default Native for new and existing users. Route only supported ordinary HTTP/HTTPS media through acceleration initially.
- [ ] Implement verified tool discovery/install/update. During implementation, choose an official release and commit its real version, download URL, published digest and license in a tool manifest. Do not enable installation with an invented or absent digest.
- [ ] Reuse bounded extraction, rollback and engine-use exclusion. A missing or incompatible acceleration tool must not break downloads when the option is off.
- [ ] Route with `--downloader http,https:aria2c`. Inspect the effective yt-dlp command before adding arguments: it already supplies connection/split defaults.
- [ ] Define an aggregate connection budget across jobs. Benchmark modest limits as well as 16; do not multiply 16 connections by every job without accounting for contention or server behavior.
- [ ] Integrate accurate progress through aria2 JSON-RPC on loopback with a random per-job secret and available per-job port. Bind only to 127.0.0.1, keep remote-origin access disabled, redact the secret, and stop the endpoint with its process. Poll at 500 ms initially; use indeterminate progress when unavailable instead of fabricated percentages.
- [ ] Keep `.aria2` control files and partial payloads in the existing owned workspace. Correct cancellation/retry coordination must apply to them too.
- [ ] Test resume after pause/restart, ignored Range requests, changed content identity, expired URLs, authenticated headers, cancellation, child crashes, tool installation failure and checksum mismatch.
- [ ] A fallback must validate partial content compatibility or use a separate owned artifact. Never append native-download bytes to an incompatible partial just to preserve apparent progress.
- [ ] Compare integrity, received/duplicate bytes, transfer time and total completion on rate-limited, unrestricted, no-range and globally capped fixtures. Perform a suitable real-source comparison before claiming the user's issue is resolved.

**Acceptance:** benefit on the intended supported source, functional progress/resume/cancellation, correct fallback and no integrity regression. Keep optional and disabled by default until the gates pass.

**Commit boundary:** `feat: add verified optional HTTP acceleration`.

## 7. Task 5 — Validate, document and package the new result

**Priority:** mandatory release gate. **Files:** relevant tests and docs, `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `Cargo.lock`, `tauri.conf.json`, `installers/`.

- [ ] For future changes, run each new regression against the old behavior before its fix, then against the corrected implementation. Preserve the existing lifecycle coverage.
- [ ] Run the existing checks and new tests:

```powershell
npm ci
npm run build
npm run test:queue
python -m unittest discover -s scripts -p test_font_metrics.py
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml -- -D warnings
git diff --check
```

- [ ] Use a disposable Windows profile to test cancel/retry/restart, blocked cleanup, interrupted publication, Library write failure, target conflict and disk-budget admission. Do not alter the owner's real queue/history as a test.
- [ ] Test representative large video/audio, identical-format source comparisons and every enabled acceleration/processing path. Keep raw benchmark evidence and record unmeasured boundaries honestly.
- [ ] Update `docs/CURRENT_PROBLEMS_AND_SPEED_REVIEW.md` when a defect actually closes, with its test evidence. Update the handoff, settings help and user instructions to match shipped behavior.
- [ ] Verify a fresh checkout and installer upgrade from 1.0.4. Preserve existing user settings/history and ambiguous legacy files. Retain the current Polaris spacing regression coverage and bundled license.
- [ ] For the future speed release, select the next unused version after checking tags. Synchronize all five version-bearing files and rebuild after the final change with `npm run tauri build`.
- [ ] Compute SHA-256 of the exact verified installer. Add the new installer and checksum to `installers/` as requested for this repository; do not pass off the older 1.0.4 artifact as this build.
- [ ] When committing/publishing is authorized, target the exact verified source commit, upload to a draft release, compare the uploaded digest/size, publish and verify Latest. Use ordinary commits; no history rewrite is needed.
- [ ] Describe speed gains only for measured scenarios. If speed work found an external cap, say so rather than claiming a fix that the application cannot deliver.

**Acceptance:** critical defects closed with regression evidence; intended speed scenario measured; installer/upgrade verified; source and artifact versions/checksums agree; no debug ports, fixture executables or credentials bundled.

**Commit boundary:** `release: validate and package remaining download fixes`.

## 8. Remaining-work acceptance matrix

| Item | Required result |
| --- | --- |
| Actual slow-source comparison | Same selected formats, repeated runs, integrity and stage timings |
| Managed processing, if enabled | Bounded slots, output parity and meaningful completion-time benefit |
| aria2c, if enabled | Verified binary, safe ranges/resume, working progress and measured target benefit |
| New installer | Built from corrected source, versioned, hash-verified and upgrade-tested |

## 9. Evidence and reference documents

- [Current unresolved findings and earlier status audit](../../CURRENT_PROBLEMS_AND_SPEED_REVIEW.md).
- [Recorded HTTP policy comparison](../../performance/download-speed-review-2026-11-04.json).
- [Current Queue benchmark and remaining initial long task](../../performance/queue-ui.md).
- [yt-dlp options](https://github.com/yt-dlp/yt-dlp#download-options).
- [yt-dlp FFmpeg processing](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/postprocessor/ffmpeg.py).
- [yt-dlp external downloader behavior](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/downloader/external.py).

Upstream behavior must be checked against the supported installed version during implementation. The HTTP result does not prove real-site throughput; the Queue result does not measure network speed. Keep historical implementation records outside this remaining-work plan.

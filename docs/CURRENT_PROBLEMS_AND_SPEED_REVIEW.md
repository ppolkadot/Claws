# Current problems and download-speed review

**Date:** 2026-11-04  
**Reviewed branch:** `download-reliability`  
**Reviewed commit:** `7c4f981`  
**Baseline before this branch:** `a9b6cbb`  
**Scope:** consolidate unresolved review findings, distinguish implemented fixes from unfinished performance work, and check whether the current transfer policy resolves low download speed.

## Executive conclusion

The branch improves reliability, queue responsiveness, and optional post-processing behavior. It does **not yet establish a fix for low raw download speed**.

A controlled plain-HTTP comparison produced essentially identical throughput before and after the policy changes. Parallel HTTP acceleration, independent processing slots, and combined FFmpeg processing are not implemented. There are also five outstanding correctness/recovery defects that should be fixed before a release.

The tracked installer is still the earlier 1.0.4 artifact from `a9b6cbb`. It does not contain these 11 branch commits. This observation concerns the repository installer; it does not identify which executable the owner currently has running.

## Owner priority update

**Low download speed is the main improvement priority.** Only correctness fixes necessary to prevent lost progress, unsafe cleanup, broken recovery or unusable downloads take precedence. This ordering supersedes the earlier suggestion to finish every general improvement before acceleration work.

- Start speed diagnosis immediately; it can run alongside the critical fixes.
- Resolve B1–B4 before shipping a speed-focused release: these protect retry data, completion recovery, publication and required persistence.
- Resolve B5 before throughput comparisons if its false disk-space rejection blocks the test/download; otherwise implement it alongside the speed work.
- After those prerequisites, prioritize the measured transfer or processing bottleneck over cosmetic changes, speculative refactors, additional Queue optimizations and unrelated enhancements.
- The priority is a demonstrated improvement on the affected download path. It is not a requirement to add aria2c when the source, protocol or measurement shows a different cause.

## 1. Open correctness defects

### B1 — Retrying a cancelled job leaves its cancellation flag set

**Status (2026-11-05): closed in `9dc6531`.** Try Again on a cancelled job starts a new attempt: the cancel, error and progress are cleared, the frozen settings are kept, and the attempt gets its own work folder (`<id>-<n>`) whose marker names the attempt, so an earlier attempt's cleanup cannot reach it. The start's own save barrier (B4) records the reset before any byte is written. Lifecycle tests through the real downloader cover Cancel → Retry → restart → resume (bytes kept and resumed), a retry while the old attempt's file is locked, repeated Cancel/Retry, removal during a retry and shutdown right after a retry; four of them fail against the old behaviour. The locked-file case also exposed and fixed a cleanup that could delete a work folder's marker before failing on a locked file, leaving a folder that no later cleanup could prove was Claws' own.

**Priority:** P1. **Evidence:** code-traced; not reproduced through the native Retry UI during this review.

**Locations:** `src-tauri/src/downloader.rs`, `resume()` around lines 565–582, cancellation around lines 586–624, and `restore_queue()` around lines 314–345.

**Trigger and result:**

1. Cancel a job. `cancel_requested` becomes true.
2. Choose Try Again. `resume()` changes the status to Queued but does not reset that flag.
3. The retry starts and its queue state is saved while the flag remains true.
4. Restart Claws before the retry completes.
5. Startup interprets the saved job as an unfinished cancellation and schedules its current workspace for deletion.

This can discard the new retry's accumulated bytes. A related race needs coverage: a cancelled job can be retried while its asynchronous cleanup is still pending.

**Required fix:** serialize cleanup completion and retry admission. Once cleanup is settled, reset cancellation intent durably before admitting the new attempt. Do not reopen a workspace while an earlier cleanup can still delete it.

**Regression cases:** Cancel → Retry → partial transfer → restart; immediate Retry while cleanup is blocked by a file handle; repeated Cancel/Retry; restart while cleanup is pending.

### B2 — Completion can lose its durable recovery record before Library save

**Status (2026-11-05): closed in `ea1f0c3`.** A published download is recorded in the Library first; only then is it settled, retired from the queue (a required save) and its work folder cleaned (never while a side file is withheld). A failed Library save leaves the job failed, queued and with its manifest; Try Again only records it. At start, an unrecorded completion is recorded exactly once, or stays failed while the Library still cannot be saved. Lifecycle tests cover a failed Library save, restarts with and without a writable Library (one entry, no second download) and fail against the old ordering.

**Priority:** P1. **Evidence:** code-traced ordering defect; no forced native-app crash injected at this exact boundary.

**Locations:** `downloader.rs`, `queue_snapshot()` around line 257, publication cleanup around line 1377, and `finish()` around lines 1494–1537.

`finish()` settles the job as Completed, then flushes the queue. The queue serialization excludes completed jobs. Only afterward does it call `history::record()`. Meanwhile, publication may already have scheduled deletion of the workspace and its manifest.

A crash after the queue flush but before the Library save can leave a valid final file on disk with neither a saved queue entry nor a discoverable completion record. The Library cannot reliably recover it. The queue flush does not make the Library write durable.

**Required fix:** retain an authoritative completion/recovery record until the Library entry is durably saved. Save history before retiring that record, then remove the queue entry and clean the workspace. History-write failure must leave a recoverable transaction, not just a notification.

**Regression cases:** crash after final move, after Completed is set, after queue flush, during history save, and just before workspace removal; exactly one Library entry after recovery.

### B3 — Publication cannot reconcile a completed move missing from its manifest

**Status (2026-11-05): closed in `ea1f0c3`.** The manifest (schema 2) journals each output: intent and file identity are saved before the move, the result after. A move whose result was lost is recognised by identity (NTFS/ReFS volume and 128-bit file id, which a same-volume rename keeps; size and SHA-256 elsewhere), never by name; a replaced or missing destination is an explicit recoverable error and is never overwritten. Tests rewind the journal to each interruption point (before a move, after a move before its record, partial multi-file sets, replaced and missing destinations, schema 1 records), including this review's exact reproduction state, and a lost record is finished through Try Again in the app without a second download.

**Priority:** P2. **Evidence:** reproduced in an isolated harness compiling the actual `download_files.rs` module.

**Location:** `src-tauri/src/download_files.rs`, `publish()` around lines 290–297.

The file is moved to the destination before its path is appended to `manifest.published` and the manifest is saved. An interruption between those operations leaves a Publishing manifest, a missing source, and an existing final file. The current retry tries to move the missing source again.

**Observed result:** retry failed with “The system cannot find the file specified,” although the final media file existed. The harness modeled the exact persisted filesystem state at the interruption boundary; it did not kill the full application.

**Required fix:** journal move intent and record sufficient identity evidence before moving. Recovery must distinguish a file published by this transaction from a pre-existing or replaced destination. Do not resolve this by accepting any existing file with the same name.

**Regression cases:** interruption before/after each media and sidecar move; an unrelated destination appearing during recovery; missing or changed published media; partial multi-file publication.

### B4 — Required persistence barriers ignore failure

**Status (2026-11-05): closed in `c7a3c45`.** Required queue saves return a result that callers honour; the writer fails a barrier as soon as an attempt covering it fails. Every start saves before yt-dlp is spawned (a failed save starts nothing and keeps the folder); cleanup deletes nothing without both the queue state and the `cleanup.json` record; Remove refuses rather than forgetting files; shutdown keeps cancelled files and reports a failed final save. Lifecycle tests use real rename failures (a folder where the JSON file must go) and fail against the old behaviour. A follow-up (`b65c562`) retries a save whose rename is refused for a moment (an antivirus opening the fresh file), which the tests hit on an ordinary cancel.

**Priority:** P2. **Evidence:** code-traced.

**Locations:** `downloader.rs`, `flush_queue_async()` around lines 286–289, `prepare()` around line 1304, and cleanup around lines 674–685.

`flush_queue_async()` discards the boolean returned by `flush_queue()` and the blocking task's failure result. Callers continue as though their required checkpoint succeeded. A new process can start without a durable execution/workspace snapshot; destructive cleanup can proceed without the expected saved transition. Saving the cleanup list also reports an error without preventing deletion.

**Required fix:** return a typed result from required barriers and propagate it. Prevent the dependent start/deletion when its prerequisite cannot be saved. Keep pending state and provide a retryable error. Ordinary coalesced progress saves can remain asynchronous.

**Regression cases:** full disk, denied queue writes, worker failure, timed-out flush, cleanup-list write failure and subsequent successful retry. Verify that no protected operation runs after a failed required checkpoint.

### B5 — Disk reservations double-count consumed space

**Status (2026-11-05): closed in `5167328`.** A ledger holds each job's remaining demand (estimated peak minus what its work folder holds, sampled every 2 s, plus margin); admission reads free space under the ledger's lock; each whole-file processing step rechecks what is still needed and is stopped before it runs (streams kept) if it no longer fits; a claim is released when the process exits. Tests cover this section's 22-of-30 GiB example, simultaneous admission, resumed partials, a pending merge and release (pure, with set free-space numbers), plus app-level admission and a merge stopped for space; they fail with the old static reservation. Not done: a physical low-disk run on disposable storage.

**Priority:** P2. **Evidence:** code-traced accounting defect.

**Locations:** `downloader.rs`, `Budget::after_line()` around lines 1143–1175 and reservation removal around line 1439.

The reservation is calculated from the initial additional requirement and remains unchanged until the worker ends. As the job downloads, real free space decreases, but later jobs still subtract the original reservation. Bytes already on disk are therefore counted again against capacity.

For example, a job reserving roughly 22 GiB can consume 10 GiB while keeping its original reservation. On a volume now showing 30 GiB free, another job sees only roughly 8 GiB available even though much of the first job's reservation has already materialized on disk.

**Required fix:** reservations must represent remaining additional demand, including future processing headroom. Update them as bytes materialize and stages change. Serialize admission/accounting consistently and avoid scanning whole workspaces on every progress tick.

**Regression cases:** two jobs sharing a volume; one largely downloaded before another is admitted; resumed partials; unknown totals; merge stage transition; cancellation releasing capacity.

## 2. Remaining performance limitations and validation gaps

These are unfinished capabilities or limits, not claims of reproduced corruption.

| ID | Current limitation | Consequence | Required next work |
| --- | --- | --- | --- |
| L1 | Plain HTTP remains on the native transfer engine; no aria2c invocation or HTTP range-parallel acceleration is wired up | A server's per-connection bandwidth limit still applies | Benchmark supported range downloading, then implement optional acceleration with resume/progress/cancellation coverage |
| L2 | Downloading and Processing both consume the same concurrency limit | If every slot is merging/converting, no queued job can start transferring | Implement the managed transfer/processing boundary and separate limits; changing the counter alone is insufficient |
| L3 | Metadata and embedded subtitles remain separate whole-file operations in the existing pipeline | Large files can still incur extra complete disk passes | Honor Fast Save and investigate combining compatible processing steps with stream-parity tests |
| L4 | MP3/M4A still request thumbnail embedding, including when Fast Save is selected | Fast Save does not eliminate every optional audio-processing operation | Decide whether thumbnail embedding should become an explicit independent choice; measure its cost before changing behavior |
| L5 | Fragment concurrency remains fixed at eight per job | No protocol/connection-aware tuning; many active jobs can contend for bandwidth and disk | Compare 1/4/8 fragments and aggregate job counts before changing defaults |
| L6 | A 500-job addition still creates all rows in one frame | One initial long task remains despite much cheaper subsequent updates | Measure whether chunked creation or virtualization is needed; preserve keyed rows and accessibility |
| L7 | Existing settings migrate to Best audio selection | Upgrading users do not automatically get the matching-native-audio preference | Explain the setting; enable Prefer Matching explicitly when avoiding conversion is desired |
| L8 | Real constrained-disk behavior, installed-build upgrades and live-site large downloads are not established by the review tests | Unit/fixture success does not prove these end-to-end paths | Validate in disposable Windows environments and perform controlled real-media comparisons |
| L9 | The committed 1.0.4 installer predates this branch | Installing that artifact does not exercise the current changes | Build and validate a newly versioned installer after fixing the blockers |

For L4, this is a processing-cost limitation rather than a contradiction of the current Fast Save contract, which specifically disables metadata embedding and keeps subtitles as sidecars.

## 3. Status of all previously reported findings

The original review IDs are retained here as a status audit. The [active implementation plan](superpowers/plans/2026-11-04-download-reliability-and-performance.md) contains only remaining work and uses new task numbers.

| Finding | Current status |
| --- | --- |
| R1 — Broad cleanup prefix deletes unrelated files | Addressed by exact legacy suffix matching and isolated workspaces. B1/B4 remain lifecycle defects. |
| R2 — Merge/stream intermediates remain after cancellation | Owned-workspace cleanup is implemented. Retry/cleanup coordination still needs B1. |
| R3 — Missing fragments can yield incomplete success | Strict fragment handling implemented; fixture regression passes. |
| R4 — Duplicate jobs share output/partial paths | Workspaces, duplicate checks and no-replace publication implemented. Crash reconciliation still needs B3. |
| R5 — Resume adopts changed defaults | Execution snapshots implemented. This does not resolve cancelled-job retry state in B1. |
| R6 — Whole queue repainted on each progress tick | Incremental updates implemented; frontend benchmark shows substantially lower layout/style cost. |
| R7 — Enqueue event/response duplication | Revisioned batching and duplicate/stale-event rejection implemented. |
| R8 — Synchronous queue saving blocks runtime workers | Background writer implemented. Required save failure handling still needs B4. |
| R9 — Unnecessary audio encoding | Prefer Matching selection implemented and tested for source choice. Actual long-file conversion-time savings remain unmeasured. |
| R10 — Single-connection HTTP throughput ceiling | Not addressed by this branch; no connected aria2c transport. |
| R11 — Insufficient retry backoff | Finite exponential backoff implemented; transient-failure fixtures pass. This is resilience rather than higher clean-path throughput. |
| R12 — Excess progress output | Source-side 0.2-second progress delta implemented and tested. |
| R13 — Extra metadata/subtitle rewrites | Explicit Fast Save and clearer hints implemented; combined processing not implemented. |
| R14 — Processing occupies network slots | Not addressed yet; both statuses are still counted as active by the same scheduler. |
| R15 — Missing disk-space budgeting | Estimates/watch logic implemented, but B5 remains and real low-disk scenarios need validation. |
| R16 — Equal stream weighting and misleading ETA | Byte-weighted progress, unknown/estimated totals and transfer-only ETA implemented with tests. |
| R17 — Untuned fixed concurrency | Still benchmark gated; no adaptive policy or changed fragment default. |
| H1 — Unbounded child output | Metadata, diagnostics and line bounds implemented with tests. |

## 4. Does this branch fix low download speed?

### Direct before/after HTTP policy test

**Answer for the tested scenario: no measurable transfer-speed improvement.**

The comparison used the same installed yt-dlp executable, a localhost plain-HTTP payload, and otherwise identical settings. It compared the previous five-retry policy against the current retry/backoff/strict-fragment/progress-delta policy. Both used eight fragment workers, which does not add parallel range connections to this plain HTTP transfer.

| Parameter | Value |
| --- | --- |
| Payload | 64 MiB generated binary file |
| Server behavior | Supports Range; 16 MiB/s limit per connection |
| Transfer engine | Native yt-dlp in both configurations |
| Tool version | yt-dlp 2026.10.26 |
| Warm-ups | One per policy, excluded from results |
| Timed runs | Three per policy, with alternating execution order |
| Integrity | Every output matched the fixture's SHA-256 |
| Processing | No conversion, metadata embedding, subtitles or FFmpeg fixups |

| Configuration | Median transfer time | Approximate throughput |
| --- | ---: | ---: |
| Previous policy | 4.005346 s | 15.98 MiB/s |
| Current policy | 4.006694 s | 15.97 MiB/s |

The current median took about **0.034% more time**. This is a negligible difference, not evidence of a meaningful regression. It shows effectively unchanged throughput in this controlled per-connection-limited case.

Raw measurements, exact policy arguments, per-run hashes and environment information are in [download-speed-review-2026-11-04.json](performance/download-speed-review-2026-11-04.json).

**Limits:** this was a direct engine-policy comparison, not a full native Claws application benchmark. It does not measure the UI, a real CDN, multi-gigabyte media, authentication, HLS acceleration, merge time, codec conversion or every possible bottleneck. A 64 MiB synthetic result cannot establish universal behavior for 10 GiB Internet downloads. It also cannot identify the specific cause of a slow site without measuring that source.

### What has improved

- Retry/backoff and strict fragments improve recovery and integrity when failures occur.
- Progress throttling reduces output/parsing overhead; it does not remove a server's bandwidth cap.
- Incremental queue updates reduce interface work and stutter.
- Matching native audio can avoid conversion where a suitable source exists and the preference is enabled.
- Fast Save can reduce total time after transfer by avoiding requested whole-file metadata/subtitle embedding.

These changes can make a task feel faster or finish sooner in the relevant circumstances. They are not evidence that the clean-path network transfer rate increased.

### Existing frontend benchmark evidence

The branch includes a development-browser benchmark in [queue-ui.md](performance/queue-ui.md). For 500 added jobs with eight active transfers, it reports:

- Addition layout/style cost: approximately 4,370 ms before, 73 ms after.
- Layout/style cost across 40 rounds of progress: 729 ms before, 63 ms after.
- Progress-period long tasks: 39 before, zero after.
- One initial long task still occurs when creating 500 rows.

Those are checked-in single-run results on one machine, not new measurements performed for this summary. They concern frontend rendering, not network MB/s or total download duration.

## 5. Installer and release state

- `installers/Claws_1.0.4_x64-setup.exe` was committed in `a9b6cbb`, before the reliability branch.
- Its SHA-256 remains `9aadc5fedde599b89fc53e199366caac465a4c3ddcf4032b0c7454f07efce955`.
- This is the earlier font-spacing repair artifact. The current branch's new reliability and performance logic is not in that tracked installer.
- The source version is still 1.0.4. A release candidate needs a new, consistently applied version, a fresh build, upgrade validation, and matching repository/release asset checksums.
- No assertion about a currently running installed application was made by this report; its executable would need to be identified separately.

## 6. Verification evidence and boundaries

During the preceding review of the same `7c4f981` revision:

- `cargo test --locked`: 94 tests passed.
- `npm run test:queue`: 10 tests passed.
- Font regression tests: two passed.
- `npm run build`: passed.
- `git diff --check`: passed.
- An isolated actual-module harness reproduced B3's publication crash gap.

For this summary:

- The reviewed revision and the five defect paths were checked again; no fix commits had appeared.
- The direct HTTP comparison above completed successfully with matching output hashes.
- Current argument construction, scheduler status counting and repository installer provenance were inspected.

Passing tests do not close B1–B5. Each needs a dedicated regression at its actual lifecycle boundary. No live-site large-media transfer, native installer upgrade, physical disk-full test or universal download-speed claim is included.

## 7. Prioritized fix list

1. **Begin diagnosis of the low-speed problem now.** Measure extraction, transfer, processing and total time for a representative slow source. Compare Claws against standalone yt-dlp using the same selected formats, tool version and authentication. Existing localhost results are a baseline, not proof about that source.
2. **Complete the critical correctness prerequisites before release:** B1 cancelled-job retry/cleanup coordination; B2 durable completion/history ordering; B3 recoverable publication; B4 enforced save barriers. Add regression tests at the failure boundaries. These fixes can proceed while measurements run.
3. **Correct B5 disk accounting** before benchmarking if it blocks the affected download; otherwise fix it alongside the main speed work.
4. **Implement the measured speed fix as the main development task:**
   - Plain HTTP with a demonstrated per-connection cap: prioritize optional range-parallel acceleration, connection budgeting and working progress/resume/cancellation.
   - HLS/DASH: benchmark fragment/job concurrency and source/extractor behavior. Do not assume that increasing eight fragments is the fix.
   - Slow merge/conversion: prioritize native audio, reduced processing passes and independent transfer/processing scheduling.
   - Extraction delay or a source-wide/network limit: fix a confirmed extraction/tool issue where possible and report a limit the application cannot remove honestly.
5. **Prove the result.** Use repeated identical-format comparisons, integrity checks, end-to-end timing and regression tests. Keep acceleration disabled if it fails correctness or provides no meaningful benefit in its target scenario.
6. **Build and validate a newly versioned installer** from the corrected source; verify the upgrade, embedded assets, repository installer and release checksums.
7. **Then address secondary improvements:** the remaining initial Queue-render long task, optional UI refinements and additional hardening that does not block correct operation or the measured speed fix.

**Release recommendation:** do not call the branch release-ready or advertise “low download speed fixed” until the correctness blockers are resolved and the intended real-world speed scenario is measured.

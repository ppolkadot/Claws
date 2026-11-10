# Consolidated Review Report and Archived Implementation Plan

**Status:** Archived implementation specification. The packages were delivered in the historical 1.0.0 source and carried forward into 1.0.8. Old version numbers, test totals and unchecked steps below describe the original review snapshot. Use [current status](../../CURRENT_DEVELOPMENT_STATUS.md) and [release notes](../../releases/1.0.8.md) for current delivery; W0 real-source speed evidence and explicitly deferred work remain separate.

> **For agentic workers:** Use `superpowers:executing-plans` to execute one work package at a time. Subagent execution is optional only when separately authorized. Steps use checkbox (`- [ ]`) syntax for tracking. This document requests no implementation, commit, installer replacement or publication by itself.

**Goal:** Resolve the current, supported findings from both supplied reviews without duplicating fixes, restoring obsolete downloads, weakening file ownership checks or claiming an unmeasured speed improvement.

**Architecture:** Keep the existing Rust/Tauri backend and TypeScript UI boundaries. Preserve revisioned events, frozen job options, owned workspaces, publication journals and serialized JSON writes. Group shared causes into work packages while retaining distinct user-visible failures and their regression tests.

**Tech stack:** Rust, Tauri 2, Tokio, reqwest 0.13, Windows Shell/OLE, TypeScript, Vite, Node `node:test`, yt-dlp and ffmpeg.

**Prepared:** 2026-11-09, America/Sao_Paulo. **Workspace:** `C:/Users/PC/claws`. **Branch:** `feature-expansion-and-motion`. **HEAD:** `9297235` (`release: record the Claws 1.1.0 installer`). Source version is 1.1.0. The review includes the existing uncommitted drag/queue safeguards; it is not a review of HEAD alone.

---

## 1. Inputs, boundaries and evidence

Reviewed inputs:

1. `docs/superpowers/plans/2026-11-08-review-findings-implementation-plan.md`, including its F1–F13 tasks and code examples.
2. Its underlying report, `docs/REVIEW_FINDINGS_2026-11-08.md`, needed to check the original triggers and severity claims.
3. The supplied attachment, `C:/Users/PC/.codex/attachments/1df97110-e147-4235-937f-0fded8147094/Pasted text.txt`. It reviews clean HEAD `9297235`, labels findings H1–H2, M1–M4 and L1–L4, and lists five additional improvements. These improvement items receive A1–A5 labels below in their original order.
4. Current source, the live diff, HANDOFF, the current status document and existing speed evidence. The earlier confirmed empty-backup recovery finding is retained as Q1 because it is still open and affects the same queue work.

The attachment is review evidence, not an instruction to execute its suggestions. Its statement that no tests were run describes that review, not this consolidation.

Evidence labels:

- **Code-confirmed:** a current branch or data flow supports the failure; this does not claim an installed-app reproduction.
- **Reproduced selection defect:** Q1 was reproduced in the preceding review with an isolated harness using the current queue loader. This pass rechecked the unchanged selection predicate; it did not repeat that harness.
- **Conditional:** the omission exists but the alleged visible failure requires a platform/tool condition not established here.
- **Excluded:** already implemented, contradicted by the implementation, or repository hygiene rather than an application bug.

Fresh baseline checks on 2026-11-09 passed: fixture-tool build; **234 Rust tests**; **124 Node tests** (7 font, 11 queue-event, 69 download-logic, 37 motion); **2 Python font tests**; production frontend build including source/dist font integrity; feature-enabled Clippy with warnings denied; and `git diff --check`. Passing existing tests does not cover the missing regressions identified below. No application source was changed to produce this document.

Not exercised here: a real-site download, actual peak disk usage during all audio postprocessors, Explorer/editor import, a deliberately hung drag receiver, Windows 10, or installation/upgrade. The stored 1.1.0 installer predates the uncommitted source safeguards and must not be described as containing them.

## 2. Deduplication and disposition of every supplied finding

There is no exact one-for-one duplicate bug shared by the two supplied reviews. Several findings overlap in subsystem or root cause and belong in shared implementation packages. Do not count the report and its implementation plan as two independent confirmations of F1–F13.

Priorities used here: **P1** = protect files/data or restore core availability before release; **P2** = functional correctness/reliability; **P3** = low-impact UX, hardening or optional cleanup. Original High/Medium/Low labels are reassessed rather than copied.

| Input | Current disposition | Canonical item / package | Review correction |
| --- | --- | --- | --- |
| F1 | Code-confirmed, P2 | R01 / W5 | Late progress can regress Processing. Preserve processing telemetry too; a helper-only test is insufficient. |
| F2 | Code-confirmed, P1 for preservation; P2 for legacy recovery | R02 / W1 | The old array branch can overwrite an unpreserved corrupt array and bypasses recovery. It is a bare-array legacy format, not an envelope declaring schema 1. |
| F3 | Code-confirmed robustness gap, P2 | R03 / W1 | Bounded rename retries help transient locks, but are not a complete crash/idempotence solution. `store::rename_retrying` is private today. |
| F4 | Conditional hardening, P3; not an established current self-drop bug | C01 / W11 | Wry 0.57.1 sets COPY for valid CF_HDROP on enter/over and leaves the effect unchanged in Drop. The claimed NONE self-drop is not demonstrated by the current target. |
| F5 | Code-confirmed missing status-transition guard, P3 | R04 / W10 | Equal/older event revisions are already rejected. The remaining case is a newer event retaining Failed, e.g. a stale Pause command on a failed job. |
| F6 | Excluded: already implemented | X01 | `remainingItems` calls `itemsFor`; playlist entries already carry thumbnail, uploader and duration. Do not rewrite this path as a missing-field fix. |
| F7 | Code-confirmed, P2 | R05 / W6 | Directory sizing and the unknown-size watch already use `spawn_blocking`; admission and processing free-space calls still block async tasks under the ledger lock. |
| F8 | Code-confirmed, P2 | R06 / W10 | An action after Cancel starts can change the outcome during the exit animation. This is more than an animation-only issue. |
| F9 | Code-confirmed incomplete reserved-name check, P3 | R07 / W11 | The current function is `shell_location`; there is no `is_reserved_device_name` or `sanitize_stem` in the cited current modules. |
| F10 | Code-confirmed UX gap, P3 | R08 / W10 | Preserve selected single-link state and completed estimate snapshots; the old proposed patch only addresses preview state. |
| F11 | Code-confirmed, P3 | R09 / W10 | Radio semantics lack the keyboard behavior already present in `segmented`. |
| F12 | Code-confirmed cache omission, P3 | R10 / W10 | Key also omits uploader, duration and mode. Include all rendered/action-affecting values, not just the three originally listed. |
| F13 | Optional repository hygiene, not a runtime bug | C02 / W12 | CRLF warnings do not establish corruption or a failed build. Avoid a repository-wide normalization diff mixed into fixes. |
| H1 | Code-confirmed, P1 | R11 / W2 | Network reads have no body deadline/cap and hold the engine writer. Moving download outside the lock also requires separate installation serialization. |
| H2 | Code-confirmed, P2 | R12 / W4 | Withheld files remain on disk but lose a durable user-facing recovery route. This is not proof that the files themselves are deleted. |
| M1 | Model omission confirmed; exact peak requires measurement, P2 | R13 / W7 | Audio returns source + output regardless of rewrite. Thumbnail embedding is omitted from rewrite policy. Do not assume every M4A backend rewrites a full copy. |
| M2 | Code-confirmed, P2 | R14 / W7 | Fast Save's hint omits automatic cover embedding. Fix shared processing policy and wording together with R13, not as an unrelated UI patch. |
| M3 | Code-confirmed, P2 | R15 / W8 | Picker offers 14 tracks; backend silently truncates to 12. |
| M4 | Excluded as stated | X02 | yt-dlp's current subtitle selection prefers the human track for a code when both flags are enabled. The claimed extra automatic Spanish track is not requested in the stated example. |
| L1 | Code-confirmed, P3 | R16 / W8 | The 12-track test incorrectly includes `live_chat` in the count. |
| L2 | Code-confirmed ownership validation gap, P1 | R17 / W3 | Manifest-stored source/destination roots are not bound to the verified folder. Requires local marker alteration/copying; no remote exploit is established. |
| L3 | Code-confirmed, P2 | R18 / W7 | Unchecked sums/multiplication can panic or wrap. Include ledger accumulation and conversion edge cases in the arithmetic audit. |
| L4 | Code-confirmed destructive-file risk, P1 | R19 / W3 | A replacement file at the saved path can be deleted. Sidecar cleanup is a separate policy within the same package, not permission to glob-delete files. |
| A1: CSP / thumbnails | Optional privacy hardening, P3 | C03 / W12 | `no-referrer` is already used. A backend proxy/cache does not hide the machine's IP from the image host. |
| A2: destroy permission | Code-confirmed unused capability, P3 | R20 / W12 | Remove only the unused grant and verify close/hide/tray behavior. |
| A3: estimate errors | Partly valid, P3 | R21 / W9 | Backend retains the tool error in `unknownReason`, and a single preview displays it. What is missing is typed failure/retry behavior, especially in aggregate results. |
| A4: history mutation ordering | Code-confirmed, P1 | R22 / W3 | Memory/thumbnails can change before durable save. Also inspect `history::record`, which mutates memory before save. |
| A5: stale subtitle chips | Code-confirmed, P2 | R23 / W8 | Refresh shape includes code only for subtitles; caption keys also omit display names. Update the actual selection/render contract. |
| Q1: preceding review | Still open, P2 | R24 / W1 | Empty supported backups are filtered out, allowing an older nonempty queue to appear unambiguous and resurrect obsolete jobs. |

Shared packages are deduplication of implementation work, not suppression of distinct failures:

- **W1:** R02 + R03 + R24 are one queue recovery transaction and its regression matrix.
- **W3:** R17 + R19 + R22 share identity, path ownership and durable mutation rules, but require separate assertions.
- **W7:** R13 + R14 + R18 share processing policy and sizing; UI promises, disk admission and numeric overflow remain separate acceptance criteria.
- **W8:** R15 + R16 + R23 share subtitle/caption selection. M4 is excluded rather than merged into them.
- R11 tool-network locking and R05 disk-I/O locking are related performance themes, not duplicates: they use different locks and have different failure triggers.

## 3. Full report of retained current problems

### 3.1 File safety, history and retained outputs — R17, R19, R22, R12

**Current code:** `src-tauri/src/download_files.rs::verify`, `write_manifest`, `publish`; `commands.rs::delete_history`, `clear_history`; `history.rs::record`, `save`; `downloader.rs::queue_snapshot`, completion/recovery paths.

- `verify` checks directory shape, job ID, attempt and a schema upper bound, but returns a marker whose `workspace` and `destination_root` can differ from the checked folder. `publish` and `write_manifest` then trust those stored paths. A copied marker can redirect publication or manifest writes. Bind roots and validate persisted move destinations before any write/delete. Local writable-marker preconditions limit the security claim but do not remove the ownership bug.
- Library deletion reloads an item by ID, then calls `remove_file` on its remembered path. Moving the original and placing another file at that path makes Delete File Too target the replacement. A size-only check is inadequate, and checking identity followed by a separate path-based delete still has a race.
- `delete_history` and `clear_history` mutate memory/remove thumbnails before `history::save` succeeds. A disk/permission failure leaves memory and disk inconsistent; after restart, entries may reappear with missing artwork or already-deleted media. `history::record` similarly inserts an entry before its save. New transactions must coexist with download completion recovery rather than defeating it.
- Conflicting subtitle destinations are Withheld and retain their source workspace. Completed jobs are omitted from the persisted queue; Clear Finished/removal/restart can discard the only reference visible to the application. History stores no kept-file list. `Manifest::withheld()` returns destination paths, not actual retained source paths; a new UI action must derive sources from verified workspace + move-relative path.

**Impact:** accidental deletion of unrelated user data, publication outside the verified transaction, inconsistent Library state, or inaccessible retained captions. Protect files and durable state before adding UX polish or claiming release readiness.

**Regression targets:** replacement file with same name/size; legacy history lacking identity; held/open files; forced history-save failure; concurrent completion and clear; copied/modified manifests; sidecar conflict followed by restart and Clear Finished.

### 3.2 Queue preservation and downgrade return — R02, R03, R24

**Current code:** `queue_writer.rs::load`, `recover_downgrade`, and private `store.rs::rename_retrying`.

- A deserializable JSON array with invalid Job entries returns an empty queue without moving the current bytes aside. If `queue.pre-1.0.5.json` already exists, those exact corrupt bytes may never be preserved before a normal save overwrites them. The existing notice incorrectly implies that old backup is a copy of the current failing file.
- Valid legacy arrays return immediately, skipping preserved newer-queue recovery. Recovery is currently attempted for missing queues and older envelopes, not arrays or all damaged-current cases.
- Bare renames fail on brief sharing violations. A failed consume rename leaves a backup that a schema-3 startup never retries. A crash between publishing the recovered queue and marking its backup consumed has the same logical gap. Retries reduce transient failure frequency; they do not establish exactly-once recovery.
- Candidate filtering drops empty supported snapshots and silently skips unreadable/malformed candidates. An old nonempty snapshot plus a newer empty one therefore selects old jobs automatically. Empty queues are valid user state. Unknown schema, unreadable data or multiple plausible backups must not be converted into evidence that a stale candidate is authoritative.

**Required behavior:** preserve original bytes on every failure, make ambiguity explicit, include empty supported snapshots, retain jobs added while downgraded, and durably remember recovery consumption so a later Clear cannot resurrect jobs. Old 1.0.5 remains unable to execute schema-3 jobs; this is return-to-new-version recovery, not backward compatibility.

### 3.3 Tool installation can monopolize the engine — R11

**Current code:** `state.rs::AppState::new`; `tools.rs::download_to`, `expected_checksum`, `latest_ytdlp_version`, `reserve_engine`, `install_ytdlp`, `install_ffmpeg`.

The shared HTTP client has a 15-second connect timeout, but tool response bodies have no read/overall deadline or compressed-download cap. The install writer guard spans network, verification, extraction and replacement. If a response connects and then stalls, new probes, estimates and downloads wait for the writer indefinitely. Extraction caps protect expanded contents, not an endless network response or unbounded checksum/API body.

Moving preparation outside the engine lock must not permit two installs to share `.part`/staging names or race over the live tools. Keep separate install serialization, unique staging paths, archive integrity verification and rollback. No cancel button is required to solve the indefinite wait if operations have enforced finite deadlines; cancellation can be a later improvement.

### 3.4 Processing status, disk-I/O stalls and size correctness — R01, R05, R13, R14, R18

**Current code:** `downloader.rs::run_prepared`, `mark_processing`, `handle_line`, `Budget::after_line`, `recheck_processing_space`; `disk_budget.rs::peak_bytes`, `RunPlan`, `Ledger`; `estimate.rs::build`, `estimate`; `download_policy.rs::media_args`; `views/download.ts::fastSaveRow`.

- stdout and stderr are independent. A started postprocessor marks Processing, then buffered progress sees `is_active()` and changes it to Downloading. That can re-enable Pause during processing and restore stale transfer speed/ETA. Protect the stage and its display fields, not just one enum assignment.
- Directory traversal and the watch branch already use blocking workers. Initial known-size admission and processing recheck still call the synchronous free-space provider while an async task holds the ledger mutex. Slow disks/SMB can stall runtime threads and all ledger contenders. Preserve atomic sampling/reservation decisions while moving blocking work off async workers.
- Audio sizing returns `source + output` even if a later stage writes another output-sized copy. For an expanded WAV/FLAC, `2 * output` can exceed that estimate. Thumbnail embedding is separately missing from the rewrite decision in admission and preview.
- MP3/M4A add `--embed-thumbnail` regardless of Fast Save. The hint can say nothing rewrites the file. MP3 thumbnail processing uses an ffmpeg temp output; M4A handling can differ by available backend, so do not describe every M4A operation as a guaranteed full rewrite. [yt-dlp thumbnail implementation](https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/yt_dlp/postprocessor/embedthumbnail.py).
- `peak_bytes`, `RunPlan::output` and estimate aggregation use unchecked `u64` sums/addition/multiplication. Extreme site metadata can wrap to a small reservation in release or panic in debug. Reservation totals and `held()` also need bounded arithmetic.

**Audio formula caution:** yt-dlp normally deletes a postprocessor's obsolete input before the next processor. With source S and converted output O, a measured sequential pipeline may peak at `max(S + O, 2 * O)`, not automatically `S + 2 * O`. Metadata temp-file behavior is visible in upstream source, but actual output expansion and pipeline order still need local media fixtures. Until validated, use a conservative upper bound with explicit assumptions rather than promising an exact peak. [yt-dlp pipeline](https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/yt_dlp/YoutubeDL.py), [metadata processing](https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/yt_dlp/postprocessor/ffmpeg.py).

### 3.5 Subtitle/caption correctness — R15, R16, R23; M4 rejected

**Current code:** `DownloadOptions::validate`; `probe.rs::collect_subtitles`, `collect_captions`; `download.ts::currentOptions`, `shape`, `adoptRefresh`, subtitle picker.

- Fourteen subtitle chips can be selected; validation silently truncates to twelve. Return a clear limit error and constrain the UI with the same advertised limit. Do not silently change a requested output.
- `collect_captions` excludes `live_chat` when building tracks but includes it in `map.len()` for the twelve-track threshold. Twelve usable tracks plus chat become unavailable.
- Refresh comparison includes subtitle codes, not kind/name. A code retaining its identity but changing kind or label does not redraw. Caption keys also do not capture display-name changes. Preserve selection only when it still represents the intended available track.
- **M4 is not a supported current bug:** upstream inserts normal subtitles first and fills automatic languages only when their code is absent. The current video picker likewise hides automatic duplicates of human languages. Human Spanish + automatic English does not additionally select automatic Spanish. A per-track schema/queue migration is therefore unnecessary to fix the alleged example. Test the precedence contract and surface a refresh-induced availability change; reconsider richer selection only if the product later exposes choosing both kinds for one code. [yt-dlp `process_subtitles`](https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/yt_dlp/YoutubeDL.py#L2914-L2922).

### 3.6 UI state, feedback and accessibility — R04, R06, R08–R10, R21

- **Toasts:** `main.ts` guards completion with a Set, but not Failed. Store rejects repeated revisions, yet a newer no-op Failed update can still toast. Track status transitions without changing existing route suppression, initial-bootstrap behavior or Show action. Remove tracking when jobs leave the store.
- **Confirmation:** `confirmSheet` leaves buttons active during its 160 ms closing animation and permits repeated close calls. Cancel followed by an action can change `result` after dismissal began. The first decision must win, including reduced-motion/hidden-window paths and animation cancellation.
- **Navigation:** single-link preview/options live in the Download view closure and are discarded on unmount. Estimate runs are reset; batch rows survive but completed estimates do not. Retain bounded plain-data sessions keyed by source, selection and options; never revive stale callbacks or obsolete estimates.
- **Radio chips:** role=radio is present without roving focus or arrow behavior. Match existing segmented-control behavior and ensure exactly one change callback per activation.
- **Library cache:** the key omits uploader/duration/mode as well as quality/path/date. Changes can leave text, fallback artwork, duration or conditional drag actions stale. Use a structured rendering signature to avoid delimiter collisions and unnecessary progress-driven rebuilds.
- **Estimates:** `estimate.rs` retains errors as `unknownReason`, and `describeSingle` displays it for unknown sizes. The missing distinction is machine-readable failure versus genuinely unavailable size; aggregate results lose that distinction and give no targeted retry. Never turn unknown sizes into zero or make estimate failure block Download.

### 3.7 Drag hardening and optional cleanup — R07, R20, C01–C03

- `shell_location` handles ASCII COM/LPT digits but not superscript ¹/²/³. Extend the existing component check. Fallible parsing already limits the crash risk; classify this as defensive name handling, not a proven application crash. [Microsoft reserved names](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file#naming-conventions).
- The proposed self-drop NONE scenario is unconfirmed with pinned Wry 0.57.1: `src/webview2/drag_drop.rs` chooses COPY for valid CF_HDROP in DragEnter, preserves it in DragOver and does not overwrite it in Drop. If the actual native checklist shows missing guidance, then separate release status and receiver effect and add a regression. Do not replace current working behavior solely to satisfy a synthetic impossible-state test.
- `core:window:allow-destroy` is unused by current frontend close/hide calls. Remove that specific grant, preserving required capabilities.
- Direct HTTPS thumbnails still contact the remote host. Existing `referrerpolicy="no-referrer"` limits referrer exposure, not IP exposure. A bounded cache reduces repeat contacts; an opt-out avoids them. Routing through this machine's backend alone does not provide anonymity. Broad `img-src https:` supports many sources; narrowing it without a replacement path would break thumbnails.
- `.gitattributes` can document line-ending rules but is optional. Keep binary artifacts explicitly binary and avoid renormalizing unrelated files in a correctness patch.

## 4. Corrections required before using the old plan

The old plan is useful input, but its patches must not be applied verbatim:

1. Task 2 calls `crate::store::rename_retrying` without changing its private visibility or adding `store.rs` to the edit list. Make it `pub(crate)` or introduce a narrow public wrapper in that module, with tests. It also references a nonexistent `schema_1_job`; use existing legacy fixtures or define the helper explicitly.
2. Task 7 uses nonexistent `disk_budget::workspace_bytes`, `Ledger::observe`, Budget fields `.id`/`.last`, and a `recheck_processing` API. Current code uses `bytes_in`, `update`, `admit`, `recheck`, and different async signatures. Its replacement neither matches current code nor fully removes blocking I/O from async workers.
3. Sampling free space outside the lock is not equivalent to atomic admission. Current ledger accounting explicitly depends on ordering the sample with reservations and on-disk updates. Keep that invariant or introduce a tested version/retry protocol; do not quietly remove it.
4. Task 5's `VersionedJob` has only a status `string`; it is not the application's JobStatus union. Its new helper test is in the queue suite, but the task runs `test:logic`. Its snippet also removes completion route suppression and changes the action text/behavior unnecessarily. Preserve behavior while fixing deduplication.
5. F6 is already satisfied by `itemsFor`. Remove the implementation task, retaining a focused metadata assertion only if existing tests do not protect it.
6. F4's high-severity classification lacks a current receiver reproduction. Put it in native acceptance/conditional hardening, not the critical fix list.
7. F10's preview patch omits completed estimates and can save stale preview state while a batch replaces it. Define replacement/invalidation rules before coding.
8. F12's suggested key still omits rendered fields. Use the complete signature in W10.
9. `npm test` does not exist in package.json. Use the explicit four Node suites, Python font check and production build in section 7.
10. A compile failure caused by an undefined helper is not a behavioral reproduction. Add tests that fail because current behavior violates an assertion, then implement the fix. Run focused tests before full integration checks.
11. Do not copy the attachment's new per-track subtitle schema suggestion: M4's stated failure is contradicted by the pinned upstream selection logic.
12. Neither review is a comprehensive security certification, native acceptance result or proof that low Internet download speed has been solved.

## 5. Prioritized implementation work packages

Dependencies and order:

```text
P1 data/availability: W1 queue ── W2 tools ── W3 file/history safety
Priority speed work: W5 stage correctness ── W6 disk I/O ── W7 processing policy ── W0 measurement
P2 recovery/features: W3 ── W4 kept files; W8 captions/subtitles
P3 UX: W9 typed estimates ── W10 session/feedback/accessibility
Hardening/optional: W11 native acceptance; W12 permissions/privacy/hygiene
Final: section 7 verification and freshly built installer acceptance
```

W0 can collect evidence while safety work proceeds. A shipping acceleration change waits for the evidence and safety regressions. Each package should be a focused commit or reviewable series after it passes its checks; this plan does not authorize making those commits now.

### W1 — Make queue recovery conservative and idempotent

**Covers:** R02, R03, R24. **Priority:** P1 preservation, then P2 recovery.

**Modify:** `src-tauri/src/queue_writer.rs`, `src-tauri/src/store.rs`. **Test:** their existing test modules and `src-tauri/src/lifecycle_tests.rs`.

- [ ] Add behavioral regressions before changing the loader: invalid legacy array with a pre-existing backup; valid legacy array + saved schema-3 queue; older nonempty + newer empty backup; multiple supported backups; unreadable candidate; failed recovery save; transient and persistent rename locks; crash after save/before consume; clearing after recovery and another downgrade.
- [ ] Preserve exact current bytes using a unique corrupt backup before returning an empty writable queue. Treat successful legacy-array parsing like other older formats for recovery, with a one-time migration backup.
- [ ] Enumerate matching backup files separately from parsing them. Include empty supported snapshots. If there are multiple plausible candidates or an unreadable/invalid candidate makes authority uncertain, keep all files and report manual recovery rather than guessing by directory iteration or modification time. Unsupported schemas must never be executed.
- [ ] Add durable consumption evidence. A small queue-recovery journal should record the selected backup fingerprint and merged snapshot before publication; after restart, finalize that transaction before permitting a new queue mutation. Completed journal state must distinguish an already-consumed backup even if the physical rename failed. All journal writes use the existing serialized/atomic store path. On an unresolved journal error, preserve data and stop automatic recovery; do not drop the journal and continue as if recovery succeeded.
- [ ] Expose a narrow `pub(crate)` retrying rename from `store.rs`, use it for set-aside/consume, and retain clear errors after the bounded retry limit. A retry is an optimization within the transaction, not its correctness mechanism.
- [ ] Keep the exact schema-3 queue semantics, Original options and older-version additions. Do not silently downgrade options or claim 1.0.5 can read this schema.

Behavioral regression to add inside the existing `queue_writer.rs` tests (uses the existing `temp`, `schema_2_job` and test `load` helpers):

```rust
#[test]
fn empty_newer_backup_does_not_make_an_old_nonempty_queue_authoritative() {
    let dir = temp("recovery-empty-candidate");
    let path = dir.join("queue.json");
    std::fs::write(&path, r#"{"schemaVersion":2,"revision":1,"jobs":[]}"#).unwrap();
    let old = dir.join("queue.newer-version-20260101.json");
    let empty = dir.join("queue.newer-version-20261108.json");
    let job = schema_2_job("obsolete", "paused", "null");
    std::fs::write(&old, format!(r#"{{"schemaVersion":3,"revision":1,"jobs":[{job}]}}"#)).unwrap();
    std::fs::write(&empty, r#"{"schemaVersion":3,"revision":2,"jobs":[]}"#).unwrap();
    let mut notices = Vec::new();
    assert!(load(&path, &mut notices).is_empty());
    assert!(old.exists() && empty.exists());
    assert!(!notices.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}
```

**Run:** `cargo test --locked --manifest-path src-tauri/Cargo.toml queue_writer`; then the full lifecycle suite. **Exit:** no failing path overwrites the only copy; no consumed backup can resurrect jobs; ambiguity remains visible and recoverable.

### W2 — Bound tool network operations and shorten engine exclusion

**Covers:** R11. **Priority:** P1. **Modify:** `src-tauri/src/tools.rs`, `state.rs`; test local HTTP fixtures in the tools tests, following `download_tests.rs` server conventions.

- [ ] Add a local server that accepts headers then stalls, stalls between chunks, streams past a cap with no Content-Length, declares an oversized length, returns oversized checksum/API JSON, and supplies a bad checksum. Inject short test deadlines rather than waiting production intervals.
- [ ] Set explicit production policies: 30-second idle/header deadline; 15-minute total artifact deadline; compressed caps of 256 MiB for yt-dlp and 512 MiB for ffmpeg; checksum text cap 1 MiB; release JSON cap 2 MiB. Constants live beside tool URLs and tests assert both declared-length and streaming enforcement. Explain cap failures without logging signed URLs or cookies.
- [ ] Check the byte counter with `checked_add` before writing every chunk. Enforce total timeout around the complete operation, not only `.send()`. Apply bounded readers/deadlines to checksum and version endpoints as well.
- [ ] Add `tool_install: tokio::sync::Mutex<()>` to AppState and initialize it. A second install fails promptly; staging uses a unique attempt directory. Existing tool readers can continue while an archive is downloaded, hashed and safely extracted there.
- [ ] Acquire the engine writer with the existing fail-fast policy only for final live-tool recheck/swap/rollback. Hash verification needs no engine lock because it touches the staged archive. If readers are active at commit time, keep or discard the staged attempt with a clear retry message; never replace a running executable or wait forever.
- [ ] Clean owned `.part`/staging artifacts on timeout, hash failure and abort. Preserve the known-good installation. Test concurrent installs, a reader arriving during staging, busy final swap, and rollback at each replacement boundary.

**Run:** `cargo test --locked --manifest-path src-tauri/Cargo.toml tools::tests`; new lifecycle tests filtered with `tool_install`. **Exit:** stalled network cannot monopolize the engine; all network bodies are bounded; existing tool replacement/recovery tests still pass.

### W3 — Bind file ownership and make Library mutation durable

**Covers:** R17, R19, R22. **Priority:** P1. **Modify:** `download_files.rs`, `history.rs`, `commands.rs`, `lib.rs` recovery setup, `src/api.ts`, `src/views/library.ts`. Add `src-tauri/src/history_mutation.rs` only for the durable mutation journal/recovery; register the module in lib.rs. Test existing download-file/lifecycle modules plus the new journal module.

- [ ] Add fixtures with a valid folder/job/attempt but altered `manifest.workspace`, `destination_root`, output relative path and recorded move destination. Assert that no sentinel file outside the owned roots is moved, overwritten or deleted.
- [ ] Derive expected workspace/output roots from the verified reference and frozen execution state. Canonicalize existing roots on blocking workers; compare real paths with appropriate Windows semantics, reject links/junction redirects, and reject unsupported schemas. Do not lowercase arbitrary Unicode strings as a substitute for filesystem identity. All manifest-writing/publication callers must receive a verified manifest/root binding; validate move destinations against those roots before trusting the journal.
- [ ] Extend HistoryItem compatibly with optional saved media identity and a recorded published-sidecar list; use `#[serde(default)]` and nullable/optional API fields. Record identity before retiring the publication journal. Old entries stay readable and are never retroactively declared owned merely because a path exists.
- [ ] Implement deletion through an opened, verified file handle. On Windows compare volume/file ID, then delete that same opened object through the handle. For fallback content identity, hash off the async/UI thread while holding the object open. If a platform/filesystem cannot guarantee deleting the verified object, refuse automatic disk deletion and offer Show in folder. A compare-then-`remove_file(path)` sequence is not sufficient.
- [ ] For legacy entries without ownership evidence, allow Library-only removal and Show in folder; explain why automatic disk deletion is unavailable. Sidecars can be deleted only when explicitly recorded and individually identity-verified; no stem or extension glob.
- [ ] Serialize mutations with completion recording using the existing global store lock ordering (store lock before history lock). Build a proposed snapshot, persist it, then commit memory and emit history-changed. Schedule thumbnail cleanup only after success. Never hold the history mutex while waiting for the store write lock.
- [ ] For Delete File Too, journal intent and ownership evidence durably before deleting. After deletion, persist history removal and finalize the journal. Restart reconciles deleted/missing media idempotently; a save failure must not leave a silent mismatch. Concurrent completion must not be lost by replacing history from a stale clone.
- [ ] Cover same-name/same-size replacement, a race between path verification and deletion, missing file, locked file, save failure at every stage, concurrent record/clear, and restart after verified deletion but before history persistence. Keep existing Library-save-failure download recovery tests passing.

Proposed persisted ownership contract (add to HistoryItem; existing FileIdentity and WorkspaceRef types are public):

```rust
#[serde(default)]
pub file_identity: Option<crate::download_files::FileIdentity>,
#[serde(default)]
pub published_sidecars: Vec<crate::download_files::Move>,
```

Only validated Done moves belong in `published_sidecars`; a marker's arbitrary move list is not sufficient evidence. The mutation journal must carry its own schema, operation ID, phase and expected identities; startup ignores no unresolved destructive transaction.

**Run:** `cargo test --locked --manifest-path src-tauri/Cargo.toml download_files`; `cargo test --locked --manifest-path src-tauri/Cargo.toml history`; full lifecycle tests. **Exit:** a path replacement cannot be deleted as the old download; save failures cannot silently mutate the Library; publication cannot escape verified roots.

### W0 — Diagnose the affected source before choosing acceleration

**Priority:** main performance investigation after P1 safety/availability fixes; can gather evidence earlier. **Files:** `scripts/dev/download-bench.mjs`, `scripts/dev/download-fixtures.py`, `docs/performance/speed-diagnosis.md` and new dated results under `docs/performance/`.

- [ ] Obtain one actually affected URL/site and chosen media settings. Current evidence lacks that source. Record versions, connection type and whether the delay is lookup, transfer, conversion, metadata or final publication. Redact cookies, signed URLs and private titles from retained logs.
- [ ] Compare Claws and standalone yt-dlp with the same format IDs, output policy, cookie source and destination volume. Use one warm-up and three timed runs, alternate order, and separate per-stage time, throughput, CPU and disk reads/writes. Verify duration/stream properties and a content hash when outputs are byte-comparable.
- [ ] First measure W5–W7 changes on local fixtures and the affected source. Less UI work or fewer postprocessing passes must not be reported as higher network throughput.
- [ ] Only if progressive HTTP is connection-capped, trial optional range concurrency/aria2c outside the default path. Measure Range support, native fallback, truthful progress, pause/resume, retries, cancel cleanup, disk allocation, tool integrity/footprint and source-wide throttling. Reuse owned workspaces and publication journals; do not bypass them with a second download engine.
- [ ] If the limit is service-wide throttling, authentication/extractor delay, or local encoding, document that cause and apply the corresponding bounded fix. Do not globally increase fragments or promise a gain for every site.

**Exit:** measured cause and verified mitigation, or an explicit evidence-blocked record naming the missing source. Missing evidence is not permission to call speed fixed. Existing localhost range benchmarks remain reference experiments, not shipped acceleration or real-site throughput.

### W5 — Preserve authoritative processing state across pipe ordering

**Covers:** R01. **Priority:** P2, early because it affects controls and speed interpretation. **Modify:** `downloader.rs`; extend `src-tauri/tools/claws-fixture-tool.rs`, `lifecycle_tests.rs`.

- [ ] Add a fixture scenario that emits Processing on stderr, then delayed progress/finished lines on stdout while the process remains alive. Assert through Job events that state never regresses, Pause remains unavailable, and speed/ETA remain absent. Also test late events after pause/cancel and the normal queued-to-downloading transition.
- [ ] Keep raw transfer accounting if useful, but do not overwrite processing UI telemetry from late transfer snapshots. Make the final-processing signal authoritative until process exit; if a pre-download postprocessor can fire the same signal, identify that stage separately rather than treating every processor as final.
- [ ] Protect the event-application path, not only an extracted enum helper. A minimal closure guard for late transfer telemetry is:

```rust
if !matches!(j.status, JobStatus::Queued | JobStatus::Downloading) {
    return;
}
j.status = JobStatus::Downloading;
j.progress_known = snapshot.fraction.is_some();
j.progress = snapshot.fraction.unwrap_or(j.progress);
j.progress_estimated = snapshot.estimated;
j.downloaded_bytes = Some(snapshot.downloaded);
j.total_bytes = snapshot.total;
j.speed = snapshot.speed;
j.eta = snapshot.eta;
```

- [ ] Verify stage handling with video merge, audio extraction, subtitles, metadata and transcript export. Do not use the progress bar as evidence that a file is already durably published.

**Run:** rebuild fixture tool, then `cargo test --locked --manifest-path src-tauri/Cargo.toml lifecycle_tests`. **Exit:** deliberate cross-pipe reordering cannot restore transfer controls or telemetry during final processing.

### W6 — Offload blocking disk checks without weakening reservations

**Covers:** R05. **Priority:** P2. **Modify:** `downloader.rs`, `disk_budget.rs` if a sampling helper is extracted; tests in `lifecycle_tests.rs` and disk-budget tests.

- [ ] Use the existing injectable `free_space` provider to block one sample on a test barrier. Assert an unrelated async heartbeat and commands not needing the ledger continue. Release the barrier; verify two admissions cannot reserve the same bytes.
- [ ] Move known-size admission's `free_space + ledger.admit` critical section into `spawn_blocking`, using owned copies of ID, workspace and volume. Keep sample + decision serialized under the ledger lock. Use the real APIs `bytes_in`, `admit`, `update`, `recheck`.
- [ ] Move processing `ledger.update + free_space + ledger.recheck` into a blocking closure with the same ordering guarantee. Audit every other ledger mutex contender, including ticker/release paths, so an async worker cannot synchronously wait behind a long disk call. Offload those accesses or introduce an async acquisition layer consistently; do not fix only the caller doing I/O.
- [ ] Retain PID/state rechecks after a slow sample so a completed job is not failed or an obsolete/reused PID killed. Keep the unknown-size watch's existing blocking behavior.
- [ ] Test slow SMB-like providers, unreadable volume, cancellation while sampling, completion before sample return, and same-/different-volume reservations. Do not hold a mutex guard across `.await`.

**Run:** `cargo test --locked --manifest-path src-tauri/Cargo.toml disk_budget`; new lifecycle tests filtered with `space`; then Clippy. **Exit:** disk I/O does not execute on async workers and atomic admission remains proven. A global ledger can still serialize reservations during a slow sample; document that remaining bound rather than claiming per-volume isolation.

### W7 — Unify processing policy, Fast Save and overflow-safe disk estimates

**Covers:** R13, R14, R18. **Priority:** P2, early performance work. **Modify:** `download_policy.rs`, `downloader.rs::Budget`, `disk_budget.rs`, `estimate.rs`, `src/views/download.ts`, `src/download-estimates.ts` only if the public estimate shape changes. Add `src/processing-options.ts` for pure hint decisions only if needed by tests; add it to the exact `test:logic` compile inputs.

- [ ] Add an audio matrix: MP3/M4A/Opus/FLAC/WAV, metadata on/off, Fast Save on/off, matching-source copy versus conversion, thumbnail present/missing, and output larger/smaller than source. Test emitted arguments and preview/admission agreement.
- [ ] Define one Rust processing-policy helper used by command construction and disk sizing: conversion, metadata embedding, subtitle embedding, thumbnail embedding, possible output rewrite. Adopt the clear contract that Fast Save skips optional metadata, embedded subtitles and embedded cover art; necessary conversion/merge/container repair can remain. Reflect this exact contract in the UI and documentation, including the cover-art tradeoff.
- [ ] Inspect actual temporary-file lifetimes with synthetic source media and real yt-dlp/ffmpeg, sampled on a disposable volume. Capture the chosen M4A backend. Use `max(S + O, 2 * O)` only when source deletion before rewrite is verified; otherwise reserve a conservative upper bound and describe it as an estimate. Do not invent exact peak usage or exact WAV/FLAC expansion from a fixed rate assumption alone.
- [ ] Replace unchecked sums with an explicit checked helper, used by preview and admission:

```rust
fn known_total(streams: &[Option<u64>]) -> Option<u64> {
    streams.iter().try_fold(0u64, |sum, size| sum.checked_add((*size)?))
}
```

Use `checked_add`/`checked_mul` for peak formulas and return unknown on overflow. Use saturating arithmetic for reservation totals/margins where overflow must conservatively mean no available capacity. Reject nonfinite duration inputs; zero/unknown sizes never become exact zero-byte reservations.

- [ ] Add direct regressions against the current public sizing functions:

```rust
#[test]
fn expanded_audio_requires_space_for_its_rewrite() {
    let steps = Steps { merge: false, rewrite: true, audio_output: Some(1000) };
    assert!(peak_bytes(&[Some(100)], steps).unwrap() >= 2000);
}

#[test]
fn overflowing_sizes_stay_unknown() {
    assert_eq!(peak_bytes(&[Some(u64::MAX), Some(1)], Steps::default()), None);
    assert_eq!(peak_bytes(&[Some(u64::MAX)], Steps { rewrite: true, ..Steps::default() }), None);
}
```

- [ ] Retain source-stream identity/no-reencode guarantees. Fast Save's fewer optional passes do not mean audio conversion itself is a copy. Record processing-time/disk-I/O measurements separately from network throughput.

**Run:** Rust filters `disk_budget`, `download_policy`, `estimate`; `npm run test:logic`; real synthetic-media lifecycle fixtures. **Exit:** policy arguments, hints, preview and admission agree; expanded audio has conservative rewrite space; extreme metadata cannot wrap or panic.

### W4 — Keep withheld sidecars durably discoverable

**Covers:** R12. **Priority:** P2. **Depends on:** W3 ownership/durable history. **Modify:** `download_files.rs`, completion and startup paths in `downloader.rs`, `history.rs`, `commands.rs`, `lib.rs` command registration, `src/api.ts`, `src/views/library.ts` and completed Queue actions.

- [ ] Add a lifecycle case with a conflicting subtitle path, successful media publication, restart, Clear Finished and Library reload. Existing destination bytes and retained subtitle bytes must both survive and the retained file must remain reachable through an action.
- [ ] Extend history with `#[serde(default)]` kept-file records containing a WorkspaceRef and plain relative sources derived from Withheld moves. Persist those records before retiring completion recovery. Expose an ID-based Show kept files command that verifies W3 root bindings and reveals the actual source folder, not the occupied destination.
- [ ] Render a persistent kept-files indicator and Show kept files action in Library and completed Queue rows. Distinguish a missing/offline kept folder from a successful reveal; a toast alone is not the durable record.
- [ ] Add a bounded background repair for pre-fix orphans in known output roots only: inspect owned manifests matching existing job/history IDs, validate ownership and attach withheld records. Do not scan entire disks, auto-delete unknown workspaces or adopt mismatched markers. Record inaccessible roots and allow a retry.
- [ ] Define removal explicitly: clearing Queue never removes these records; Library-only removal does not delete retained files. Destructive removal follows W3 identity/journal rules and must warn about retained sidecars. Do not silently delete them merely because the main media is Completed.

**Run:** lifecycle tests filtered with `withheld`; Library action tests and production build. **Exit:** sidecar conflicts survive restart with a discoverable, verified source location and no overwrite of user files.

### W8 — Enforce subtitle limits and refresh correct track information

**Covers:** R15, R16, R23; excludes M4. **Priority:** P2, with small P3 caption boundary fix. **Modify:** `downloader.rs::DownloadOptions::validate`, `probe.rs`, `src/views/download.ts`, optionally `src/transcript-options.ts`; extend existing logic tests and Rust fixture tests.

- [ ] Add boundary tests for 12/13/14 requested subtitle languages and invalid language syntax beyond position 12. Validate all input, then reject over-limit requests; never truncate before validation. Picker prevents exceeding the same displayed limit and explains it. Export a Rust constant and a frontend constant with a contract test or bootstrap capability value to prevent future drift.
- [ ] Count only usable automatic tracks for the 12-track fallback threshold. Test 12 tracks + live_chat, 13 real tracks, and a `-orig` track. Preserve the existing exclusion of machine-translated long lists.
- [ ] Include code, auto flag and display name in subtitle refresh signatures; include caption display names and playlist metadata used by rendering in their corresponding signatures. Decide selection retention by actual available identity. Show a changed-availability notice if the selected kind can no longer be honored.
- [ ] Add an offline yt-dlp fixture with human Spanish, automatic Spanish and automatic English. Assert human Spanish wins and only one Spanish track is produced alongside automatic English. This protects the valid mixed case without a new per-track persistence schema.
- [ ] Test refresh name-only/kind-only changes without losing unrelated quality/audio/transcript choices. Keep transcript's exactly-one-language validation separate from video's multi-language limit.

**Run:** Rust filters `probe`, `download_policy`, `downloader::tests`; `npm run test:logic`; offline subtitle fixture. **Exit:** no silent dropped selection, correct transcript threshold, labels reflect the adopted result, and mixed-language behavior matches upstream precedence.

### W9 — Distinguish estimate failures from legitimately unknown sizes

**Covers:** R21. **Priority:** P3. **Modify:** `estimate.rs`, `src/download-estimates.ts`, `src/views/download.ts`; existing estimate Rust/Node tests.

- [ ] Add serialized, backward-compatible estimate state distinguishing available, unknown and failed, plus a safe diagnostic for failed. Keep `unknownReason` readable for older-shaped responses; a source with no size is unknown, while tool launch/timeout/parse failure is failed.
- [ ] Show a targeted Retry for failures. Aggregate results count failed separately from missing-size items and retry failed items without resetting valid estimates. Use the existing shared CallLimit and selection/options keys; no unbounded automatic retry loop.
- [ ] Test mixed exact/approximate/unknown/failed totals, a missing tool, malformed selection, timeout, retry success, and a late reply after options changed. Unknown and failed sizes remain null and never add zero to a purportedly complete total.

**Run:** `cargo test --locked --manifest-path src-tauri/Cargo.toml estimate`; `npm run test:logic`. **Exit:** users can distinguish/retry a broken estimate while Download remains available.

### W10 — Fix confirmation, feedback, session state and radio behavior

**Covers:** R04, R06, R08, R09, R10. **Priority:** P2 confirmation, P3 remainder. **Modify:** `src/ui/components.ts`, `src/main.ts`, `src/store.ts`, `src/views/download.ts`, `src/views/library.ts`, `src/queue-events.ts` only if it owns the transition helper. Add DOM-facing tests using the repository's existing fake-DOM patterns; do not assert source strings as a substitute for behavior.

- [ ] Confirmation first: add an explicit closing flag, capture the first decision before animation starts, disable/ignore subsequent actions and close requests, and make reduced-motion/hidden/cancelled-animation paths settle once. Test Cancel then Delete, Delete then Cancel, repeated Escape, hidden window and animation cancellation. A cancelled dismissal must never become destructive.
- [ ] Add a status-transition tracker seeded from bootstrap. Ignore same-status newer events, notify a new Failed after a real retry, prune removed IDs and keep existing Queue-route suppression and Show action. Test with the queue suite, not `test:logic` if the helper lives in `queue-events.ts`.
- [ ] Persist a bounded plain-data preview session and completed estimate cache in Store. Key estimates by URL, playlist position, selected formats/options and source refresh generation. Preserve choice/filter state on navigation; invalidate on a replacement link/batch, accepted enqueue or incompatible refresh. In-flight view callbacks are cancelled/ignored and never serialized. Retain only the existing 200-item estimate cap per active session.
- [ ] Give `chipRadio` one Tab stop and ArrowLeft/Right/Up/Down navigation with wrap; Home/End can follow the existing control convention. Focus and checked state move together, and onChange fires once. Test mouse, keyboard, unknown initial value and an empty options list without division by zero.
- [ ] Replace the Library string-concatenation key with a structured signature of all values consumed by the tile or captured handlers. The current rendering inputs are:

```ts
const key = JSON.stringify([
  item.title, item.thumbnailLocal, item.thumbnail,
  item.fileSize, item.filePath, item.quality, item.finishedAt,
  item.uploader, item.duration, item.mode,
]);
```

After W3/W4 add the identity/kept-file fields that determine action availability. Verify a changed field rebuilds once, unchanged items preserve DOM/focus, and removed items leave no cache entry.

- [ ] Run browser-level flows: probe a link, choose options, navigate to Queue/Settings and back; replace with a batch; estimate, navigate and return; refresh a label; rapidly cancel a confirmation. No owner data is needed.

**Run:** the exact suites owning the added tests (`test:queue`, `test:logic`, `test:motion`), then `npm run build`. **Exit:** first confirmation decision wins; notices are transition-based; navigation retains valid state; radio keyboard behavior and tile invalidation are correct.

### W11 — Complete native drag hardening and acceptance

**Covers:** R07 and conditional C01. **Priority:** P3, but native acceptance is a release gate. **Modify when justified:** `file_drag.rs` and its tests; current status/manual checklist.

- [ ] Extend reserved device components to COM/LPT followed by one of ASCII 1–9 or superscript ¹/²/³, case-insensitive and before an extension. Test reserved and nonreserved neighbors such as COM0, COM10 and COM¹-title. Preserve long/UNC normalization and fallible Shell behavior.
- [ ] Run actual Explorer copy, one editor import, self-drop, Escape, quick release, long/UNC source, vanished source, swapped primary buttons and a deliberately stalled receiver with disposable media. Record copy/import separately from reported drop outcome and verify source identities remain intact.
- [ ] If a self-drop genuinely returns DROP + NONE, return a typed native result with separate `released` and `copy_effect` fields, read the posted self-drop marker before classification, and classify released-on-self before requiring COPY. Preserve Cancelled and NotStarted behavior. Add native evidence before labelling this a fixed current bug.
- [ ] Verify a stuck receiver cannot create unlimited worker threads; report that a receiver blocked inside COM can retain the single worker until it returns or Claws restarts. Do not terminate an OLE thread forcibly. The prior automation tool's cross-window restriction is a validation limitation, not an application defect.

**Run:** `cargo test --locked --manifest-path src-tauri/Cargo.toml file_drag`; the manual checklist above. **Exit:** no unsupported native acceptance claim; hardening cannot reintroduce panic-based path handling.

### W12 — Small capabilities, privacy choice and optional line-ending policy

**Covers:** R20, C02, C03. **Priority:** P3; privacy/hygiene are optional improvements, not blockers for unrelated bug fixes.

**Files:** `src-tauri/capabilities/default.json`; for privacy, `history.rs`, a focused thumbnail cache helper, `probe.rs`, `src/api.ts`, `src/views/download.ts`, `queue.ts`, `library.ts`, `settings.ts`, `src-tauri/tauri.conf.json`; optional root `.gitattributes`.

- [ ] Remove only `core:window:allow-destroy`; verify Close, hide-to-tray, minimize, zoom, Quick Open and Quit with current settings. Keep required close/hide permissions.
- [ ] For privacy, choose an explicit remote-artwork preference, defaulting to existing behavior to avoid a silent functional change. Disabled means neither frontend nor backend fetches remote images. Enabled may use a bounded shared cache with deduplicated requests, current 5 MiB body/20-second deadline protections and safe redirects/content types. Cache any preview path needed before narrowing CSP; never claim the local backend conceals the user's IP.
- [ ] If CSP is narrowed, test first-use and cached images on several supported hostnames, expired URLs, no image, offline mode and the disabled preference. Avoid passing arbitrary filesystem paths to the frontend outside scoped assets.
- [ ] If adding `.gitattributes`, use `* text=auto eol=lf`, explicit CRLF only for scripts that need it, and binary rules for `.exe`, font, archive and raster assets. Do not run a blanket renormalization in the same change. Verify font and installer hashes remain unchanged.

**Run:** production build, font suites, targeted artwork tests and native window smoke. **Exit:** required functionality remains permitted; artwork preference means what it says; hygiene produces no unrelated source/binary churn.

## 6. File ownership map and compatibility rules

| Area | Files / responsibility |
| --- | --- |
| Queue migration | `queue_writer.rs`, `store.rs`: authoritative candidate selection, preservation, durable recovery consumption and bounded rename retries. |
| Tool availability | `tools.rs`, `state.rs`: bounded HTTP, install serialization/staging, short engine exclusion, rollback. |
| Ownership | `download_files.rs`: verified roots, move evidence and safe object identity; no generic path deletion helper. |
| History | `history.rs`, proposed `history_mutation.rs`, `commands.rs`: durable record/remove/clear/delete intent, identity-aware deletion and kept-file actions. |
| Download lifecycle | `downloader.rs`, fixture tool, `lifecycle_tests.rs`: processing-state ordering, safe sampling, completion/kept-file persistence. |
| Sizing/policy | `download_policy.rs`, `disk_budget.rs`, `estimate.rs`: one optional-processing policy and conservative checked arithmetic. |
| Subtitle/caption choices | `probe.rs`, `DownloadOptions::validate`, `src/views/download.ts`, existing options/logic tests. |
| UI | `components.ts`, `main.ts`, `store.ts`, `download-estimates.ts`, `views/library.ts`: confirmation, transitions, session invalidation, typed estimate retries, radio focus and rendering signatures. |
| Native and packaging | `file_drag.rs`, capabilities/config, README/HANDOFF/status/release notes and installer/checksum artifacts after final verification. |

- Preserve older history/queue JSON with serde defaults and legacy-shape fixtures. Do not add a queue schema bump just to represent the rejected M4 scenario.
- New journals need explicit schema handling, atomic writes and startup recovery before related mutations. Do not recycle a publication manifest as an unrelated deletion journal.
- Keep Original/no-reencode guarantees, owned cleanup, destination no-replace moves, missing-subtitle retry tolerance and Library-failure recovery.
- Never use the owner's installed profile, cookies or actual Downloads as test fixtures. Use generated media, temporary roots and a separate native-app identifier.
- Excluded scope remains excluded: macOS, ProRes, vocal removal and clip sections. Speech recognition, replay heatmaps and lyrics are deferred. No incidental feature expansion belongs in these fixes.

## 7. Verification and delivery gates

Run focused behavioral tests while implementing each package. Once integrated, run these exact existing commands from `C:/Users/PC/claws`:

```powershell
npm ci
npm run test:fonts
npm run test:queue
npm run test:logic
npm run test:motion
python scripts/test_font_metrics.py
npm run build
cargo build --locked --manifest-path src-tauri/Cargo.toml --features test-tools --bin claws-fixture-tool
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets --features test-tools -- -D warnings
git diff --check
```

`npm ci` belongs at the integrated clean-checkout check; it was not needed/repeated for this document's baseline. If a work package adds test compile inputs or a new suite, add its exact script and run it explicitly. Existing suite counts will grow; do not preserve old counts in the final handoff without re-reading the output.

- [ ] Behavioral regressions fail on the pre-fix implementation and pass on the resulting implementation. Compilation errors alone are not reproduction evidence.
- [ ] Fault injection covers disk full, persistent/transient file locks, blocked saves, cancellation, restart, conflicting outputs, replaced files and late events.
- [ ] Synthetic media verifies output properties and postprocessing lifetimes. Real-source speed evidence is recorded separately or remains explicitly unavailable.
- [ ] Native drag/receiver tests, confirmation/keyboard flows and installed proportional font rendering are checked in a real Windows window.
- [ ] Update README, HANDOFF and current status with final behavior, fresh counts, measured limits and pending native/real-site boundaries. Remove completed implementation tasks from the active plan; retain original reviews as historical evidence, not competing execution instructions.
- [ ] Synchronize the five version files if the release version changes. Rebuild the installer only after final source changes with `npm run tauri build`; record commit/working-tree provenance, bytes and SHA-256. The current stored installer cannot prove these fixes shipped.
- [ ] Test clean installation and upgrade using disposable profiles, preserving settings, Library and queue. Test supported schema migration and downgrade-return recovery; old 1.0.5 execution of schema 3 remains unsupported.
- [ ] Commit only reviewed files when authorized. Publish only after the user authorizes release delivery, then verify the uploaded asset digest and release target. Do not add collaborators or change repository access.

## 8. Final disposition

This is the consolidated execution reference for the reviewed findings. It replaces the **recommendations** in the 2026-11-08 F1–F13 plan, not the source files or historical evidence. It contains the valid findings from both supplied reviews, the still-open empty-backup defect, explicit exclusions/corrections and their implementation/test order.

No application fixes, source commits, installer rebuilds or releases were performed as part of creating this report. No claim is made that all bugs are exhausted, that Explorer/editor integration passed, or that the affected Internet-download speed has been fixed.

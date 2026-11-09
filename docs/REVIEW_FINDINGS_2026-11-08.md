# Code Review Findings (2026-11-08)

**Date:** 2026-11-08\
**Branch:** `feature-expansion-and-motion` (version 1.1.0 source + uncommitted working-tree changes)\
**Scope:** Full repository review across the Rust backend (`src-tauri/src/`), TypeScript frontend (`src/`), styles (`src/styles/`), and build/verification scripts (`scripts/`).

---

## Summary Table

| ID | Severity | Area | Summary |
| --- | --- | --- | --- |
| **F1** | High (Bug) | `src-tauri/src/downloader.rs` | Buffered `stdout` progress lines can overwrite `JobStatus::Processing` back to `JobStatus::Downloading` |
| **F2** | High (Bug) | `src-tauri/src/queue_writer.rs` | Corrupt schema-1 (1.0.4 array) `queue.json` is not moved aside and valid schema-1 migrations skip `recover_downgrade` |
| **F3** | High (Bug) | `src-tauri/src/queue_writer.rs` | `recover_downgrade` and `set_aside` use bare `std::fs::rename` instead of `store::rename_retrying`, risking stranded `queue.newer-version-*.json` files |
| **F4** | High (Bug) | `src-tauri/src/file_drag.rs` | Self-drop detection in `file_drag::start` checks `DROPEFFECT_COPY` before `window_received_drop`, misclassifying non-copy self-drops as `Cancelled` |
| **F5** | Medium (Bug) | `src/main.ts` | Failed-download toast does not deduplicate on status transitions |
| **F6** | Medium (Bug) | `src/download-input.ts` | Batch playlist enqueue (`remainingItems`) omits `thumbnail`, `uploader`, and `duration` |
| **F7** | Medium (Robustness) | `src-tauri/src/downloader.rs` | Synchronous `GetDiskFreeSpaceExW` runs inside Tokio async tasks while holding `state.disk_ledger` mutex |
| **F8** | Medium (Robustness) | `src/ui/components.ts` | `confirmSheet` close animation can re-enter on repeated `Escape` / `Cancel` presses during the 160 ms exit animation |
| **F9** | Low (Hardening) | `src-tauri/src/file_drag.rs` | `is_reserved_device_name` does not reject ISO-8859-1 superscript digits (`¹`, `²`, `³`) on `COM`/`LPT` |
| **F10** | Low (UX / State) | `src/views/download.ts` | Navigating away from `Download` discards single-link preview state and completed size-estimate runs |
| **F11** | Low (Accessibility) | `src/ui/components.ts` | `chipRadio` uses `role="radiogroup"` / `role="radio"` without roving `tabIndex` or arrow-key navigation |
| **F12** | Low (Polish) | `src/views/library.ts` | `tileFor` DOM cache key omits `filePath`, `quality`, and `finishedAt` |
| **F13** | Low (Repo Hygiene) | `.gitattributes` | Missing `.gitattributes` causes `LF will be replaced by CRLF` warnings on `src-tauri/Cargo.toml` and `src-tauri/Cargo.lock` |

---

## 1. Bugs & Correctness Issues

### F1 — Buffered `stdout` progress lines can regress job status from `Processing` back to `Downloading`

* **Location:** `src-tauri/src/downloader.rs` (`handle_line` at lines 1501–1546, `mark_processing` at lines 1432–1443, `JobStatus::is_active` at lines 27–29, and `run_prepared` at lines 1168–1203)
* **Problem:** In `run_prepared`, `yt-dlp`'s `stdout` (`CLAWS|download|...` / `CLAWS|finished|...`) and `stderr` (`CLAWSPP|started`) are read concurrently on two independent Tokio tasks. When `ffmpeg` post-processing begins (`Merger`, `ExtractAudio`, `EmbedSubtitle`, `Metadata`), `stderr_task` sees `TAG_POST` and calls `mark_processing`, setting `j.status = JobStatus::Processing`. However, `handle_line` updates the job with:
  ```rust
  update_job(app, id, |j| {
      if j.status.is_active() {
          j.status = JobStatus::Downloading;
      }
      ...
  });
  ```
  Because `JobStatus::is_active()` returns `true` for **both** `JobStatus::Downloading` and `JobStatus::Processing`, any buffered `CLAWS|finished|...` or `CLAWS|download|...` line read from `stdout` after `stderr_task` has already processed `CLAWSPP|started` overwrites `j.status` back to `JobStatus::Downloading`. That re-exposes the `Pause` action in the Queue row and replaces the indeterminate processing progress bar while `ffmpeg` is mid-flight.
* **Fix:** Only set `j.status = JobStatus::Downloading` when `j.status == JobStatus::Queued` (or `matches!(j.status, JobStatus::Queued | JobStatus::Downloading)`), leaving `JobStatus::Processing` untouched, and skip overwriting `progress = 1.0` / `speed = None` / `eta = None` if `j.status == JobStatus::Processing`.

---

### F2 — Corrupt schema-1 (1.0.4 JSON array) `queue.json` is not moved aside and skips downgrade recovery

* **Location:** `src-tauri/src/queue_writer.rs` (`load` at lines 216–222)
* **Problem:**
  ```rust
  if value.is_array() {
      let backup = path.with_file_name("queue.pre-1.0.5.json");
      if !backup.exists() {
          std::fs::copy(path, &backup)?;
      }
      return Ok(serde_json::from_value(value).unwrap_or_else(|e| {
          notices.push(format!("Your saved download queue could not be read ({e}). Starting with an empty queue."));
          Vec::new()
      }));
  }
  ```
  1. If `queue.pre-1.0.5.json` already exists from a previous run and `queue.json` contains a malformed JSON array (for example, an element whose fields fail `Job` deserialization), `load` does **not** call `set_aside(path, "corrupt", notices)?`. Instead it returns `Ok(Vec::new())` while leaving `queue.json` in place, so the next queue save overwrites it.
  2. When `serde_json::from_value(value)` succeeds on a schema-1 array, it returns `Ok(jobs)` directly without calling `recover_downgrade(path, jobs, notices)`. If a user downgraded from 1.1.0 to 1.0.4 (after 1.0.5 or manually) with a preserved `queue.newer-version-*.json`, returning to 1.1.0 will not recover the schema-3 backup.
* **Fix:** On deserialization error in the `value.is_array()` branch, call `set_aside(path, "corrupt", notices)?` and return `Ok(recover_downgrade(path, Vec::new(), notices))`. On success, pass the parsed schema-1 jobs to `Ok(recover_downgrade(path, jobs, notices))`.

---

### F3 — `recover_downgrade` and `set_aside` use bare `std::fs::rename` instead of `store::rename_retrying`

* **Location:** `src-tauri/src/queue_writer.rs` (`recover_downgrade` at lines 276–285, `set_aside` at lines 316–328)
* **Problem:** In `recover_downgrade`:
  ```rust
  if let Err(e) = store::save_blocking(path, &snapshot) {
      notices.push(format!("A saved queue from a newer Claws version could not be restored ({e}). Your file was kept."));
      return current;
  }
  let consumed = backup.with_extension("json.recovered");
  if let Err(e) = std::fs::rename(&backup, &consumed) {
      notices.push(format!("Your saved queue was restored, but its backup could not be marked as used ({e})."));
  }
  ```
  * On Windows, antivirus or search indexers frequently open newly written/read files briefly without `FILE_SHARE_DELETE`. Using raw `std::fs::rename` instead of `store::rename_retrying` makes transient sharing violations fail immediately.
  * Because `store::save_blocking(path, &snapshot)` already wrote `queue.json` at `schemaVersion: 3`, future startups take the `snapshot.schema_version == QUEUE_SCHEMA` path and never consume `backup`. If the user later downgrades again, a second `queue.newer-version-*.json` is created, causing `newer_version_backups` to see multiple backups and refuse automatic recovery.
* **Fix:** Use `store::rename_retrying(&backup, &consumed)` in `recover_downgrade` and `store::rename_retrying(path, &aside)` in `set_aside`.

---

### F4 — Self-drop detection in `file_drag::start` conflates `DRAGDROP_S_DROP` with `DROPEFFECT_COPY`

* **Location:** `src-tauri/src/file_drag.rs` (`outcome` at lines 29–35, `start` at lines 184–195, and `native::drag` at lines 237–257)
* **Problem:** `outcome(dropped, on_claws)` was originally written when `dropped` meant `status == DRAGDROP_S_DROP`. In the current `native::drag` implementation, the single boolean was changed to `status == DRAGDROP_S_DROP && (effect & DROPEFFECT_COPY).0 != 0`:
  ```rust
  let Some(dropped) = ended else { return Ok(DragOutcome::NotStarted) };
  if !dropped {
      return Ok(DragOutcome::Cancelled);
  }
  ```
  If a user drops a file onto Claws' own window (`status == DRAGDROP_S_DROP`, `window_received_drop() == true`) and the drop target reports `DROPEFFECT_NONE` (`copy == false`), `native::drag` returns `Ok(Some(false))` and `start` returns `DragOutcome::Cancelled` before ever checking `window_received_drop()`, suppressing the guidance toast (`"To copy the file, drop it on a folder or an app outside Claws."`).
* **Fix:** Separate `dropped` (`status == DRAGDROP_S_DROP`) from `copy` (`(effect & DROPEFFECT_COPY).0 != 0`) in `outcome(dropped: bool, copy: bool, on_claws: bool)`:
  * `(false, _, _)` → `DragOutcome::Cancelled`
  * `(true, _, true)` → `DragOutcome::DroppedOnSelf`
  * `(true, true, false)` → `DragOutcome::Dropped`
  * `(true, false, false)` → `DragOutcome::Cancelled`

---

### F5 — Failed-download toast in `main.ts` does not deduplicate on status transitions

* **Location:** `src/main.ts` (lines 132–142)
* **Problem:**
  ```ts
  const finished = new Set<string>();
  store.on("job", (detail) => {
    const job = detail as Job;
    if (job.status === "completed" && !finished.has(job.id)) {
      finished.add(job.id);
      ...
    } else if (job.status === "failed" && store.route !== "queue") {
      toast(`Couldn't download “${job.title}”.`, ...);
    }
  });
  ```
  * The `"failed"` branch does not check whether the job just transitioned into `"failed"`. Any subsequent `"job"` event for an already-failed job while `store.route !== "queue"` triggers a duplicate error toast.
  * Tracking transitions via a `Map<string, JobStatus>` (updated on `"job"` and `"jobs"` events) ensures both `"completed"` and `"failed"` toasts fire only once per status transition—and fire again if a failed job is retried and fails a second time.
* **Fix:** Extract a pure helper or track `previousStatus` per job ID so toasts only fire when `previousStatus !== job.status`.

---

### F6 — Batch playlist enqueue (`remainingItems`) omits `thumbnail`, `uploader`, and `duration`

* **Location:** `src/download-input.ts` (`remainingItems` at lines 194–212)
* **Problem:**
  ```ts
  const items: EnqueueItem[] =
    res.kind === "playlist"
      ? res.entries.map((e) => ({
          url: e.url,
          playlistIndex: e.playlistIndex,
          title: e.title || res.title || row.url,
        }))
      : [
          {
            url: res.url || row.url,
            title: res.title || row.url,
            thumbnail: res.thumbnail,
            uploader: res.uploader,
            duration: res.duration,
          },
        ];
  ```
  When a playlist is queued from the single-link preview (`src/views/download.ts:653-660`), each `EnqueueItem` includes `thumbnail: e.thumbnail`, `uploader: e.uploader`, and `duration: e.duration`. When the same playlist is queued from a batch (`remainingItems`), those three fields are omitted, leaving playlist items without thumbnails, uploaders, or durations in the Queue and Library.
* **Fix:** Include `thumbnail: e.thumbnail`, `uploader: e.uploader ?? res.uploader`, and `duration: e.duration` in the `res.kind === "playlist"` branch of `remainingItems`.

---

## 2. Edge Cases & Robustness Improvements

### F7 — Synchronous `GetDiskFreeSpaceExW` inside Tokio tasks while holding `state.disk_ledger` mutex

* **Location:** `src-tauri/src/downloader.rs` (`Budget::after_line` at lines 1415–1426 and `recheck_processing_space` at lines 1452–1486)
* **Problem:** Both functions execute on Tokio async tasks (`run_prepared`'s stdout reader and `stderr_task`) and invoke `(state.free_space)(workspace)` while holding `state.disk_ledger.lock()`. If the output folder resides on a slow SMB share or a spun-down external USB disk, `GetDiskFreeSpaceExW` can block the async worker thread while holding the `disk_ledger` mutex, delaying other downloads and IPC commands (`estimate`, `enqueue`).
* **Fix:** Run `Budget::after_line` and `recheck_processing_space` inside `tokio::task::spawn_blocking` (or sample free space before acquiring the lock when only updating an existing reservation).

---

### F8 — `confirmSheet` close animation can re-enter on rapid `Escape` / `Cancel` presses

* **Location:** `src/ui/components.ts` (`confirmSheet` at lines 295–344)
* **Problem:** `dialog.close` is overridden to run a 160 ms exit animation and invoke the native `close()` in `out.onfinish`. If the user presses `Escape` or clicks an action/cancel button again during that 160 ms window, `dialog.close()` starts a second exit animation from `opacity: 1` and can overwrite `result` after dismissal has begun.
* **Fix:** Guard `dialog.close` and action button handlers with a `let closing = false;` flag so repeat triggers during the exit animation are ignored.

---

### F9 — `is_reserved_device_name` does not check ISO-8859-1 superscript digits (`¹`, `²`, `³`)

* **Location:** `src-tauri/src/file_drag.rs` (`is_reserved_device_name` at lines 140–149) and `src-tauri/src/download_files.rs` (`sanitize_stem` at lines 653–680)
* **Problem:** `is_reserved_device_name` checks `COM1`..`COM9` and `LPT1`..`LPT9`. In Win32 path normalization (used by `SHParseDisplayName` on non-`\\?\` paths), Windows also treats `COM¹`, `COM²`, `COM³` and `LPT¹`, `LPT²`, `LPT³` (`\u{00b9}`, `\u{00b2}`, `\u{00b3}`) as reserved DOS device names.
* **Fix:** Treat `COM` / `LPT` followed by `\u{00b9}`, `\u{00b2}`, or `\u{00b3}` as reserved device names in `is_reserved_device_name`.

---

### F10 — Navigating away from `Download` discards single-link preview state and batch size estimates

* **Location:** `src/views/download.ts` (`mountDownload` at lines 250–302 and cleanup at lines 1524–1543)
* **Problem:** While `store.batch` is kept in `store` across route changes, a single-link preview (`result`, `lastUrl`, and selected options) is stored only in `mountDownload`'s local closure variables. Switching from `Download` to `Queue` or `Settings` and returning resets the view to the empty state.
* **Fix:** Keep the active single-link preview session in `store` (or restore it on mount when no batch is active) and clear it when queued or replaced.

---

## 3. Polish & Maintainability Improvements

### F11 — `chipRadio` lacks roving `tabIndex` and arrow-key navigation

* **Location:** `src/ui/components.ts` (`chipRadio` at lines 123–152 vs. `segmented` at lines 47–114)
* **Problem:** `segmented` implements ARIA `radiogroup` keyboard semantics (`tabIndex = 0` on the checked radio, `-1` on others, plus `ArrowLeft`/`ArrowRight` navigation). `chipRadio` also uses `role="radiogroup"` and `role="radio"`, but leaves every chip in the `Tab` sequence and does not respond to arrow keys.
* **Fix:** Add roving `tabIndex` (`0` for checked, `-1` for unchecked) and `ArrowRight`/`ArrowLeft` navigation to `chipRadio`.

---

### F12 — `library.ts` tile cache key omits `filePath`, `quality`, and `finishedAt`

* **Location:** `src/views/library.ts` (`tileFor` at lines 107–114)
* **Problem:** `const key = `${item.title}|${item.thumbnailLocal ?? item.thumbnail}|${item.fileSize}`;` omits `item.filePath`, `item.quality`, and `item.finishedAt`, which also drive the rendered tile (`fileDragButton(item.id)`, metadata text, and formatted date).
* **Fix:** Include `item.filePath`, `item.quality`, and `item.finishedAt` in `key`.

---

### F13 — Missing `.gitattributes` causes CRLF warnings on `Cargo.toml` and `Cargo.lock`

* **Location:** Repository root (`.gitattributes`)
* **Problem:** `git diff` warns `LF will be replaced by CRLF the next time Git touches it` on `src-tauri/Cargo.toml` and `src-tauri/Cargo.lock`.
* **Fix:** Add a `.gitattributes` file to normalize text line endings consistently across platforms and tools.

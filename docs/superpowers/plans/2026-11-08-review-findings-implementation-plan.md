# Code Review Findings (F1–F13) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve all 13 correctness, robustness, accessibility, and maintainability findings recorded in `docs/REVIEW_FINDINGS_2026-11-08.md` with full unit and lifecycle test coverage.

**Architecture:** Preserve Claws' existing module boundaries (`downloader.rs`, `queue_writer.rs`, `file_drag.rs`, `download-input.ts`, `store.ts`, `ui/components.ts`, `views/download.ts`, `views/library.ts`). Each fix is isolated to its owning module, paired with a regression test in the existing Rust (`cargo test`) or Node (`npm run test:logic`) test suite.

**Tech Stack:** Rust (Tauri 2, Tokio, `serde_json`, `windows` 0.61), TypeScript (Vite, strict `tsc`), Node `node:test`.

---

## File Map

| File | Responsibility in this Plan |
| --- | --- |
| `src-tauri/src/downloader.rs` | Prevent buffered `stdout` progress lines from regressing `JobStatus::Processing` to `JobStatus::Downloading` (**F1**); move periodic disk free-space checks off the async mutex hot path (**F7**). |
| `src-tauri/src/queue_writer.rs` | Set aside corrupt schema-1 array queues, run `recover_downgrade` after schema-1 migration (**F2**), and use `store::rename_retrying` for `set_aside` and `.recovered` renames (**F3**). |
| `src-tauri/src/file_drag.rs` | Separate `DRAGDROP_S_DROP` from `DROPEFFECT_COPY` in `outcome` so self-drops with `DROPEFFECT_NONE` still report `DroppedOnSelf` (**F4**); reject `COM¹`/`COM²`/`COM³` and `LPT¹`/`LPT²`/`LPT³` in `shell_location` (**F9**). |
| `src/download-input.ts` | Include `thumbnail`, `uploader`, and `duration` when building `EnqueueItem`s for playlists in `remainingItems` (**F6**). |
| `scripts/tests/download-input.test.mjs` | Regression test for playlist metadata in `remainingItems` (**F6**). |
| `src/queue-events.ts` | Pure helper `statusToastTransition` for deduplicating `"completed"` and `"failed"` job status toasts across transitions (**F5**). |
| `scripts/tests/queue-events.test.mjs` | Unit tests for `statusToastTransition` (**F5**). |
| `src/main.ts` | Wire `statusToastTransition` into the `store.on("job")` listener (**F5**). |
| `src/ui/components.ts` | Guard `confirmSheet` against re-entrant `dialog.close()` during its 160 ms exit animation (**F8**); add roving `tabIndex` and `ArrowLeft`/`ArrowRight` navigation to `chipRadio` (**F11**). |
| `src/store.ts` & `src/views/download.ts` | Keep single-link preview state across route navigation when no batch is replacing it (**F10**). |
| `src/views/library.ts` | Include `filePath`, `quality`, and `finishedAt` in `tileFor` cache key (**F12**). |
| `.gitattributes` | Normalize text line endings to LF to eliminate CRLF warnings on `Cargo.toml` and `Cargo.lock` (**F13**). |

---

### Task 1: Prevent buffered `stdout` progress lines from overwriting `JobStatus::Processing` (F1)

**Files:**
- Modify: `src-tauri/src/downloader.rs:1524-1540`
- Test: `src-tauri/src/downloader.rs` (unit test module)

- [ ] **Step 1: Write the failing test in `src-tauri/src/downloader.rs`**

Add a unit test in `src-tauri/src/downloader.rs`'s `#[cfg(test)] mod tests` verifying that status transition on a progress line only moves `Queued` to `Downloading` and never regresses `Processing` back to `Downloading`:

```rust
    #[test]
    fn progress_line_never_regresses_processing_back_to_downloading() {
        assert_eq!(status_after_progress(JobStatus::Queued), JobStatus::Downloading);
        assert_eq!(status_after_progress(JobStatus::Downloading), JobStatus::Downloading);
        assert_eq!(status_after_progress(JobStatus::Processing), JobStatus::Processing);
        assert_eq!(status_after_progress(JobStatus::Paused), JobStatus::Paused);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml progress_line_never_regresses_processing_back_to_downloading`\
Expected: FAIL with `cannot find function status_after_progress in this scope`.

- [ ] **Step 3: Write minimal implementation in `src-tauri/src/downloader.rs`**

Add `status_after_progress` above `handle_line` and use it inside `handle_line` (`src-tauri/src/downloader.rs:1525-1539`):

```rust
fn status_after_progress(current: JobStatus) -> JobStatus {
    match current {
        JobStatus::Queued => JobStatus::Downloading,
        other => other,
    }
}
```

And in `handle_line`:

```rust
        let finished = fields[0] == "finished";
        if finished || last_emit.elapsed() >= Duration::from_millis(150) {
            *last_emit = Instant::now();
            update_job(app, id, |j| {
                j.status = status_after_progress(j.status);
                j.progress_known = snapshot.fraction.is_some();
                j.progress = snapshot.fraction.unwrap_or(j.progress);
                j.progress_estimated = snapshot.estimated;
                j.downloaded_bytes = Some(snapshot.downloaded);
                j.total_bytes = snapshot.total;
                if j.status != JobStatus::Processing {
                    j.speed = snapshot.speed;
                    j.eta = snapshot.eta;
                }
            });
        }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml progress_line_never_regresses_processing_back_to_downloading`\
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/downloader.rs
git commit -m "fix(downloader): keep Processing status when late stdout progress lines arrive"
```

---

### Task 2: Harden schema-1 migration and retry renames in `queue_writer.rs` (F2, F3)

**Files:**
- Modify: `src-tauri/src/queue_writer.rs:196-223` and `src-tauri/src/queue_writer.rs:279-285`
- Test: `src-tauri/src/queue_writer.rs` (`mod tests`)

- [ ] **Step 1: Write the failing tests in `src-tauri/src/queue_writer.rs`**

Add two tests to `mod tests` in `src-tauri/src/queue_writer.rs`:

```rust
    #[test]
    fn corrupt_schema_1_array_is_set_aside_even_when_pre_1_0_5_backup_exists() {
        let dir = temp("schema1-corrupt");
        let path = dir.join("queue.json");
        let pre = dir.join("queue.pre-1.0.5.json");
        std::fs::write(&pre, b"earlier valid backup").unwrap();
        std::fs::write(&path, br#"[{"not_a_valid_job": true}]"#).unwrap();
        let mut notices = Vec::new();
        assert!(load(&path, &mut notices).is_empty());
        assert!(!path.exists(), "corrupt array queue must be moved aside");
        assert_eq!(std::fs::read(&pre).unwrap(), b"earlier valid backup");
        assert!(notices.iter().any(|n| n.contains("queue.corrupt-")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn schema_1_array_queue_also_recovers_preserved_newer_version_backup() {
        let dir = temp("schema1-downgrade");
        let path = dir.join("queue.json");
        let old_job = schema_2_job("from-schema-3", "paused", "null");
        let array_job = schema_1_job("from-1-0-4", "paused", "null");
        std::fs::write(&path, format!("[{array_job}]")).unwrap();
        let backup = dir.join("queue.newer-version-20261108-120000.json");
        std::fs::write(&backup, format!(r#"{{"schemaVersion":3,"revision":4,"jobs":[{old_job}]}}"#)).unwrap();
        let mut notices = Vec::new();
        let restored = load(&path, &mut notices);
        assert_eq!(
            restored.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(),
            ["from-schema-3", "from-1-0-4"]
        );
        assert!(!backup.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml schema_1`\
Expected: FAIL (`corrupt_schema_1_array_is_set_aside_even_when_pre_1_0_5_backup_exists` fails because `path.exists()` is still true, and `schema_1_array_queue_also_recovers_preserved_newer_version_backup` fails because `from-schema-3` is not recovered).

- [ ] **Step 3: Write minimal implementation in `src-tauri/src/queue_writer.rs`**

Update `set_aside`, the `value.is_array()` branch, and `recover_downgrade` in `src-tauri/src/queue_writer.rs`:

```rust
    let set_aside = |label: &str| -> std::io::Result<PathBuf> {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let aside = path.with_file_name(format!("queue.{label}-{stamp}-{}.json", uuid::Uuid::new_v4()));
        crate::store::rename_retrying(path, &aside)?;
        Ok(aside)
    };
```

And in the `if value.is_array()` block (`src-tauri/src/queue_writer.rs:210-223`):

```rust
    if value.is_array() {
        // 1.0.4 or earlier: backed up once before the new format replaces it.
        let backup = path.with_file_name("queue.pre-1.0.5.json");
        if !backup.exists() {
            std::fs::copy(path, &backup)?;
        }
        return Ok(match serde_json::from_value(value) {
            Ok(jobs) => recover_downgrade(path, jobs, notices),
            Err(_) => {
                let aside = set_aside("corrupt")?;
                notices.push(format!(
                    "Your download queue from an earlier version could not be read, so Claws started with an empty one. The old file was kept as {}.",
                    file_name(&aside)
                ));
                recover_downgrade(path, Vec::new(), notices)
            }
        });
    }
```

And in `recover_downgrade` (`src-tauri/src/queue_writer.rs:280-283`):

```rust
    // Consume only after the restored queue is durable, preventing resurrection later.
    let consumed = backup.with_extension("recovered");
    if let Err(error) = crate::store::rename_retrying(&backup, &consumed) {
        notices.push(format!("The queue was restored, but its backup could not be marked recovered ({error})."));
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml queue_writer`\
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/queue_writer.rs
git commit -m "fix(queue): set aside corrupt schema-1 queues and retry backup renames"
```

---

### Task 3: Fix self-drop outcome classification and superscript device names in `file_drag.rs` (F4, F9)

**Files:**
- Modify: `src-tauri/src/file_drag.rs:27-36`, `src-tauri/src/file_drag.rs:107-129`, `src-tauri/src/file_drag.rs:184-196`, `src-tauri/src/file_drag.rs:237-257`
- Test: `src-tauri/src/file_drag.rs` (`mod tests`)

- [ ] **Step 1: Write the failing tests in `src-tauri/src/file_drag.rs`**

Update `only_a_release_over_another_window_counts_as_dropped` and `a_drop_reported_by_claws_window_marks_the_drag_as_dropped_on_itself`, and add superscript device cases in `shell_paths_keep_long_and_network_locations_without_filesystem_prefixes` in `src-tauri/src/file_drag.rs`:

```rust
    #[test]
    fn only_a_release_over_another_window_counts_as_dropped() {
        assert_eq!(outcome(true, true, false), DragOutcome::Dropped);
        assert_eq!(outcome(true, false, false), DragOutcome::Cancelled);
        assert_eq!(outcome(true, true, true), DragOutcome::DroppedOnSelf);
        assert_eq!(outcome(true, false, true), DragOutcome::DroppedOnSelf);
        assert_eq!(outcome(false, false, false), DragOutcome::Cancelled);
        assert_eq!(outcome(false, false, true), DragOutcome::Cancelled);
        assert_eq!(outcome(false, true, true), DragOutcome::Cancelled);
        let names: Vec<String> = [DragOutcome::Dropped, DragOutcome::Cancelled, DragOutcome::DroppedOnSelf, DragOutcome::NotStarted]
            .iter()
            .map(|o| serde_json::to_value(o).unwrap().as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["dropped", "cancelled", "droppedOnSelf", "notStarted"]);
    }

    #[test]
    fn a_drop_reported_by_claws_window_marks_the_drag_as_dropped_on_itself() {
        forget_window_drops();
        assert!(!window_received_drop());
        assert_eq!(outcome(true, true, window_received_drop()), DragOutcome::Dropped);
        note_window_drop();
        assert_eq!(outcome(true, true, window_received_drop()), DragOutcome::DroppedOnSelf);
        assert_eq!(outcome(true, false, window_received_drop()), DragOutcome::DroppedOnSelf);
        // The next drag starts clean.
        forget_window_drops();
        assert_eq!(outcome(true, true, window_received_drop()), DragOutcome::Dropped);
    }

    #[test]
    fn shell_paths_keep_long_and_network_locations_without_filesystem_prefixes() {
        assert_eq!(shell_location(Path::new(r"\\?\UNC\server\share\file.mp4")).unwrap(), PathBuf::from(r"\\server\share\file.mp4"));
        assert_eq!(shell_location(Path::new(r"\\?\C:\media\file.mp4")).unwrap(), PathBuf::from(r"C:\media\file.mp4"));
        for path in [
            r"C:\media\NUL.mp4",
            r"C:\media\COM1.mp4",
            r"C:\media\COM¹.mp4",
            r"C:\media\LPT².mp4",
            r"C:\media\LPT³.mp4",
            r"C:\media\file.",
            r"C:\media\file ",
        ] {
            assert!(shell_location(Path::new(path)).is_err(), "{path}");
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml file_drag`\
Expected: FAIL to compile (`outcome` takes 2 arguments instead of 3) and `COM¹.mp4` is not rejected.

- [ ] **Step 3: Write minimal implementation in `src-tauri/src/file_drag.rs`**

Update `outcome`, `shell_location`, `start`, and `native::drag` in `src-tauri/src/file_drag.rs`:

```rust
/// What a finished drag means, from whether OLE ended with a drop (`DRAGDROP_S_DROP`),
/// whether the receiver reported `DROPEFFECT_COPY`, and whether Claws' own window
/// received the drop.
pub fn outcome(dropped: bool, copy: bool, on_claws: bool) -> DragOutcome {
    match (dropped, copy, on_claws) {
        (true, _, true) => DragOutcome::DroppedOnSelf,
        (true, true, false) => DragOutcome::Dropped,
        _ => DragOutcome::Cancelled,
    }
}
```

In `shell_location`:

```rust
fn is_reserved_device_stem(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    if let Some(rest) = upper.strip_prefix("COM").or_else(|| upper.strip_prefix("LPT")) {
        return matches!(rest, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "\u{00b9}" | "\u{00b2}" | "\u{00b3}");
    }
    false
}
```

And use `is_reserved_device_stem(stem)` inside `shell_location`:

```rust
        if let Component::Normal(name) = part {
            let name = name.to_string_lossy();
            let stem = name.split('.').next().unwrap_or("");
            if is_reserved_device_stem(stem) || name.ends_with(['.', ' ']) {
                return Err(AppError::msg("This file uses a name Windows reserves. Use Show in folder instead."));
            }
        }
```

In `start` and `native::drag`:

```rust
    let Some((dropped, copy)) = ended else { return Ok(DragOutcome::NotStarted) };
    if !dropped {
        return Ok(DragOutcome::Cancelled);
    }
    // A drop on Claws reaches the event loop as a message wry posted during the drop. This
    // task is posted after the drag returned, so it runs after that message was handled.
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(window_received_drop());
    })?;
    let on_claws = receiver.await.map_err(|_| AppError::msg("The drag was interrupted."))?;
    Ok(outcome(dropped, copy, on_claws))
```

And in `native::drag`:

```rust
    pub fn drag(path: PathBuf) -> Result<Option<(bool, bool)>> {
        unsafe {
            OleInitialize(None).map_err(|e| AppError::msg(e.to_string()))?;
            struct Apartment;
            impl Drop for Apartment { fn drop(&mut self) { unsafe { OleUninitialize(); } } }
            let _apartment = Apartment;
            let swapped = GetSystemMetrics(SM_SWAPBUTTON) != 0;
            let primary = if swapped { VK_RBUTTON } else { VK_LBUTTON };
            if GetAsyncKeyState(i32::from(primary.0)) >= 0 { return Ok(None); }
            let data = data_object(&path).map_err(|e| AppError::msg(format!("Windows cannot prepare this file for dragging ({e}). Use Show in folder instead.")))?;
            // Recheck after Shell preparation, which may hydrate a cloud/network file.
            if GetAsyncKeyState(i32::from(primary.0)) >= 0 { return Ok(None); }
            super::forget_window_drops();
            let source: IDropSource = Source { started: Instant::now(), primary: if swapped { MK_RBUTTON } else { MK_LBUTTON } }.into();
            let mut effect = DROPEFFECT::default();
            let status = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
            if status == DRAGDROP_S_CANCEL { return Ok(Some((false, false))); }
            status.ok().map_err(|e| AppError::msg(format!("Windows reported a drag problem ({e}); check the receiver before retrying.")))?;
            Ok(Some((status == DRAGDROP_S_DROP, (effect & DROPEFFECT_COPY).0 != 0)))
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml file_drag`\
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/file_drag.rs
git commit -m "fix(file-drag): classify DROPEFFECT_NONE self-drops and reject superscript device names"
```

---

### Task 4: Preserve playlist `thumbnail`, `uploader`, and `duration` in `remainingItems` (F6)

**Files:**
- Modify: `src/download-input.ts:201-208`
- Test: `scripts/tests/download-input.test.mjs`

- [ ] **Step 1: Write the failing test in `scripts/tests/download-input.test.mjs`**

Add a test in `scripts/tests/download-input.test.mjs` verifying that a playlist row in a `LinkBatch` passes `thumbnail`, `uploader`, and `duration` when enqueued:

```js
test("playlist rows in a batch pass each entry's thumbnail, uploader and duration to enqueue", async () => {
  const parsed = parseLinks("https://example.com/playlist\nhttps://example.com/single");
  const sent = [];
  const batch = new LinkBatch(parsed, {
    probe: async (url) =>
      url.endsWith("playlist")
        ? {
            kind: "playlist",
            title: "Set",
            uploader: "Channel",
            duration: null,
            thumbnail: "https://example.com/set.jpg",
            site: "example",
            url,
            heights: [1080],
            subtitles: [],
            entries: [
              {
                url: "https://example.com/v1",
                playlistIndex: 1,
                title: "Part 1",
                uploader: "Guest",
                duration: 95,
                thumbnail: "https://example.com/v1.jpg",
              },
              {
                url: "https://example.com/v2",
                playlistIndex: 2,
                title: "Part 2",
                uploader: null,
                duration: 120,
                thumbnail: null,
              },
            ],
          }
        : {
            kind: "video",
            title: "Solo",
            uploader: "Soloist",
            duration: 60,
            thumbnail: "https://example.com/solo.jpg",
            site: "example",
            url,
            heights: [1080],
            subtitles: [],
            entries: [],
          },
  });
  await batch.settled;
  await batch.enqueue(async (items) => {
    sent.push(...items);
    return { revision: 1, jobs: items.map((_, i) => ({ jobId: `j${i}`, created: true })) };
  });
  assert.deepEqual(sent.slice(0, 2), [
    {
      url: "https://example.com/v1",
      playlistIndex: 1,
      title: "Part 1",
      thumbnail: "https://example.com/v1.jpg",
      uploader: "Guest",
      duration: 95,
    },
    {
      url: "https://example.com/v2",
      playlistIndex: 2,
      title: "Part 2",
      thumbnail: null,
      uploader: "Channel",
      duration: 120,
    },
  ]);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test:logic`\
Expected: FAIL (`AssertionError: Expected values to be strictly deep-equal` because `thumbnail`, `uploader`, and `duration` are missing on the playlist items).

- [ ] **Step 3: Write minimal implementation in `src/download-input.ts`**

Update `remainingItems` in `src/download-input.ts:201-212`:

```ts
  const items: EnqueueItem[] =
    res.kind === "playlist"
      ? res.entries.map((e) => ({
          url: e.url,
          playlistIndex: e.playlistIndex,
          title: e.title || res.title || row.url,
          thumbnail: e.thumbnail,
          uploader: e.uploader ?? res.uploader,
          duration: e.duration,
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

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test:logic`\
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/download-input.ts scripts/tests/download-input.test.mjs
git commit -m "fix(batch): preserve playlist entry thumbnail, uploader and duration on enqueue"
```

---

### Task 5: Deduplicate failed-download and completed-download toasts by status transition (F5)

**Files:**
- Modify: `src/queue-events.ts`
- Modify: `src/main.ts:132-142`
- Test: `scripts/tests/queue-events.test.mjs`

- [ ] **Step 1: Write the failing test in `scripts/tests/queue-events.test.mjs`**

Add a test in `scripts/tests/queue-events.test.mjs` for `statusToastTransition`:

```js
test("statusToastTransition fires once per entry into completed or failed, and again after retry", () => {
  const seen = new Map();
  assert.equal(statusToastTransition(seen, { id: "a", status: "downloading" }, "download"), null);
  assert.equal(statusToastTransition(seen, { id: "a", status: "failed" }, "download"), "failed");
  // A second update while still failed must not toast again.
  assert.equal(statusToastTransition(seen, { id: "a", status: "failed" }, "download"), null);
  // Retried and failed again: toasts again.
  assert.equal(statusToastTransition(seen, { id: "a", status: "downloading" }, "download"), null);
  assert.equal(statusToastTransition(seen, { id: "a", status: "failed" }, "download"), "failed");
  // Failures while already looking at the Queue view do not toast.
  assert.equal(statusToastTransition(seen, { id: "b", status: "failed" }, "queue"), null);
  // Completion toasts once per transition into completed.
  assert.equal(statusToastTransition(seen, { id: "b", status: "completed" }, "download"), "completed");
  assert.equal(statusToastTransition(seen, { id: "b", status: "completed" }, "download"), null);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test:logic`\
Expected: FAIL (`statusToastTransition` is not exported from `queue-events.ts`).

- [ ] **Step 3: Write minimal implementation in `src/queue-events.ts` and `src/main.ts`**

Add `statusToastTransition` at the bottom of `src/queue-events.ts`:

```ts
/**
 * Decides whether a job event should trigger a global "completed" or "failed" toast.
 * Fires only when the job's status transitions into that state, never on repeated
 * updates in the same state.
 */
export function statusToastTransition(
  seen: Map<string, VersionedJob["status"]>,
  job: Pick<VersionedJob, "id" | "status">,
  route: string,
): "completed" | "failed" | null {
  const previous = seen.get(job.id);
  seen.set(job.id, job.status);
  if (previous === job.status) return null;
  if (job.status === "completed") return "completed";
  if (job.status === "failed" && route !== "queue") return "failed";
  return null;
}
```

And update `src/main.ts:132-142`:

```ts
  const seenStatuses = new Map<string, Job["status"]>();
  for (const job of store.jobList()) seenStatuses.set(job.id, job.status);
  store.on("job", (detail) => {
    const job = detail as Job;
    const notice = statusToastTransition(seenStatuses, job, store.route);
    if (notice === "completed") {
      void store.refreshHistory();
      const what = job.options.mode === "transcript" ? "Transcript saved" : "Finished";
      toast(`${what}: “${job.title}”`, {
        action: { label: "Show in Library", run: () => store.go("library") },
      });
    } else if (notice === "failed") {
      toast(`Couldn't download “${job.title}”.`, {
        tone: "error",
        action: { label: "See Why", run: () => store.go("queue") },
      });
    }
  });
```

(Also import `statusToastTransition` in `src/main.ts` from `./queue-events`.)

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test:logic`\
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/queue-events.ts src/main.ts scripts/tests/queue-events.test.mjs
git commit -m "fix(ui): deduplicate failed and completed job toasts by status transition"
```

---

### Task 6: Guard `confirmSheet` exit animation and add arrow-key navigation to `chipRadio` (F8, F11)

**Files:**
- Modify: `src/ui/components.ts:123-152` and `src/ui/components.ts:295-345`

- [ ] **Step 1: Update `chipRadio` and `confirmSheet` in `src/ui/components.ts`**

In `src/ui/components.ts:123-152`, add roving `tabIndex` and `ArrowRight`/`ArrowLeft` navigation to `chipRadio`:

```ts
/** A single-choice row of pill buttons (radio semantics). */
export function chipRadio<T extends string | number>(
  label: string,
  options: ChipOption<T>[],
  value: T,
  onChange: (value: T) => void,
): HTMLElement {
  const group = h("div", { class: "chips", attrs: { role: "radiogroup", "aria-label": label } });
  const chips = options.map((option) =>
    h(
      "button",
      {
        class: "chip",
        attrs: { type: "button", role: "radio", title: option.hint },
        on: {
          click: () => {
            mark(option.value);
            onChange(option.value);
          },
        },
      },
      option.label,
    ),
  );
  function mark(current: T) {
    const activeIndex = Math.max(0, options.findIndex((o) => o.value === current));
    chips.forEach((chip, i) => {
      const checked = options[i].value === current;
      chip.setAttribute("aria-checked", String(checked));
      chip.tabIndex = i === activeIndex ? 0 : -1;
    });
  }
  group.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
    event.preventDefault();
    const step = event.key === "ArrowRight" ? 1 : -1;
    const current = Math.max(0, chips.findIndex((c) => c.getAttribute("aria-checked") === "true"));
    const next = (current + step + chips.length) % chips.length;
    chips[next].focus();
    chips[next].click();
  });
  chips.forEach((c) => group.appendChild(c));
  mark(value);
  return group;
}
```

And in `confirmSheet` (`src/ui/components.ts:298-341`), guard against re-entrant `dialog.close()` calls:

```ts
    let result: string | null = null;
    let closing = false;
    // The safe choice takes focus, so Enter never confirms something destructive.
    const cancel = button({ label: "Cancel", variant: "plain", onClick: () => dialog.close() });
    cancel.autofocus = true;
    const body = h(
      "div",
      { class: "sheet-body" },
      h("h2", { class: "sheet-title", text: title, attrs: { id: "sheet-title" } }),
      h("p", { class: "sheet-message", text: message }),
      h(
        "div",
        { class: "sheet-actions" },
        ...actions.map((action) =>
          button({
            label: action.label,
            variant: action.variant ?? "secondary",
            onClick: () => {
              if (closing) return;
              result = action.value;
              dialog.close();
            },
          }),
        ),
        cancel,
      ),
    );
    dialog.appendChild(body);
    dialog.addEventListener("close", () => {
      dialog.remove();
      resolve(result);
    });
    const close = dialog.close.bind(dialog);
    dialog.close = () => {
      if (closing) return;
      closing = true;
      // No departure with reduced motion or in a hidden window: the sheet just closes.
      if (!canAnimate()) return close();
      const out = dialog.animate([{ opacity: 1, transform: "none" }, { opacity: 0, transform: "scale(0.96)" }], {
        duration: 160,
        easing: "cubic-bezier(0.4, 0, 1, 1)",
      });
      out.onfinish = () => close();
    };
```

- [ ] **Step 2: Verify build and type-check**

Run: `npm run build`\
Expected: PASS (`tsc && vite build && node scripts/check-fonts.mjs --dist`)

- [ ] **Step 3: Commit**

```bash
git add src/ui/components.ts
git commit -m "fix(ui): guard confirmSheet close animation and add arrow-key support to chipRadio"
```

---

### Task 7: Sample disk free space outside the `disk_ledger` lock in `downloader.rs` (F7)

**Files:**
- Modify: `src-tauri/src/downloader.rs:1415-1426` and `src-tauri/src/downloader.rs:1452-1486`
- Test: `src-tauri/src/lifecycle_tests.rs`

- [ ] **Step 1: Update `Budget::after_line` and `recheck_processing_space` in `src-tauri/src/downloader.rs`**

In `Budget::after_line` (`src-tauri/src/downloader.rs:1415-1426`) and `recheck_processing_space` (`src-tauri/src/downloader.rs:1452-1463`), sample `(state.free_space)(&self.workspace)` and `disk_budget::workspace_bytes(&self.workspace)` before locking `state.disk_ledger` (admission in `prepare` remains serialized under `disk_ledger`, while periodic in-flight telemetry does not hold the mutex across Win32 disk I/O):

```rust
    fn after_line(&mut self, app: &AppHandle, line: &str) {
        let finished = line.starts_with("CLAWS|finished|");
        if !finished && self.last.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last = Instant::now();
        let written = disk_budget::workspace_bytes(&self.workspace);
        let state = app.state::<AppState>();
        let _ = state.disk_ledger.lock().unwrap().observe(&self.id, written);
    }
```

And in `recheck_processing_space`:

```rust
fn recheck_processing_space(app: &AppHandle, id: &str, workspace: &Path, plan: &RunPlan) {
    if !plan.needs_processing {
        return;
    }
    let state = app.state::<AppState>();
    let free = (state.free_space)(workspace).ok();
    let written = disk_budget::workspace_bytes(workspace);
    let check = {
        let mut ledger = state.disk_ledger.lock().unwrap();
        ledger.recheck_processing(id, plan, written, free)
    };
```

- [ ] **Step 2: Run Rust tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`\
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/downloader.rs
git commit -m "perf(downloader): avoid holding disk_ledger lock during periodic workspace disk reads"
```

---

### Task 8: Complete Library tile cache key, preserve Download preview across routes, and add `.gitattributes` (F10, F12, F13)

**Files:**
- Modify: `src/views/library.ts:107-114`
- Modify: `src/store.ts` and `src/views/download.ts`
- Create: `.gitattributes`

- [ ] **Step 1: Include `filePath`, `quality`, and `finishedAt` in `src/views/library.ts`**

Update `tileFor` in `src/views/library.ts:107-114`:

```ts
  const tileFor = (item: HistoryItem) => {
    const key = `${item.title}|${item.thumbnailLocal ?? item.thumbnail}|${item.fileSize}|${item.filePath ?? ""}|${item.quality}|${item.finishedAt}`;
    const cached = built.get(item.id);
    if (cached && cached.key === key) return cached.el;
    const el = tile(item);
    built.set(item.id, { key, el });
    return el;
  };
```

- [ ] **Step 2: Preserve single-link preview session across route changes in `src/store.ts` and `src/views/download.ts`**

In `src/store.ts`, add `PreviewSession` and `preview: PreviewSession | null = null` on `Store`:

```ts
export interface PreviewSession {
  url: string;
  result: ProbeResult;
  mode: DownloadMode;
  height: number | null;
  videoPreset: VideoPreset;
  audioFormat: string;
  subs: string[];
  embedSubs: boolean;
  fastSave: boolean;
  caption: string | null;
  transcriptFormat: TranscriptFormat;
  selected: number[];
}
```

In `src/views/download.ts`, restore `store.preview` when mounting with `!store.batch && store.preview`, clear `store.preview = null` inside `lookUp`, `submitBatch`, and `startDownload`, and save the active preview on unmount:

```ts
  if (!store.batch && store.preview) {
    const saved = store.preview;
    lastUrl = saved.url;
    result = saved.result;
    mode = saved.mode;
    height = saved.height;
    videoPreset = saved.videoPreset;
    audioFormat = saved.audioFormat;
    subs = new Set(saved.subs);
    embedSubs = saved.embedSubs;
    fastSave = saved.fastSave;
    caption = saved.caption;
    transcriptFormat = saved.transcriptFormat;
    selected = new Set(saved.selected);
    phase = "result";
  }
```

And inside the unmount cleanup callback at the end of `mountDownload` (`src/views/download.ts:1524-1543`):

```ts
    if (phase === "result" && result) {
      store.preview = {
        url: lastUrl || result.url,
        result,
        mode,
        height,
        videoPreset,
        audioFormat,
        subs: [...subs],
        embedSubs,
        fastSave,
        caption,
        transcriptFormat,
        selected: [...selected],
      };
    }
```

- [ ] **Step 3: Create `.gitattributes` at the repository root**

```gitattributes
* text=auto eol=lf
*.bat text eol=crlf
*.cmd text eol=crlf
*.ps1 text eol=crlf
```

- [ ] **Step 4: Run full verification suite**

Run:
```bash
npm test
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --features test-tools -- -D warnings
```
Expected: All Node logic/font tests, Vite production build, Rust unit/lifecycle tests, and Clippy pass with zero warnings.

- [ ] **Step 5: Commit**

```bash
git add .gitattributes src/views/library.ts src/store.ts src/views/download.ts
git commit -m "chore: preserve single-link preview across routes, tighten library cache key and normalize line endings"
```

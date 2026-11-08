# Queue view: main-thread cost

Measured with `node scripts/dev/queue-bench.mjs` (raw results in `queue-baseline-legacy.json` and `queue-incremental-v1.json`).

## Setup

- The real frontend (`npm run dev`, dev mock, no backend) ran in headless Microsoft Edge on the development PC (Windows 11, NVIDIA GeForce GT 340), at a 1160 × 760 window.
- For each case the page was reloaded, and the Queue view opened with the mock's 6 sample jobs.
- `N` jobs were then added. The legacy protocol sent one `job-update` per job; v1 sends one `jobs-added` batch.
- After that came 40 rounds of progress, each with one event per active transfer, a frame apart.
- Milliseconds are Chrome DevTools Protocol layout plus style-recalculation time. A long task is a main-thread task over 50 ms.
- These are single runs on one machine. They are evidence of the change's direction and size, not a promise for other hardware.

## Results

| Added jobs | Active | Add, before | Add, after | Progress (40 rounds), before | Progress, after | Long tasks during progress, before → after |
| ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 1 | 1 | 4 ms | 2 ms | 42 ms | 34 ms | 0 → 0 |
| 100 | 8 | 233 ms | 18 ms | 193 ms | 48 ms | 0 → 0 |
| 500 | 1 | 4,418 ms | 74 ms | 760 ms | 47 ms | 6 → 0 |
| 500 | 8 | 4,370 ms | 73 ms | 729 ms | 63 ms | 39 → 0 |

After the change (v1, from the dev counters):

- Adding 500 jobs is one structural sync and 500 row paints. Before, every job triggered a full re-sort and repaint of the list.
- Each progress event repaints only its own row: 320 row paints for 8 × 40 events, with no structural syncs.
- Adding 500 jobs still produced one long task: building 500 rows in a single frame.

## Batched row creation (2026-11-05)

New rows are now built at most 40 per frame, also when the view opens. Measured with the updated `queue-bench.mjs`, which waits until every row exists and records the longest task; the "before" run used the previous `queue.ts` with the same benchmark (raw results: `queue-batched-v2.json`).

| Added jobs | Active | Long tasks while adding, before → after | Longest task, before → after | Rows |
| ---: | ---: | --- | --- | ---: |
| 100 | 1, 3, 8 | 0 → 0 | — | 106 |
| 500 | 1 | 1 → 0 | 109 ms → none | 506 |
| 500 | 3 | 1 → 0 | 109 ms → none | 506 |
| 500 | 8 | 1 → 0 | 117 ms → none | 506 |

Script time while adding 500 rises slightly (about 28 → 33–42 ms in total) because the work is split over more frames; progress updates are unchanged (≤ 0.016 ms per event, no long tasks).

## Pasted link batches (2026-11-07)

Measured with `node scripts/dev/batch-bench.mjs` on the same PC and headless Edge, against the dev mock with its lookup delays scaled to 2% (6 to 46 ms each). Every 25th link fails and every 50th is an 8-video playlist. Paste and Ctrl+Enter are separate tasks, as when typed. Three runs; raw results of the second are in `batch-links-v1.json`.

| Links | Long tasks: paste / lookups / add | Lookups: script, layout, style | Most lookups at once | Enqueue calls (items) |
| ---: | --- | --- | ---: | --- |
| 100 | 0 / 0 / 0 | 26–41, 54–60, 48–60 ms | 3 | 1 (110) |
| 500 | 0 / 0 / 0 | 140–169, 147–178, 74–83 ms | 3 | 2 (500 + 50) |

- Rows are built 40 per frame. Each answered lookup repaints its own row; the counts repaint at most once per frame.
- Two clicks on Download in one task sent each item once. A third press after everything was added sent nothing. The Queue view then showed 6 + 550 rows.
- Stop after 400 ms (30 links, unscaled mock delays): no lookup started after Stop, and the replies still running were ignored. All 30 rows showed as stopped.
- An earlier version animated the check's colours on every row as it became ready. Style recalculation for 500 links was then 2,730 ms in total, and rose as more rows were ready. Transform-only check feedback reduced this to 183–227 ms, and `content-visibility: auto` on the rows reduced it to 74–83 ms. Before these changes, adding 550 items at once caused one 55–115 ms long task in 4 of 5 runs; there were none in the 3 runs after.
- The existing `queue-bench.mjs` was re-run after this change: no long tasks while adding 1, 100 or 500 jobs, and at most 0.012 ms per progress event.
- These numbers come from the mock, not from real yt-dlp lookups, which take seconds each and are limited to three at a time.
- Disk: each video lookup saves yt-dlp's full answer (it can be megabytes, up to the 32 MB read limit) so the download can skip a second extraction. A batch writes one such file per video, so the cache is now capped at 256 MB in total as well as 20 minutes per file; the oldest files go first, after every write, one prune at a time. A download that starts after its file was evicted reads the site again; a file a running download already uses is held until that run ends, so the prune skips it. Not measured with real lookups.

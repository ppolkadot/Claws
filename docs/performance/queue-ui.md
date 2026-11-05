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
- Adding 500 jobs still produces one long task. That is building 500 rows in a single frame.

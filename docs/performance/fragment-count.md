# Fragment count with several jobs (improvement plan step 1.6, limitation L5)

**Date:** 2026-11-10 · **Raw results:** [`fragments-2026-11-10/`](fragments-2026-11-10/) · **Script:** `scripts/dev/download-bench.mjs`

## Result

**The default stays at 8 fragments.** Sixteen was faster for one job against a per-connection cap, but the gain did not repeat with three jobs at once, where 8 was the fastest and 16 was slower and erratic.

## Setup

Windows 11 (10.0.26200), Intel Core i5-12400F (12 logical CPUs), temp folder on an SSD, yt-dlp 2026.10.26. Localhost fixtures (`scripts/dev/download-fixtures.py`), HLS in 64 segments. One warm-up and three timed rounds per configuration, order alternated between rounds, every file checked against the fixture's SHA-256 (all passed). With several jobs the time is the whole round and the throughput is the aggregate.

These are fixture numbers, not Internet downloads. This PC differs from the one in the [speed diagnosis](speed-diagnosis.md), so absolute numbers are not comparable with that page.

## A. Per-connection cap of 8 MiB/s, 64 MiB

| Fragments | 1 job: median (spread) | Throughput | 3 jobs: median (spread) | Aggregate |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 9.36 s (0.16) | 6.8 MiB/s | 9.70 s (0.09) | 19.8 MiB/s |
| 4 | 3.25 s (0.25) | 19.7 MiB/s | 3.97 s (0.36) | 48.4 MiB/s |
| 8 (default) | 2.49 s (1.03) | 25.7 MiB/s | 3.30 s (0.10) | 58.2 MiB/s |
| 16 | 1.92 s (0.15) | 33.4 MiB/s | 4.06 s (3.70) | 47.3 MiB/s |

## B. As A, plus a server-wide cap of 16 MiB/s

| Fragments | 1 job: median (spread) | Throughput | 3 jobs: median (spread) | Aggregate |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 9.36 s (0.09) | 6.8 MiB/s | 13.72 s (0.39) | 14.0 MiB/s |
| 4 | 5.36 s (0.37) | 11.9 MiB/s | 13.52 s (0.17) | 14.2 MiB/s |
| 8 (default) | 5.48 s (0.32) | 11.7 MiB/s | 13.60 s (0.17) | 14.1 MiB/s |
| 16 | 5.28 s (0.23) | 12.1 MiB/s | 13.58 s (0.17) | 14.1 MiB/s |

## C. No cap, 128 MiB

| Fragments | 1 job | 3 jobs: median (spread) | 8 jobs: median (spread) |
| ---: | ---: | ---: | ---: |
| 1 | 2.85 s, 44.9 MiB/s | 5.62 s (3.84), 68.3 MiB/s | 11.66 s (2.19), 87.8 MiB/s |
| 4 | 1.97 s, 65.1 MiB/s | 9.92 s (7.56), 38.7 MiB/s | 24.83 s (2.12), 41.2 MiB/s |
| 8 (default) | 2.02 s, 63.3 MiB/s | 5.11 s (8.01), 75.2 MiB/s | 27.72 s (2.13), 36.9 MiB/s |
| 16 | 1.67 s, 76.7 MiB/s | 11.78 s (9.36), 32.6 MiB/s | 27.20 s (5.61), 37.6 MiB/s |

## What the results say

- **Against a per-connection cap, 8 is the best measured value for several jobs.** One job gains from 16 (1.3× over 8, though the 8-fragment runs spread by 1.03 s). Three jobs do best at 8; at 16 the aggregate drops and the spread grows to 3.7 s.
- **Against a server-wide cap, the fragment count does not matter beyond 4.** Every multi-job configuration ends at about 14 MiB/s.
- **The uncapped table mostly measures the fixture.** The fixture server is one Python process that generates every byte, so with many connections it is the bottleneck, and the multi-job spreads are as large as the medians. With 8 jobs, one fragment each was the fastest, which suggests that many connections contend, but it does not separate the server from yt-dlp or the disk. No conclusion is drawn from it.
- CPU time per job was 1.2–3.0 s in every configuration.

## Not measured

A real HLS or DASH site, real network latency and loss, a spinning or nearly full disk, the Claws app end to end, and counts between 8 and 16.

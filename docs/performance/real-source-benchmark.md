# Real-source benchmark gate (improvement plan step 1.1)

**Date:** 2026-11-10 · **Source:** `improve/phase-0-safety-net` (1.0.8 plus CI) · **Raw results:** [`real-source-2026-11-10/`](real-source-2026-11-10/) · **Script:** `scripts/dev/real-source-bench.mjs`

## Result

**No variant passed the gate** (median at least 1.5× faster with matching output). Steps 1.2 (`formats=dashy`) and 1.4 (aria2c transport) are not justified by these numbers.

Neither public link was slow on this connection, so this run does not reproduce a per-connection cap on a real source. It shows that the alternatives do not help when the source is already fast; it does not show what they do against a throttled source. The owner's slow link, if one exists, is still the missing evidence (see [speed diagnosis](speed-diagnosis.md)).

## Setup

Windows 11 (10.0.26200), yt-dlp 2026.10.26 (the copy Claws manages), aria2 1.37.0 (official Windows 64-bit build, zip SHA-256 `67d015301eef0b612191212d564c5bb0a14b5b9c4796b76454276a4d28d9b288`, used for this benchmark only), ffmpeg from PATH. Three rounds per variant, variant order rotated between rounds, no warm-up. Every run extracts the link again, downloads to a fresh temporary folder, is inspected with ffprobe and deleted. yt-dlp is driven directly with Claws' arguments (`-f bv*+ba/b -S res:N,vcodec:h264,acodec:aac --merge-output-format mp4 --concurrent-fragments 8`); the Claws app itself was not measured.

- **baseline:** the arguments above.
- **dashy:** plus `--extractor-args youtube:formats=dashy`.
- **aria2c:** plus `--downloader http,https:aria2c` (yt-dlp's default aria2c options).

"Transfer" runs from yt-dlp's `before_dl` hook to its `post_process` hook. "Total" is the whole process, including extraction and the merge.

## YouTube, 1080p60 H.264 + AAC, 275 MiB

`https://www.youtube.com/watch?v=aqz-KE-bpKQ` (Big Buck Bunny, Blender Foundation).

| Variant | Formats (protocol) | Total, 3 runs (s) | Median total | Transfer, 3 runs (s) | Median transfer | Speed-up, total / transfer |
| --- | --- | --- | ---: | --- | ---: | ---: |
| baseline | 299+258 (https+https) | 13.09, 7.82, 12.06 | 12.06 s | 7.59, 3.66, 6.14 | 6.14 s (44.9 MiB/s) | 1.00 / 1.00 |
| dashy | 312+258 (m3u8_native + http_dash_segments) | 8.82, 8.84, 8.46 | 8.82 s | 4.20, 4.51, 3.54 | 4.20 s (65.6 MiB/s) | 1.37 / 1.46 |
| aria2c | 299+258 (https+https) | 17.00, 8.29, 18.38 | 17.00 s | 5.38, 4.20, 4.20 | 4.20 s (65.6 MiB/s) | 0.71 / 1.46 |

- Output matched the baseline in every run: H.264 1080p60 + AAC, 634.62 s. The dashy file is 0.2 MiB larger (275.6 against 275.4 MiB) because it takes the video from the HLS rendition (format 312) rather than format 299; codec, size and frame rate are the same.
- The baseline already moved 36–75 MiB/s. Its fastest run (3.66 s) beat every dashy and aria2c run but one, so the ranges overlap and three runs cannot separate them.
- dashy was the steadiest (total spread 0.38 s against 5.27 s for the baseline). That is a consistency observation, not a 1.5× gain.
- aria2c's long totals (17.0 and 18.4 s) were spent after the transfer, in the merge step (8.4 and 11.4 s against 0.9–3.0 s elsewhere). The cause was not investigated; it may be disk write-back as in the earlier processing measurements.

## archive.org, plain HTTP, 360p MP4, 59 MiB

`https://archive.org/details/BigBuckBunny_124`, one plain-HTTP file (format 1).

| Variant | Total, 3 runs (s) | Median total | Transfer, 3 runs (s) | Median transfer | Speed-up, total / transfer |
| --- | --- | ---: | --- | ---: | ---: |
| baseline | 16.55, 9.48, 9.20 | 9.48 s | 14.57, 7.68, 7.30 | 7.68 s (7.7 MiB/s) | 1.00 / 1.00 |
| dashy (no effect off YouTube; a noise control) | 9.26, 9.62, 9.27 | 9.27 s | 7.34, 7.84, 7.58 | 7.58 s (7.8 MiB/s) | 1.02 / 1.01 |
| aria2c | 8.87, 10.80, 9.23 | 9.23 s | 7.11, 8.95, 7.44 | 7.44 s (7.9 MiB/s) | 1.03 / 1.03 |

- All nine outputs were identical in streams, duration and size.
- aria2c with 16 connections ended at the same roughly 8 MiB/s as one connection. That matches the earlier fixture case of a server-wide or link limit (cases C and G in the speed diagnosis): more connections cannot help.
- The first baseline run (16.55 s) was the first request of the session and is treated as a cold outlier.

## Decision

| Step | Decision | Reason |
| --- | --- | --- |
| 1.2 `formats=dashy` | Not built | 1.37× total and 1.46× transfer on one fast link, under the 1.5× gate, with overlapping run ranges. |
| 1.4 aria2c transport | Not built | No gain on the plain-HTTP source; a transfer gain on YouTube inside the noise, and a worse median total. |
| 1.3, 1.5, 1.6 | Unaffected | They do not depend on this gate. |

## Not measured

- A source that is actually slow or capped per connection. Both links ran at the speed of the link or the server.
- The Claws app end to end, several jobs at once, other resolutions, other times of day, authenticated sources.
- More than three runs per variant; the YouTube spread is large for that sample size.
- CPU use and request counts under `formats=dashy`.

## Speed limit and fragments (step 1.5)

Raw: [`real-source-2026-11-10/limit-rate-fragments.json`](real-source-2026-11-10/limit-rate-fragments.json). Local fixture (`scripts/dev/download-fixtures.py`, 16 MiB, 64 HLS segments), yt-dlp 2026.10.26, `--limit-rate 1024K`, one run each.

| Source | Fragments | Time | Effective speed |
| --- | ---: | ---: | ---: |
| HLS | 1 | 17.39 s | 942 KiB/s |
| HLS | 8 | 3.16 s | 5,179 KiB/s |
| plain HTTP | 8 | 16.99 s | 964 KiB/s |

yt-dlp applies the limit to each fragment connection, so eight fragments ran at about five times the limit. A limited start in Claws therefore uses one fragment. Not measured: the limit through the Claws app, or on a real site.

# Speed diagnosis (Task 1)

**Date:** 2026-11-05 · **Source:** `download-reliability` at `5167328` · **Raw results:** [`speed-diagnosis-2026-11-05/`](speed-diagnosis-2026-11-05/)

## What this is, and what it is not

The affected slow source has not been identified: no URL, site or format was available
for this pass. **The real-source diagnosis is therefore pending.** What follows are
controlled measurements on this PC that separate the possible causes, so that once the
slow source is known, one comparison against it says which fix applies.

Every number here is a localhost fixture (`scripts/dev/download-fixtures.py`) or synthetic
media, not an Internet download. A fixture's rate cap stands in for a server's limit; it
says what each strategy does *against that kind of limit*, not how fast any real site is.

**Setup.** Windows 11 (10.0.26200), AMD Ryzen 7 5700X (16 logical CPUs), temp folder on
an SSD. yt-dlp 2026.10.26 (the copy Claws installed), ffmpeg from yt-dlp's FFmpeg-Builds
"latest" (checksum verified against the published `checksums.sha256`). Transfer runs:
one warm-up and three timed rounds per configuration, configuration order alternated
between rounds, every file checked against the fixture's SHA-256 (all passed). Times are
whole rounds, including yt-dlp's link reading (about 1.3 s).

## Transfer

| | Configuration | Median | Spread | Throughput |
| --- | --- | ---: | ---: | ---: |
| **A** plain HTTP, 128 MiB, 8 MiB/s per connection | fragments 1 | 17.21 s | 0.25 s | 7.4 MiB/s |
| | fragments 8 | 17.47 s | 0.28 s | 7.3 MiB/s |
| | 3 jobs at once (aggregate) | 17.55 s | 0.19 s | 21.9 MiB/s |
| **B** HLS, 128 MiB in 32 segments, 8 MiB/s per connection | fragments 1 | 18.02 s | 0.59 s | 7.1 MiB/s |
| | fragments 4 | 5.66 s | 0.77 s | 22.6 MiB/s |
| | fragments 8 (Claws' setting) | 3.77 s | 0.40 s | 34.0 MiB/s |
| **C** server-wide cap 16 MiB/s, 128 MiB | HLS fragments 1 | 11.00 s | 0.40 s | 11.6 MiB/s |
| | HLS fragments 8 | 9.42 s | 0.20 s | 13.6 MiB/s |
| | plain HTTP, 1 job | 10.51 s | 0.07 s | 12.2 MiB/s |
| | plain HTTP, 3 jobs (aggregate) | 25.43 s | 0.03 s | 15.1 MiB/s |
| **D** uncapped, 512 MiB | plain HTTP | 2.38 s | 0.16 s | 215 MiB/s |
| | HLS fragments 1 / 8 | 3.84 / 2.20 s | 0.04 / 0.05 s | 133 / 233 MiB/s |
| **E** plain HTTP, 64 MiB, 8 MiB/s per connection | 1 job / 8 jobs (aggregate) | 9.33 / 10.08 s | 0.11 / 0.27 s | 6.9 / 50.8 MiB/s |
| **F** plain HTTP, 128 MiB, 8 MiB/s per connection | yt-dlp native | 17.35 s | 0.39 s | 7.4 MiB/s |
| | reference: 4 parallel ranges | 4.01 s | 0.01 s | 31.9 MiB/s |
| | reference: 8 parallel ranges | 2.01 s | 0.00 s | 63.7 MiB/s |
| **G** as F, plus a server-wide 16 MiB/s cap | native / 4 ranges / 8 ranges | 17.31 / 8.01 / 8.01 s | ≤ 0.19 s | 7.4 / 16.0 / 16.0 MiB/s |

"Reference" ranges are fetched by the benchmark script itself (`--downloader ranges:N`),
not by any Claws code path, and their times exclude yt-dlp's link reading, so they are
somewhat flattering. They show what a range-parallel transport could gain; aria2c through
yt-dlp (the plan's Task 8 candidate) was **not measured**: running the downloaded aria2c
binary was not permitted in this session.

What the transfer results say:

- **The standalone engine is not the measured ceiling.** Uncapped, one plain HTTP download runs at
  about 215 MiB/s and 1.3–1.5 CPU-seconds per job (D). This does not identify the cause of a slow real source or rule out overhead in the native Claws app.
- **HLS/DASH against a per-connection limit is already handled.** Throughput scales with
  fragments (B: 7.1 → 22.6 → 34.0 MiB/s for 1/4/8), and Claws already uses 8. No change
  is indicated; raising it is not tested and not recommended without a real source.
- **Plain HTTP against a per-connection limit is the one case Claws cannot speed up
  today.** One file stays at the cap whatever `--concurrent-fragments` says (A); only
  separate files run in parallel (A, E). Parallel ranges would lift it 4–8× (F). This is
  the case Task 8 (optional aria2c) exists for.
- **A server-wide or link limit cannot be beaten.** With the server capped at 16 MiB/s,
  more fragments, jobs or ranges all end at the cap (C, G). If the real source behaves
  like this, the honest answer is that the limit is outside Claws.

## Processing (ffmpeg, synthetic media)

`processing-bench.mjs`, one warm-up and three timed runs, inputs: a 3.8 GiB H.264 video
(60 s, lossless with grain; a single chunk already exceeded the 2 GiB target), a matching
AAC track, and a 60-minute Opus track. Stream copy throughout except `audio-mp3`.

| Step | Runs (s) | Median |
| --- | --- | ---: |
| merge (video + audio → MP4) | 39.6, 19.1, 8.3 | 19.1 s |
| metadata rewrite of the merged file | 34.4, 19.1, 57.3 | 34.4 s |
| merge + metadata in one pass | 54.8, 34.6, 8.9 | 34.6 s |
| 60 min Opus → MP3 (`-q:a 0`) | 30.2, 36.8, 28.3 | 30.2 s |
| 60 min Opus → .opus, copied | 2.3, 1.2, 1.3 | 1.3 s |

ffprobe found the separate and combined outputs identical in streams, duration and title.

What the processing results say:

- **Each whole-file pass over a large video costs seconds to about a minute here**,
  dominated by writing ~4 GiB (the spread is disk write-back, not ffmpeg). Skipping a
  pass, as Fast Save does for metadata and subtitles, saves one such pass.
- **Combining merge and metadata into one pass is inconclusive on this data.** The medians
  (53.5 s separate vs 34.6 s combined) favour it, but the run-to-run spread is larger than
  the difference. It needs a quieter measurement (smaller files, or a disk-flush between
  runs) before Task 7 can be justified on it.
- **Audio conversion is real work; copying is nearly free.** An hour of audio took ~30 s
  to encode to MP3 and ~1.3 s to copy. That is the saving *Prefer matching source audio*
  gives when the source already has the chosen format.
- CPU time was not captured in this run (the benchmark asked ffmpeg for errors only, which
  hides its `bench:` line; fixed in the script for the next run).

## Decision (plan Task 6)

| If the real slow source turns out to be… | Then |
| --- | --- |
| plain HTTP (one file, no fragments) and limited per connection | Task 8, optional parallel HTTP transfer: the only lever, worth 4–8× in the reference. Needs aria2c measured through yt-dlp first. |
| HLS/DASH | Already at 8 fragments; look at the source/extractor instead. No code change indicated. |
| limited server-wide or by the connection | An external limit. Report it; adding connections cannot help. |
| fast to transfer but slow to finish | Fast Save and Prefer matching already remove the measurable passes; Task 7 (combined processing) needs a cleaner measurement first. |
| slow before the transfer starts | Compare extraction against standalone yt-dlp with the same arguments. |

**Not measured:** the real source; aria2c through yt-dlp; Internet latency, loss and
congestion; authenticated sources; cold-disk processing; the native Claws app end to end
(these are yt-dlp and ffmpeg driven directly with Claws' arguments).

To finish the diagnosis, the source is needed: the link (or site), the quality chosen
and the speed seen. Then: `yt-dlp -F <link>` shows its protocols (plain HTTP vs HLS/DASH),
and timing a standalone yt-dlp download with Claws' arguments, against the same file in
Claws, shows whether Claws adds anything.

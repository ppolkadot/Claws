#!/usr/bin/env node
// What processing costs after a large download, measured with ffmpeg on synthetic media.
//
//   node scripts/dev/processing-bench.mjs --ffmpeg <folder with ffmpeg.exe and ffprobe.exe>
//        [--gib 2] [--audio-minutes 60] [--runs 3] [--storage "..."] [--out results.json]
//
// Generates (untimed) an H.264 video of about --gib GiB, a matching AAC track and a long
// Opus track, then times, with one warm-up and --runs timed runs in alternating order:
//
//   merge            video + audio -> MP4, stream copy (what yt-dlp's merger does)
//   metadata         rewrite of the merged MP4 with tags, stream copy (--embed-metadata)
//   merge+metadata   both in a single ffmpeg pass
//   audio-mp3        Opus -> MP3 at libmp3lame -q:a 0 (Claws' --audio-quality 0 conversion)
//   audio-copy       Opus -> .opus with the stream copied (a matching source, no encoding)
//
// ffprobe compares the separate and combined results (streams, codecs, duration, title).
// CPU is ffmpeg's own -benchmark report (user + system). Timings include reading the
// inputs, which right after a download are usually still in the OS file cache; the
// generated inputs are too, so these are not cold-disk figures.
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, statSync, writeFileSync } from "node:fs";
import { cpus, tmpdir, totalmem, release, platform } from "node:os";
import { join, resolve } from "node:path";

const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, value, i, all) => (value.startsWith("--") ? [...pairs, [value.slice(2), all[i + 1]]] : pairs), []),
);
if (!args.ffmpeg) {
  console.error("Pass --ffmpeg <folder containing ffmpeg.exe and ffprobe.exe>.");
  process.exit(2);
}
const ffmpeg = join(args.ffmpeg, "ffmpeg.exe");
const ffprobe = join(args.ffmpeg, "ffprobe.exe");
const targetBytes = Number(args.gib ?? 2) * (1 << 30);
const audioMinutes = Number(args["audio-minutes"] ?? 60);
const runs = Number(args.runs ?? 3);
const repo = resolve(import.meta.dirname, "..", "..");
const work = mkdtempSync(join(tmpdir(), "claws-processing-"));
const at = (name) => join(work, name);

function ff(argv) {
  const started = performance.now();
  // -benchmark's "bench:" line is printed at info level; -nostats keeps the output small.
  const run = spawnSync(ffmpeg, ["-hide_banner", "-nostdin", "-nostats", "-y", "-benchmark", "-loglevel", "info", ...argv], { encoding: "utf8", maxBuffer: 1 << 26 });
  const seconds = (performance.now() - started) / 1000;
  if (run.status !== 0) throw new Error(`ffmpeg failed: ${run.stderr.slice(-2000)}`);
  const bench = /bench: utime=([\d.]+)s stime=([\d.]+)s/.exec(run.stdout + run.stderr);
  return { seconds, cpuSeconds: bench ? Number(bench[1]) + Number(bench[2]) : null };
}

function probe(path) {
  const out = execFileSync(ffprobe, ["-v", "error", "-show_entries", "format=duration:format_tags=title:stream=codec_type,codec_name", "-of", "json", path], { encoding: "utf8" });
  const value = JSON.parse(out);
  return {
    streams: value.streams.map((s) => `${s.codec_type}:${s.codec_name}`),
    durationSeconds: Number(Number(value.format.duration).toFixed(2)),
    title: value.format.tags?.title ?? null,
  };
}

const size = (path) => statSync(path).size;
const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
};

try {
  console.log("generating inputs...");
  // A 60-second lossless chunk with grain (barely compressible, like real high-bitrate
  // video), repeated with the concat demuxer up to the target size.
  ff(["-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=30,noise=alls=12:allf=t", "-t", "60", "-c:v", "libx264", "-preset", "ultrafast", "-qp", "0", "-pix_fmt", "yuv420p", at("chunk.mp4")]);
  const copies = Math.max(1, Math.ceil(targetBytes / size(at("chunk.mp4"))));
  writeFileSync(at("list.txt"), Array.from({ length: copies }, () => "file 'chunk.mp4'").join("\n"));
  ff(["-f", "concat", "-safe", "0", "-i", at("list.txt"), "-c", "copy", at("video.mp4")]);
  rmSync(at("chunk.mp4"));
  const videoSeconds = probe(at("video.mp4")).durationSeconds;
  ff(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", String(videoSeconds), "-c:a", "aac", "-b:a", "160k", at("audio.m4a")]);
  ff(["-f", "lavfi", "-i", `anoisesrc=d=${audioMinutes * 60}:c=pink:r=48000:a=0.05`, "-ac", "2", "-c:a", "libopus", "-b:a", "160k", at("long.webm")]);
  const inputs = {
    videoBytes: size(at("video.mp4")),
    videoSeconds,
    audioBytes: size(at("audio.m4a")),
    longAudioBytes: size(at("long.webm")),
    longAudioSeconds: audioMinutes * 60,
  };
  console.log(JSON.stringify(inputs));

  const tags = ["-metadata", "title=Benchmark title", "-metadata", "artist=Claws", "-metadata", "comment=https://example.invalid/watch"];
  const steps = {
    merge: () => ff(["-i", at("video.mp4"), "-i", at("audio.m4a"), "-map", "0:v:0", "-map", "1:a:0", "-c", "copy", at("merged.mp4")]),
    metadata: () => ff(["-i", at("merged.mp4"), "-map", "0", "-dn", "-ignore_unknown", "-c", "copy", ...tags, at("tagged.mp4")]),
    "merge+metadata": () => ff(["-i", at("video.mp4"), "-i", at("audio.m4a"), "-map", "0:v:0", "-map", "1:a:0", "-dn", "-ignore_unknown", "-c", "copy", ...tags, at("combined.mp4")]),
    "audio-mp3": () => ff(["-i", at("long.webm"), "-vn", "-c:a", "libmp3lame", "-q:a", "0", at("long.mp3")]),
    "audio-copy": () => ff(["-i", at("long.webm"), "-vn", "-c:a", "copy", at("long.opus")]),
  };
  // merge must run before metadata (it produces its input), so they form one sequence.
  const configs = [["merge", "metadata"], ["merge+metadata"], ["audio-mp3"], ["audio-copy"]];
  const timings = Object.fromEntries(Object.keys(steps).map((k) => [k, []]));
  for (let round = 0; round <= runs; round++) {
    const order = round % 2 === 0 ? configs : [...configs].reverse();
    for (const sequence of order) {
      for (const name of sequence) {
        const t = steps[name]();
        if (round > 0) timings[name].push(t);
      }
    }
    console.log(`round ${round} done`);
  }

  const separate = probe(at("tagged.mp4"));
  const combined = probe(at("combined.mp4"));
  const parity = {
    separate,
    combined,
    sameStreams: JSON.stringify(separate.streams) === JSON.stringify(combined.streams),
    sameDuration: separate.durationSeconds === combined.durationSeconds,
    sameTitle: separate.title === combined.title,
    mp3: probe(at("long.mp3")),
    opusCopy: probe(at("long.opus")),
  };
  const summary = Object.fromEntries(
    Object.entries(timings).map(([name, list]) => [
      name,
      {
        medianSeconds: median(list.map((t) => t.seconds)),
        spreadSeconds: Math.max(...list.map((t) => t.seconds)) - Math.min(...list.map((t) => t.seconds)),
        medianCpuSeconds: median(list.map((t) => t.cpuSeconds ?? 0)),
        runs: list,
      },
    ]),
  );
  const result = {
    complete: false,
    note: "Synthetic media through ffmpeg directly, with the stream-copy arguments yt-dlp's merger and metadata steps use; not a full Claws run.",
    sourceCommit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: repo, encoding: "utf8" }).trim(),
    toolVersions: { ffmpeg: execFileSync(ffmpeg, ["-version"], { encoding: "utf8" }).split("\n")[0].trim() },
    environment: { os: `${platform()} ${release()}`, cpu: cpus()[0]?.model ?? "unknown", logicalCpus: cpus().length, ramBytes: totalmem(), storage: args.storage ?? "unrecorded" },
    inputs,
    outputs: { mergedBytes: size(at("merged.mp4")), taggedBytes: size(at("tagged.mp4")), combinedBytes: size(at("combined.mp4")), mp3Bytes: size(at("long.mp3")), opusCopyBytes: size(at("long.opus")) },
    parity,
    summary,
    derived: {
      separateMedianSeconds: summary.merge.medianSeconds + summary.metadata.medianSeconds,
      combinedMedianSeconds: summary["merge+metadata"].medianSeconds,
      conversionVsCopyMedianSeconds: [summary["audio-mp3"].medianSeconds, summary["audio-copy"].medianSeconds],
    },
  };
  const text = JSON.stringify(result, null, 2);
  if (args.out) writeFileSync(args.out, text);
  console.log(JSON.stringify(result.derived), JSON.stringify({ sameStreams: parity.sameStreams, sameDuration: parity.sameDuration, sameTitle: parity.sameTitle }));
} finally {
  rmSync(work, { recursive: true, force: true });
}

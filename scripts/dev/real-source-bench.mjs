#!/usr/bin/env node
// Times one real link with Claws' download arguments, three ways:
//   baseline  the arguments Claws passes today
//   dashy     the same plus --extractor-args youtube:formats=dashy (a no-op off YouTube)
//   aria2c    the same plus --downloader http,https:aria2c (needs --aria2c)
//
//   node scripts/dev/real-source-bench.mjs --ytdlp <yt-dlp.exe> --url <link> --label <name>
//        [--res 1080] [--runs 3] [--variants baseline,dashy,aria2c] [--aria2c <aria2c.exe>]
//        [--out results.json]
//
// Every run extracts the link again (formats=dashy changes extraction, so a shared
// info.json would not be fair), downloads into a fresh temporary folder, inspects the
// result with ffprobe and deletes it. The variant order rotates between rounds.
//
// Limits: this is an Internet measurement on one PC at one time of day, so the spread
// matters as much as the median. It drives yt-dlp directly, not the Claws app, and it
// skips the optional metadata, cover-art and subtitle passes.
import { spawn, execFileSync, execSync } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir, cpus, release } from "node:os";
import { dirname, join, resolve, delimiter } from "node:path";
import { createInterface } from "node:readline";

const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, value, i, all) => (value.startsWith("--") ? [...pairs, [value.slice(2), all[i + 1]]] : pairs), []),
);
if (!args.ytdlp || !args.url || !args.label) {
  console.error("Pass --ytdlp <yt-dlp.exe> --url <link> --label <name>.");
  process.exit(2);
}
const runs = Number(args.runs ?? 3);
const res = String(args.res ?? 1080);
const variants = (args.variants ?? "baseline,dashy,aria2c").split(",");
if (variants.includes("aria2c") && !args.aria2c) {
  console.error("Pass --aria2c <aria2c.exe>, or leave aria2c out of --variants.");
  process.exit(2);
}
const ffmpegDir = dirname(execSync("where ffmpeg").toString().split(/\r?\n/)[0]);
const EXTRA = {
  baseline: [],
  dashy: ["--extractor-args", "youtube:formats=dashy"],
  aria2c: ["--downloader", "http,https:aria2c"],
};

function ytdlpArgs(variant, outDir) {
  return [
    "--no-colors", "--ignore-config", "--encoding", "utf-8", "--ffmpeg-location", ffmpegDir, "--js-runtimes", "node",
    "--newline", "--no-warnings", "--progress", "--no-simulate", "--windows-filenames",
    "--retries", "10", "--fragment-retries", "10", "--concurrent-fragments", "8", "--no-playlist",
    "--print", "before_dl:MARK|before_dl",
    "--print", "post_process:MARK|post_process",
    "--print", "after_move:FMT|%(format_id)s|%(protocol)s",
    "-f", "bv*+ba/b", "-S", `res:${res},vcodec:h264,acodec:aac`, "--merge-output-format", "mp4",
    ...EXTRA[variant],
    "-P", outDir, "-o", "v.%(ext)s", "--", args.url,
  ];
}

function probe(file) {
  const raw = execFileSync(join(ffmpegDir, "ffprobe"), ["-v", "error", "-show_streams", "-show_format", "-of", "json", file], { encoding: "utf8" });
  const data = JSON.parse(raw);
  return {
    container: data.format.format_name,
    durationSeconds: Number(Number(data.format.duration).toFixed(2)),
    bytes: statSync(file).size,
    streams: data.streams.map((s) => ({
      type: s.codec_type,
      codec: s.codec_name,
      profile: s.profile ?? null,
      width: s.width ?? null,
      height: s.height ?? null,
      frameRate: s.avg_frame_rate ?? null,
      sampleRate: s.sample_rate ?? null,
      channels: s.channels ?? null,
      durationSeconds: s.duration ? Number(Number(s.duration).toFixed(2)) : null,
    })),
  };
}

function runOnce(variant) {
  const outDir = mkdtempSync(join(tmpdir(), "claws-real-bench-"));
  const env = { ...process.env, PYTHONIOENCODING: "utf-8" };
  if (args.aria2c) env.PATH = `${dirname(resolve(args.aria2c))}${delimiter}${env.PATH}`;
  const started = performance.now();
  const at = () => Number(((performance.now() - started) / 1000).toFixed(2));
  const marks = {};
  let formats = null;
  const tail = [];
  return new Promise((done) => {
    const child = spawn(args.ytdlp, ytdlpArgs(variant, outDir), { env });
    const onLine = (line) => {
      const [tag, a, b] = line.split("|");
      if (tag === "MARK") marks[a] ??= at();
      else if (tag === "FMT") formats = { ids: a, protocols: b };
      else if (/^ERROR/.test(line)) tail.push(line.slice(0, 300));
    };
    createInterface({ input: child.stdout }).on("line", onLine);
    createInterface({ input: child.stderr }).on("line", onLine);
    child.on("close", (code) => {
      const totalSeconds = at();
      let output = null;
      let error = code === 0 ? null : tail.join(" / ") || `exit ${code}`;
      try {
        const file = readdirSync(outDir).find((name) => name.startsWith("v."));
        if (code === 0 && file) output = probe(join(outDir, file));
      } catch (e) {
        error = String(e.message).slice(0, 300);
      }
      rmSync(outDir, { recursive: true, force: true });
      const extract = marks.before_dl ?? null;
      const transfer = extract != null && marks.post_process != null ? Number((marks.post_process - extract).toFixed(2)) : null;
      done({
        variant,
        exitCode: code,
        error,
        totalSeconds,
        extractSeconds: extract,
        transferSeconds: transfer,
        finishSeconds: transfer != null ? Number((totalSeconds - marks.post_process).toFixed(2)) : null,
        transferMiBps: transfer && output ? Number((output.bytes / 1048576 / transfer).toFixed(2)) : null,
        formats,
        output,
      });
    });
  });
}

const median = (values) => {
  const sorted = values.filter((v) => v != null).sort((a, b) => a - b);
  return sorted.length ? sorted[Math.floor(sorted.length / 2)] : null;
};
const sameOutput = (a, b) =>
  !!a && !!b && Math.abs(a.durationSeconds - b.durationSeconds) <= 0.1 &&
  JSON.stringify(a.streams.map(({ durationSeconds, ...rest }) => rest)) === JSON.stringify(b.streams.map(({ durationSeconds, ...rest }) => rest));

const results = [];
for (let round = 0; round < runs; round++) {
  const order = variants.map((_, i) => variants[(i + round) % variants.length]);
  for (const variant of order) {
    const result = await runOnce(variant);
    results.push({ round: round + 1, ...result });
    console.log(`round ${round + 1} ${variant.padEnd(8)} total ${result.totalSeconds}s transfer ${result.transferSeconds}s ${result.transferMiBps} MiB/s ${result.formats?.ids ?? ""} ${result.formats?.protocols ?? ""} ${result.error ?? ""}`);
  }
}

const baseline = results.find((r) => r.variant === "baseline" && r.output)?.output ?? null;
const summary = variants.map((variant) => {
  const mine = results.filter((r) => r.variant === variant);
  const ok = mine.filter((r) => r.exitCode === 0);
  const totals = ok.map((r) => r.totalSeconds);
  return {
    variant,
    completed: `${ok.length}/${mine.length}`,
    medianTotalSeconds: median(totals),
    spreadTotalSeconds: totals.length ? Number((Math.max(...totals) - Math.min(...totals)).toFixed(2)) : null,
    medianTransferSeconds: median(ok.map((r) => r.transferSeconds)),
    medianTransferMiBps: median(ok.map((r) => r.transferMiBps)),
    formats: [...new Set(ok.map((r) => `${r.formats?.ids} (${r.formats?.protocols})`))],
    outputMatchesBaseline: ok.length > 0 && ok.every((r) => sameOutput(r.output, baseline)),
  };
});
const base = summary.find((s) => s.variant === "baseline");
for (const s of summary) {
  s.speedupVsBaseline = base?.medianTotalSeconds && s.medianTotalSeconds ? Number((base.medianTotalSeconds / s.medianTotalSeconds).toFixed(2)) : null;
  s.transferSpeedupVsBaseline = base?.medianTransferSeconds && s.medianTransferSeconds ? Number((base.medianTransferSeconds / s.medianTransferSeconds).toFixed(2)) : null;
}

const version = (file, flag = "--version") => execFileSync(file, [flag], { encoding: "utf8" }).split("\n")[0].trim();
const report = {
  label: args.label,
  url: args.url,
  date: new Date().toISOString(),
  resolutionCap: res,
  runsPerVariant: runs,
  environment: {
    os: `Windows ${release()}`,
    cpu: cpus()[0].model.trim(),
    ytdlp: version(args.ytdlp),
    aria2c: args.aria2c ? version(args.aria2c) : null,
    ffmpeg: version(join(ffmpegDir, "ffmpeg"), "-version"),
  },
  note: "Internet measurement through yt-dlp directly, not the Claws app. Each run extracts again and deletes its output.",
  summary,
  runs: results,
};
console.table(summary.map(({ formats, ...rest }) => rest));
if (args.out) {
  writeFileSync(args.out, JSON.stringify(report, null, 2) + "\n");
  console.log(`wrote ${args.out}`);
}

#!/usr/bin/env node
// Reproducible download benchmarks against the local fixtures.
//
//   node scripts/dev/download-bench.mjs --ytdlp <yt-dlp.exe> [--size-mib 64] [--runs 3]
//        [--scenario range|no-range|slow|hls-ok] [--fragments 1,4,8] [--out results.json]
//
// Starts scripts/dev/download-fixtures.py on a free localhost port, runs one warm-up and
// then `--runs` timed runs per configuration, and writes one JSON result per configuration
// in the format of the plan's section 8. Every run is checked against the fixture's
// SHA-256; a failed integrity check rejects the whole configuration instead of being
// reported. No cookies, signed URLs or authorization headers are involved or recorded.
//
// Measurements and their limits:
// - extract/transfer/process/publish seconds come from marker lines yt-dlp prints;
// - cpuSeconds samples the yt-dlp process every 250 ms (children such as ffmpeg excluded);
// - peakDiskBytes samples the work directory every 250 ms;
// - longTasks is a UI measurement and is not available here, so these results are
//   labelled "complete": false (exploratory) and cannot gate a default change on their own.
import { spawn, execFile, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream, mkdtempSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { cpus, tmpdir, totalmem, release, platform } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";

const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, value, i, all) => (value.startsWith("--") ? [...pairs, [value.slice(2), all[i + 1]]] : pairs), []),
);
if (!args.ytdlp) {
  console.error("Pass --ytdlp <path to yt-dlp.exe>.");
  process.exit(2);
}
const runs = Number(args.runs ?? 3);
const sizeMiB = Number(args["size-mib"] ?? 64);
const scenarios = (args.scenario ?? "range").split(",");
const fragmentChoices = (args.fragments ?? "1,4,8").split(",").map(Number);
const repo = resolve(import.meta.dirname, "..", "..");

function git(...a) {
  try {
    return execFileSync("git", a, { cwd: repo, encoding: "utf8" }).trim();
  } catch {
    return "unknown";
  }
}

function startFixtures() {
  return new Promise((resolvePromise, reject) => {
    const child = spawn("python", [join(repo, "scripts/dev/download-fixtures.py"), "--size-mib", String(sizeMiB), "--segment-kib", String(Math.max(64, Math.round((sizeMiB * 1024) / 3)))], {
      stdio: ["ignore", "pipe", "inherit"],
    });
    createInterface({ input: child.stdout }).once("line", (line) => resolvePromise({ child, ...JSON.parse(line) }));
    child.once("error", reject);
  });
}

function dirBytes(dir) {
  let total = 0;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    total += entry.isDirectory() ? dirBytes(path) : statSync(path).size;
  }
  return total;
}

function sha256(path) {
  return new Promise((resolvePromise, reject) => {
    const hash = createHash("sha256");
    createReadStream(path).on("data", (d) => hash.update(d)).on("end", () => resolvePromise(hash.digest("hex"))).on("error", reject);
  });
}

// Asynchronous, so sampling never stalls the timers that measure the run.
function cpuOf(pid) {
  return new Promise((resolvePromise) => {
    execFile(
      "powershell",
      ["-NoProfile", "-Command", `(Get-Process -Id ${pid} -ErrorAction SilentlyContinue).TotalProcessorTime.TotalSeconds`],
      { encoding: "utf8", windowsHide: true },
      (error, out) => resolvePromise(error ? null : Number(out.trim()) || null),
    );
  });
}

async function runOnce(baseUrl, scenario, fragments) {
  const work = mkdtempSync(join(tmpdir(), "claws-bench-"));
  const url = scenario === "hls-ok" ? `${baseUrl}/hls-ok/list.m3u8` : `${baseUrl}/${scenario}/file.bin`;
  const ytArgs = [
    "--ignore-config", "--no-colors", "--no-warnings", "--newline", "--fixup", "never",
    "--concurrent-fragments", String(fragments), "--retries", "10", "--fragment-retries", "10",
    "--abort-on-unavailable-fragments", "--progress-delta", "0.2",
    "--print", "before_dl:BENCH|before_dl", "--print", "after_move:BENCH|done|%(filepath)s", "--no-simulate",
    "-P", work, "-o", "payload.%(ext)s", "--", url,
  ];
  const started = performance.now();
  const marks = {};
  const child = spawn(args.ytdlp, ytArgs, { stdio: ["ignore", "pipe", "pipe"] });
  let peak = 0;
  let cpu = 0;
  let sampling = false;
  const sampler = setInterval(() => {
    try {
      peak = Math.max(peak, dirBytes(work));
    } catch {}
    if (sampling) return;
    sampling = true;
    void cpuOf(child.pid).then((sample) => {
      if (sample !== null) cpu = Math.max(cpu, sample);
      sampling = false;
    });
  }, 250);
  let finalPath = null;
  createInterface({ input: child.stdout }).on("line", (line) => {
    if (line === "BENCH|before_dl") marks.beforeDl = performance.now();
    if (line.startsWith("BENCH|done|")) {
      marks.done = performance.now();
      finalPath = line.slice("BENCH|done|".length);
    }
  });
  let stderr = "";
  child.stderr.on("data", (d) => (stderr = (stderr + d).slice(-8000)));
  const code = await new Promise((r) => child.once("exit", r));
  clearInterval(sampler);
  const ended = performance.now();
  try {
    peak = Math.max(peak, dirBytes(work));
  } catch {}
  const result = { code, stderr, finalPath, work };
  if (code !== 0 || !finalPath || !marks.beforeDl || !marks.done) return result;
  result.run = {
    extractSeconds: (marks.beforeDl - started) / 1000,
    transferSeconds: (marks.done - marks.beforeDl) / 1000,
    // --fixup never and no post-processors: nothing runs between transfer and the move.
    processSeconds: 0,
    publishSeconds: (ended - marks.done) / 1000,
    receivedBytes: statSync(finalPath).size,
    peakDiskBytes: peak,
    cpuSeconds: cpu,
  };
  result.digest = await sha256(finalPath);
  return result;
}

function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

const fixtures = await startFixtures();
const results = [];
try {
  const environment = {
    os: `${platform()} ${release()}`,
    cpu: cpus()[0]?.model ?? "unknown",
    ramBytes: totalmem(),
    storage: args.storage ?? "unrecorded (pass --storage to describe the disk)",
  };
  const ytVersion = execFileSync(args.ytdlp, ["--version"], { encoding: "utf8" }).trim();
  for (const scenario of scenarios) {
    const expected = (await (await fetch(`${fixtures.baseUrl}/sha256/${scenario}`)).text()).trim();
    for (const fragments of fragmentChoices) {
      const label = `${scenario} fragments=${fragments}`;
      const collected = [];
      let integrityPassed = true;
      for (let i = 0; i <= runs; i++) {
        const outcome = await runOnce(fixtures.baseUrl, scenario, fragments);
        rmSync(outcome.work, { recursive: true, force: true });
        if (!outcome.run || outcome.digest !== expected) {
          integrityPassed = false;
          console.error(`${label}: run ${i} failed (exit ${outcome.code}) ${outcome.stderr.trim().split("\n").pop() ?? ""}`);
          break;
        }
        if (i > 0) collected.push({ ...outcome.run, longTasks: null }); // run 0 is the warm-up
      }
      if (!integrityPassed) {
        console.error(`${label}: rejected, integrity or run failure.`);
        continue;
      }
      const transfer = collected.map((r) => r.transferSeconds);
      const result = {
        scenario: label,
        complete: false,
        unmeasured: ["longTasks"],
        sourceCommit: git("rev-parse", "HEAD"),
        toolVersions: { ytDlp: ytVersion, ffmpeg: "not used (--fixup never)" },
        environment,
        settings: { jobs: 1, fragments, transport: "native", metadata: false, subtitles: false },
        payload: { advertisedBytes: Math.round(sizeMiB * 1024 * 1024), sha256: expected },
        integrityPassed,
        runs: collected,
        summary: { transferMedianSeconds: median(transfer), transferSpreadSeconds: Math.max(...transfer) - Math.min(...transfer) },
      };
      results.push(result);
      console.log(`${label}: transfer median ${result.summary.transferMedianSeconds.toFixed(2)} s (spread ${result.summary.transferSpreadSeconds.toFixed(2)} s)`);
    }
  }
} finally {
  fixtures.child.kill();
}
if (args.out) writeFileSync(args.out, JSON.stringify(results, null, 2));
else console.log(JSON.stringify(results, null, 2));

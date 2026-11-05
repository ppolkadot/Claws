#!/usr/bin/env node
// Reproducible download benchmarks against the local fixtures.
//
//   node scripts/dev/download-bench.mjs --ytdlp <yt-dlp.exe> [--size-mib 64] [--runs 3]
//        [--scenario range|no-range|slow|hls-ok|capped|capped-hls] [--fragments 1,4,8]
//        [--jobs 1] [--downloader native|aria2c|ranges:N] [--aria2c <aria2c.exe>]
//        [--segments 32] [--conn-rate-kib 8192] [--global-rate-kib 0] [--out results.json]
//
// Starts scripts/dev/download-fixtures.py on a free localhost port, runs one warm-up and
// then `--runs` timed runs per configuration, and writes one JSON result per configuration.
// Every file is checked against the fixture's SHA-256; a failed check rejects the whole
// configuration.
//
// Configurations are the product of scenario x fragments x downloader x jobs. `--jobs N`
// starts N downloads of the same fixture at once and times the round from the first start
// to the last finish. `--downloader aria2c` routes plain HTTP(S) through aria2c with
// yt-dlp's own aria2c defaults; HLS stays native.
//
// Measurements and their limits:
// - extract/transfer/publish seconds come from marker lines yt-dlp prints; with jobs > 1
//   they are per job, and roundSeconds is the aggregate;
// - cpuSeconds sums every process in each run's tree (yt-dlp, aria2c), sampled every 0.5 s
//   by one PowerShell process, so a child that lives less than that can be missed;
// - peakDiskBytes samples the work directories every 250 ms;
// - longTasks is a UI measurement and is not available here, so these results are
//   labelled "complete": false (exploratory) and cannot gate a default change on their own;
// - localhost has no real network latency, loss or congestion: a fixture cap stands in
//   for a server's limit, and results describe that limit, not an Internet source.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream, mkdtempSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { open } from "node:fs/promises";
import { cpus, tmpdir, totalmem, release, platform } from "node:os";
import { dirname, join, resolve, delimiter } from "node:path";
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
const segments = Number(args.segments ?? 3);
const connRateKiB = Number(args["conn-rate-kib"] ?? 8192);
const globalRateKiB = Number(args["global-rate-kib"] ?? 0);
const scenarios = (args.scenario ?? "range").split(",");
const fragmentChoices = (args.fragments ?? "1,4,8").split(",").map(Number);
const jobChoices = (args.jobs ?? "1").split(",").map(Number);
const downloaders = (args.downloader ?? "native").split(",");
if (downloaders.includes("aria2c") && !args.aria2c) {
  console.error("Pass --aria2c <path to aria2c.exe> to compare the aria2c transport.");
  process.exit(2);
}
const repo = resolve(import.meta.dirname, "..", "..");
const isHls = (scenario) => scenario.endsWith("hls") || scenario === "hls-ok";

function git(...a) {
  try {
    return execFileSync("git", a, { cwd: repo, encoding: "utf8" }).trim();
  } catch {
    return "unknown";
  }
}

function startFixtures() {
  const segmentKiB = Math.max(64, Math.round((sizeMiB * 1024) / segments));
  return new Promise((resolvePromise, reject) => {
    const child = spawn(
      "python",
      [
        join(repo, "scripts/dev/download-fixtures.py"),
        "--size-mib", String(sizeMiB),
        "--segments", String(segments),
        "--segment-kib", String(segmentKiB),
        "--conn-rate-kib", String(connRateKiB),
        "--global-rate-kib", String(globalRateKiB),
      ],
      { stdio: ["ignore", "pipe", "inherit"] },
    );
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

// One PowerShell process per round samples each root's process-tree CPU seconds every
// 0.5 s; the highest value seen is that run's CPU time.
function treeCpuSampler(roots) {
  const script = `
$roots = @(${roots.join(",")})
while ($true) {
  $all = Get-CimInstance Win32_Process -Property ProcessId,ParentProcessId,UserModeTime,KernelModeTime
  $children = @{}
  foreach ($p in $all) { $children[[int]$p.ParentProcessId] += ,$p }
  $byId = @{}
  foreach ($p in $all) { $byId[[int]$p.ProcessId] = $p }
  $line = @()
  foreach ($root in $roots) {
    $total = 0.0; $queue = @($root); $seen = @{}
    while ($queue.Count -gt 0) {
      $id = $queue[0]; $queue = $queue[1..($queue.Count)]
      if ($seen[$id]) { continue }; $seen[$id] = $true
      $p = $byId[$id]
      if ($p) { $total += ($p.UserModeTime + $p.KernelModeTime) / 1e7 }
      foreach ($c in $children[$id]) { $queue += [int]$c.ProcessId }
    }
    $line += "$root=$total"
  }
  Write-Output ($line -join " ")
  Start-Sleep -Milliseconds 500
}`;
  const child = spawn("powershell", ["-NoProfile", "-Command", script], { stdio: ["ignore", "pipe", "ignore"], windowsHide: true });
  const peak = new Map();
  createInterface({ input: child.stdout }).on("line", (line) => {
    for (const pair of line.trim().split(" ")) {
      const [pid, value] = pair.split("=");
      if (pid && value) peak.set(Number(pid), Math.max(peak.get(Number(pid)) ?? 0, Number(value) || 0));
    }
  });
  return { stop: () => child.kill(), cpuOf: (pid) => peak.get(pid) ?? null };
}

// A reference strategy, not a Claws code path: the file fetched as N byte ranges at once
// and written in place, to show what a range-parallel transport could gain.
function startRanges(baseUrl, scenario, parts) {
  const work = mkdtempSync(join(tmpdir(), "claws-bench-"));
  const started = performance.now();
  const marks = {};
  let finalPath = null;
  let error = "";
  const exited = (async () => {
    const url = `${baseUrl}/${scenario}/file.bin`;
    const total = Number((await fetch(url, { method: "HEAD" })).headers.get("content-length"));
    marks.beforeDl = performance.now();
    const target = join(work, "payload.bin");
    const handle = await open(target, "w");
    try {
      const step = Math.ceil(total / parts);
      await Promise.all(
        Array.from({ length: parts }, async (_, i) => {
          const from = i * step;
          const to = Math.min(total, from + step) - 1;
          if (from > to) return;
          const response = await fetch(url, { headers: { Range: `bytes=${from}-${to}` } });
          if (response.status !== 206) throw new Error(`range ${from}-${to}: HTTP ${response.status}`);
          let position = from;
          for await (const chunk of response.body) {
            await handle.write(chunk, 0, chunk.length, position);
            position += chunk.length;
          }
        }),
      );
    } finally {
      await handle.close();
    }
    marks.done = performance.now();
    finalPath = target;
    return { code: 0, ended: performance.now() };
  })().catch((e) => {
    error = String(e);
    return { code: 1, ended: performance.now() };
  });
  return { child: { pid: process.pid }, work, started, marks, exited, finalPath: () => finalPath, stderr: () => error };
}

function startRun(baseUrl, scenario, fragments, downloader) {
  if (downloader.startsWith("ranges:")) return startRanges(baseUrl, scenario, Number(downloader.slice("ranges:".length)));
  const work = mkdtempSync(join(tmpdir(), "claws-bench-"));
  const url = isHls(scenario) ? `${baseUrl}/${scenario}/list.m3u8` : `${baseUrl}/${scenario}/file.bin`;
  const transport = downloader === "aria2c" && !isHls(scenario) ? ["--downloader", "http,https:aria2c"] : [];
  const ytArgs = [
    "--ignore-config", "--no-colors", "--no-warnings", "--newline", "--fixup", "never",
    "--concurrent-fragments", String(fragments), "--retries", "10", "--fragment-retries", "10",
    "--retry-sleep", "http:exp=1:30", "--retry-sleep", "fragment:exp=1:30", "--socket-timeout", "30",
    "--abort-on-unavailable-fragments", "--progress-delta", "0.2", ...transport,
    "--print", "before_dl:BENCH|before_dl", "--print", "after_move:BENCH|done|%(filepath)s", "--no-simulate",
    "-P", work, "-o", "payload.%(ext)s", "--", url,
  ];
  // aria2c is found on PATH, as Claws would find a managed copy.
  const env = { ...process.env };
  if (args.aria2c) env.PATH = `${dirname(resolve(args.aria2c))}${delimiter}${env.PATH}`;
  const started = performance.now();
  const marks = {};
  const child = spawn(args.ytdlp, ytArgs, { stdio: ["ignore", "pipe", "pipe"], env });
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
  const exited = new Promise((r) => child.once("exit", r)).then((code) => ({ code, ended: performance.now() }));
  return { child, work, started, marks, exited, finalPath: () => finalPath, stderr: () => stderr };
}

async function runRound(baseUrl, scenario, fragments, downloader, jobs) {
  const roundStart = performance.now();
  const runsInRound = Array.from({ length: jobs }, () => startRun(baseUrl, scenario, fragments, downloader));
  // The reference strategy runs inside this process, whose CPU is not sampled.
  const cpu = treeCpuSampler(runsInRound.map((r) => r.child.pid).filter((pid) => pid !== process.pid));
  let peak = 0;
  const sampler = setInterval(() => {
    try {
      peak = Math.max(peak, runsInRound.reduce((sum, r) => sum + dirBytes(r.work), 0));
    } catch {}
  }, 250);
  const exits = await Promise.all(runsInRound.map((r) => r.exited));
  const roundSeconds = (performance.now() - roundStart) / 1000;
  clearInterval(sampler);
  cpu.stop();
  const results = [];
  for (const [i, r] of runsInRound.entries()) {
    const { code, ended } = exits[i];
    const finalPath = r.finalPath();
    if (code !== 0 || !finalPath || !r.marks.beforeDl || !r.marks.done) {
      results.push({ failed: `exit ${code}: ${r.stderr().trim().split("\n").pop() ?? ""}`, work: r.work });
      continue;
    }
    results.push({
      work: r.work,
      digest: await sha256(finalPath),
      run: {
        extractSeconds: (r.marks.beforeDl - r.started) / 1000,
        transferSeconds: (r.marks.done - r.marks.beforeDl) / 1000,
        // --fixup never and no post-processors: nothing runs between transfer and the move.
        processSeconds: 0,
        publishSeconds: (ended - r.marks.done) / 1000,
        receivedBytes: statSync(finalPath).size,
        cpuSeconds: cpu.cpuOf(r.child.pid),
      },
    });
  }
  return { roundSeconds, peakDiskBytes: peak, results };
}

function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

const fixtures = await startFixtures();
const output = [];
try {
  const environment = {
    os: `${platform()} ${release()}`,
    cpu: cpus()[0]?.model ?? "unknown",
    logicalCpus: cpus().length,
    ramBytes: totalmem(),
    storage: args.storage ?? "unrecorded (pass --storage to describe the disk)",
    network: "localhost fixtures; caps stand in for server limits",
  };
  const ytVersion = execFileSync(args.ytdlp, ["--version"], { encoding: "utf8" }).trim();
  const aria2Version = args.aria2c ? execFileSync(args.aria2c, ["--version"], { encoding: "utf8" }).split("\n")[0].trim() : null;
  const configs = [];
  for (const scenario of scenarios)
    for (const downloader of downloaders)
      for (const fragments of fragmentChoices)
        for (const jobs of jobChoices) {
          // aria2c and ranges only change plain HTTP; fragments only matter to HLS/DASH.
          if (downloader !== "native" && isHls(scenario)) continue;
          if (downloader.startsWith("ranges:") && fragments !== fragmentChoices[0]) continue;
          configs.push({ scenario, downloader, fragments, jobs, label: `${scenario} downloader=${downloader} fragments=${fragments} jobs=${jobs}`, rounds: [], failed: null });
        }
  const expected = {};
  for (const scenario of scenarios) expected[scenario] = (await (await fetch(`${fixtures.baseUrl}/sha256/${scenario}`)).text()).trim();

  // Round 0 is the warm-up. Later rounds alternate the order so slow drift (thermal,
  // caches) does not always favour the same configuration.
  for (let round = 0; round <= runs; round++) {
    const order = round % 2 === 0 ? configs : [...configs].reverse();
    for (const config of order) {
      if (config.failed) continue;
      const outcome = await runRound(fixtures.baseUrl, config.scenario, config.fragments, config.downloader, config.jobs);
      for (const r of outcome.results) rmSync(r.work, { recursive: true, force: true });
      const bad = outcome.results.find((r) => r.failed || r.digest !== expected[config.scenario]);
      if (bad) {
        config.failed = bad.failed ?? "integrity check failed";
        console.error(`${config.label}: round ${round} rejected: ${config.failed}`);
        continue;
      }
      if (round > 0) config.rounds.push(outcome);
    }
  }

  for (const config of configs) {
    if (config.failed) {
      output.push({ scenario: config.label, integrityPassed: false, rejected: config.failed });
      continue;
    }
    const roundSeconds = config.rounds.map((r) => r.roundSeconds);
    const totalBytes = config.rounds[0].results.reduce((sum, r) => sum + r.run.receivedBytes, 0);
    const result = {
      scenario: config.label,
      complete: false,
      unmeasured: ["longTasks"],
      sourceCommit: git("rev-parse", "HEAD"),
      toolVersions: { ytDlp: ytVersion, aria2c: config.downloader === "aria2c" ? aria2Version : null, ffmpeg: "not used (--fixup never)" },
      environment,
      settings: { jobs: config.jobs, fragments: config.fragments, transport: config.downloader, metadata: false, subtitles: false },
      fixture: { route: config.scenario, perConnectionRateBytesPerSecond: config.scenario.startsWith("capped") ? connRateKiB * 1024 : null, globalRateBytesPerSecond: globalRateKiB ? globalRateKiB * 1024 : null, segments: isHls(config.scenario) ? segments : null },
      payload: { advertisedBytes: Math.round(sizeMiB * 1024 * 1024), sha256: expected[config.scenario] },
      integrityPassed: true,
      runs: config.rounds.map((r) => ({ roundSeconds: r.roundSeconds, peakDiskBytes: r.peakDiskBytes, jobs: r.results.map((x) => ({ ...x.run, longTasks: null })) })),
      summary: {
        roundMedianSeconds: median(roundSeconds),
        roundSpreadSeconds: Math.max(...roundSeconds) - Math.min(...roundSeconds),
        aggregateMiBPerSecond: totalBytes / (1 << 20) / median(roundSeconds),
        transferMedianSeconds: median(config.rounds.flatMap((r) => r.results.map((x) => x.run.transferSeconds))),
        cpuMedianSeconds: median(config.rounds.flatMap((r) => r.results.map((x) => x.run.cpuSeconds ?? 0))),
      },
    };
    output.push(result);
    const s = result.summary;
    console.log(`${config.label}: round median ${s.roundMedianSeconds.toFixed(2)} s (spread ${s.roundSpreadSeconds.toFixed(2)} s), ${s.aggregateMiBPerSecond.toFixed(1)} MiB/s, cpu ${s.cpuMedianSeconds.toFixed(1)} s/job`);
  }
} finally {
  fixtures.child.kill();
}
if (args.out) writeFileSync(args.out, JSON.stringify(output, null, 2));
else console.log(JSON.stringify(output, null, 2));

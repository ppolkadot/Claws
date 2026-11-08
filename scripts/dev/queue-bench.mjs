#!/usr/bin/env node
// Measures what the Queue view costs the main thread, in the real frontend running in a
// headless Edge/Chrome against the dev mock (npm run dev). No backend, no live sites.
//
//   node scripts/dev/queue-bench.mjs [--protocol legacy|v1] [--progress unknown|known] [--browser <exe>] [--out file.json]
//
// --progress known sends progressKnown: true, so the rows draw the determinate fill (and its
// transition) instead of the indeterminate sheen; the default keeps the recorded baseline's setup.
//
// For each queue size (1, 100, 500 added jobs) and active-transfer count (1, 3, 8):
//   1. adds the jobs (legacy: one job-update event per job; v1: one jobs-added batch),
//   2. sends 40 rounds of progress, one event per active job per round, a frame apart.
// Recorded per phase, from CDP Performance.getMetrics and a longtask observer:
// script, layout and style-recalculation milliseconds, and long tasks (> 50 ms).
// If the page exposes window.__clawsQueueStats (dev builds), structural syncs and row
// paints are recorded too.
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, value, i, all) => (value.startsWith("--") ? [...pairs, [value.slice(2), all[i + 1]]] : pairs), []),
);
const protocol = args.protocol ?? "v1";
const progressKind = args.progress ?? "unknown";
const browser = args.browser ?? "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe";
const repo = resolve(import.meta.dirname, "..", "..");
const port = 1431;
const debugPort = 9333;
const sizes = [1, 100, 500];
const actives = [1, 3, 8];
const rounds = 40;

function waitForOutput(child, pattern) {
  return new Promise((resolvePromise, reject) => {
    let seen = "";
    const onData = (d) => {
      seen += d;
      if (pattern.test(seen)) resolvePromise();
    };
    child.stdout.on("data", onData);
    child.stderr.on("data", onData);
    child.once("exit", (code) => reject(new Error(`exited ${code}: ${seen.slice(-500)}`)));
  });
}

const vite = spawn(process.execPath, [join(repo, "node_modules/vite/bin/vite.js"), "--port", String(port), "--strictPort"], { cwd: repo, stdio: ["ignore", "pipe", "pipe"] });
const profile = mkdtempSync(join(tmpdir(), "claws-queue-bench-"));
const chrome = spawn(browser, ["--headless=new", `--remote-debugging-port=${debugPort}`, `--user-data-dir=${profile}`, "--no-first-run", "--window-size=1160,760", "about:blank"], { stdio: "ignore" });

async function target() {
  for (let i = 0; i < 100; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json();
      const page = list.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error("no browser page");
}

let nextId = 1;
const pending = new Map();
let socket;
function cdp(method, params = {}) {
  const id = nextId++;
  socket.send(JSON.stringify({ id, method, params }));
  return new Promise((resolvePromise, reject) => pending.set(id, { resolvePromise, reject }));
}
async function evaluate(expression) {
  const result = await cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails).slice(0, 500));
  return result.result.value;
}
async function metrics() {
  const { metrics: list } = await cdp("Performance.getMetrics");
  const pick = (name) => list.find((m) => m.name === name)?.value ?? 0;
  return { script: pick("ScriptDuration") * 1000, layout: pick("LayoutDuration") * 1000, style: pick("RecalcStyleDuration") * 1000 };
}
const diff = (a, b) => ({ scriptMs: +(b.script - a.script).toFixed(1), layoutMs: +(b.layout - a.layout).toFixed(1), styleMs: +(b.style - a.style).toFixed(1) });

// Runs inside the page. Emits through the mock's event bus, exactly as the backend would.
const pageHelpers = (protocol) => `
  window.__bench = {
    emit: (event, payload) => window.__TAURI_INTERNALS__.invoke('plugin:event|emit', { event, payload }),
    frame: () => new Promise((r) => requestAnimationFrame(() => r())),
    revision: 1000,
    job(id, i, status, progress) {
      return { progressKnown: ${progressKind === "known"} && status === 'downloading', id, url: 'https://example.com/b/' + id, title: 'Bench job ' + i, thumbnail: null, uploader: 'Bench', duration: 60,
        options: { mode: 'video', maxHeight: null, audioFormat: 'mp3', subtitleLangs: [], autoSubs: false, embedSubs: false, fastSave: false },
        status, progress, speed: 5000000, eta: 30, filePath: null, error: null, createdAt: 1e13 + i,
        downloadedBytes: null, totalBytes: null, progressEstimated: false, revision: ++this.revision };
    },
    async add(n) {
      const rows = () => document.querySelectorAll('.jobs .job').length;
      const expected = rows() + n;
      const jobs = Array.from({ length: n }, (_, i) => this.job('b' + i, i, 'queued', 0));
      if ('${protocol}' === 'legacy') { for (const j of jobs) await this.emit('job-update', j); }
      else await this.emit('jobs-added', { schemaVersion: 1, jobs: jobs.map((j) => ({ schemaVersion: 1, jobId: j.id, revision: j.revision, job: j })) });
      // Until every row exists (rows may be built over several frames), then one more.
      for (let i = 0; i < 300 && rows() < expected; i++) await this.frame();
      await this.frame(); await this.frame();
    },
    async progress(active, rounds) {
      for (let r = 1; r <= rounds; r++) {
        for (let i = 0; i < active; i++) {
          const j = this.job('b' + i, i, 'downloading', r / (rounds + 1));
          if ('${protocol}' === 'legacy') await this.emit('job-update', j);
          else await this.emit('job-update', { schemaVersion: 1, jobId: j.id, revision: j.revision, job: j });
        }
        await this.frame();
      }
      await this.frame();
    },
  };
  window.__lt = [];
  new PerformanceObserver((list) => list.getEntries().forEach((e) => window.__lt.push(e.duration))).observe({ type: 'longtask' });
  true;
`;

const results = [];
try {
  await waitForOutput(vite, /ready in/i);
  socket = new WebSocket(await target());
  await new Promise((r) => socket.addEventListener("open", r, { once: true }));
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    const waiter = pending.get(message.id);
    if (waiter) {
      pending.delete(message.id);
      message.error ? waiter.reject(new Error(message.error.message)) : waiter.resolvePromise(message.result);
    }
  });
  await cdp("Page.enable");
  await cdp("Runtime.enable");
  await cdp("Performance.enable");
  for (const size of sizes) {
    for (const active of actives) {
      if (active > size) continue;
      await cdp("Page.navigate", { url: `http://localhost:${port}/` });
      for (let i = 0; i < 100; i++) {
        if (await evaluate("Boolean(document.querySelector('.nav-badge')) && document.readyState === 'complete'").catch(() => false)) break;
        await new Promise((r) => setTimeout(r, 100));
      }
      await evaluate(pageHelpers(protocol));
      await evaluate("document.dispatchEvent(new KeyboardEvent('keydown', { key: '2', ctrlKey: true, bubbles: true })); new Promise((r) => setTimeout(r, 600))");
      const stats = () => evaluate("JSON.stringify(window.__clawsQueueStats ?? null)").then(JSON.parse);
      const s0 = await stats();
      const m0 = await metrics();
      await evaluate(`window.__bench.add(${size})`);
      const m1 = await metrics();
      const s1 = await stats();
      const takeLongTasks = () => evaluate("(() => { const d = window.__lt.splice(0); return { count: d.length, longestMs: d.length ? Math.round(Math.max(...d)) : 0 }; })()");
      const lt1 = await takeLongTasks();
      await evaluate(`window.__bench.progress(${active}, ${rounds})`);
      const m2 = await metrics();
      const s2 = await stats();
      const lt2 = await takeLongTasks();
      const rows = await evaluate("document.querySelectorAll('.jobs .job').length");
      const delta = (a, b) => (a && b ? Object.fromEntries(Object.keys(b).map((k) => [k, b[k] - (a[k] ?? 0)])) : null);
      const progressEvents = active * rounds;
      const result = {
        protocol,
        progressKind,
        addedJobs: size,
        activeTransfers: active,
        renderedRows: rows,
        add: { ...diff(m0, m1), longTasks: lt1.count, longestTaskMs: lt1.longestMs, counters: delta(s0, s1) },
        progress: { events: progressEvents, ...diff(m1, m2), longTasks: lt2.count, longestTaskMs: lt2.longestMs, counters: delta(s1, s2), scriptMsPerEvent: +((m2.script - m1.script) / progressEvents).toFixed(3) },
      };
      results.push(result);
      console.log(`${protocol}/${progressKind} jobs=${size} active=${active}: add ${result.add.scriptMs} ms script, ${result.add.longTasks} long (longest ${result.add.longestTaskMs} ms); progress ${result.progress.scriptMsPerEvent} ms/event, ${result.progress.longTasks} long; rows ${rows}`);
    }
  }
} finally {
  socket?.close();
  chrome.kill();
  vite.kill();
}
if (args.out) writeFileSync(args.out, JSON.stringify(results, null, 2));
